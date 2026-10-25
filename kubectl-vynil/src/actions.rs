//! Active commands that drive the operator from the client's kubectl context.
//!
//! `scan` / `upgrade` talk to the apiserver directly (annotate + poll); the
//! diagnostic verbs reuse the aggregation transport. The operator deletes and
//! recreates the relevant job on `force-scan` / `force-reinstall`, so we track the
//! previous job UID to wait for the *new* job rather than a stale terminal one.

use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use k8s_openapi::api::{
    batch::v1::Job,
    core::v1::{Namespace, Pod},
};
use kube::{
    Client,
    api::{Api, DynamicObject, ListParams, Patch, PatchParams},
    discovery::ApiResource,
};

use crate::{
    bundle::build_bundle,
    cli::{
        DiagnosticArgs, InstanceArgs, InstanceKindInfo, InstanceScanArgs, InstanceTarget, JukeboxArgs,
        JukeboxVerb, TransportArgs, UpgradeArgs,
    },
    items::resolve_items,
    transport::{TransportMode, get_item, make_client, read_sa_token},
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Outcome of a terminal job condition.
enum JobOutcome {
    Complete,
    Failed(String),
}

/// Builds an `ApiResource` for a `vynil.solidite.fr/v1` kind.
pub(crate) fn vynil_api_resource(kind: &str, plural: &str) -> ApiResource {
    ApiResource {
        group: "vynil.solidite.fr".to_string(),
        version: "v1".to_string(),
        api_version: "vynil.solidite.fr/v1".to_string(),
        kind: kind.to_string(),
        plural: plural.to_string(),
    }
}

/// Sets a single annotation via a strategic merge patch.
async fn set_annotation(api: &Api<DynamicObject>, name: &str, key: &str, value: &str) -> Result<()> {
    let patch = serde_json::json!({ "metadata": { "annotations": { key: value } } });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .with_context(|| format!("failed to annotate {name} with {key}={value}"))?;
    Ok(())
}

/// Reads the current UID of `name`, or `None` if the job does not exist.
async fn job_uid(api: &Api<Job>, name: &str) -> Result<Option<String>> {
    Ok(api
        .get_opt(name)
        .await
        .context("failed to query existing job")?
        .and_then(|j| j.metadata.uid))
}

/// Polls until a job named `name` exists with a UID different from `old_uid`
/// (i.e. the operator has recreated it), then returns it.
async fn wait_job_recreated(
    api: &Api<Job>,
    name: &str,
    old_uid: Option<&str>,
    deadline: Instant,
    err_id: &str,
) -> Result<Job> {
    loop {
        if let Some(job) = api.get_opt(name).await.context("failed to query job")? {
            let recreated = match old_uid {
                Some(old) => job.metadata.uid.as_deref() != Some(old),
                None => true,
            };
            if recreated {
                return Ok(job);
            }
        }
        if Instant::now() >= deadline {
            bail!("{err_id}: timed out waiting for job {name} to be (re)created");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Polls a job until it reaches a terminal condition (Complete or Failed).
async fn wait_job_terminal(
    api: &Api<Job>,
    name: &str,
    deadline: Instant,
    err_id: &str,
) -> Result<JobOutcome> {
    loop {
        if let Some(job) = api.get_opt(name).await.context("failed to query job")?
            && let Some(status) = &job.status
        {
            for cond in status.conditions.iter().flatten() {
                if cond.status == "True" {
                    match cond.type_.as_str() {
                        "Complete" => return Ok(JobOutcome::Complete),
                        "Failed" => {
                            return Ok(JobOutcome::Failed(
                                cond.message.clone().unwrap_or_else(|| "no message".to_string()),
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            bail!("{err_id}: timed out waiting for job {name} to finish");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Extracts the `TAG` env value from the job's first container.
fn job_tag(job: &Job) -> Option<String> {
    let container = job.spec.as_ref()?.template.spec.as_ref()?.containers.first()?;
    container
        .env
        .as_ref()?
        .iter()
        .find(|e| e.name == "TAG")
        .and_then(|e| e.value.clone())
}

/// Annotates a (cluster-scoped) `JukeBox` and waits for its scan job to complete.
/// `filter` is `None` for a full scan, or `Some("<cat>[/<pkg>]")` for a partial one.
async fn scan_jukebox_and_wait(
    client: &Client,
    jukebox: &str,
    filter: Option<&str>,
    vynil_namespace: &str,
    timeout: u64,
) -> Result<()> {
    let job_name = format!("scan-{jukebox}");
    let ar = vynil_api_resource("JukeBox", "jukeboxes");
    // JukeBox is cluster-scoped.
    let box_api: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
    let job_api: Api<Job> = Api::namespaced(client.clone(), vynil_namespace);

    let old_uid = job_uid(&job_api, &job_name)
        .await
        .context("SCAN-ERR-02: failed to read current scan job")?;

    let value = filter.unwrap_or("true");
    eprintln!("annotating jukebox {jukebox} with force-scan={value}");
    set_annotation(&box_api, jukebox, "vynil.solidite.fr/force-scan", value)
        .await
        .context("SCAN-ERR-03: failed to trigger scan")?;

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(timeout))
        .context("SCAN-ERR-09: timeout overflow")?;
    eprintln!("waiting for scan job {vynil_namespace}/{job_name} ...");
    wait_job_recreated(&job_api, &job_name, old_uid.as_deref(), deadline, "SCAN-ERR-04").await?;
    match wait_job_terminal(&job_api, &job_name, deadline, "SCAN-ERR-05").await? {
        JobOutcome::Complete => {
            println!("scan complete: job {vynil_namespace}/{job_name} succeeded");
            Ok(())
        }
        JobOutcome::Failed(msg) => bail!("SCAN-ERR-06: scan job {job_name} failed: {msg}"),
    }
}

/// `kubectl-vynil <box> scan [<cat>[/<pkg>]]`.
///
/// # Errors
///
/// Returns an error when the Kubernetes client cannot be created, the annotation cannot
/// be set, or the job/pod wait fails.
pub async fn run_jukebox_scan(name: &str, args: JukeboxScanArgsRef<'_>, context: Option<&str>) -> Result<()> {
    let client = make_client(context)
        .await
        .context("SCAN-ERR-01: failed to create kube client")?;
    scan_jukebox_and_wait(&client, name, args.package, args.vynil_namespace, args.timeout).await
}

/// Contexte de l'erreur de lecture d'une instance. `namespace_exists` vaut `Some(false)` quand
/// le namespace est introuvable (le 404 vient alors de lui, pas de l'instance), `Some(true)` quand
/// il existe, `None` quand on ne sait pas (autre erreur que 404, ou lecture du namespace refusée).
fn instance_read_error(namespace: &str, name: &str, namespace_exists: Option<bool>) -> String {
    match namespace_exists {
        Some(false) => format!("SCAN-ERR-11: namespace \"{namespace}\" not found"),
        Some(true) => format!("SCAN-ERR-07: failed to read instance \"{name}\" in namespace \"{namespace}\""),
        None => format!(
            "SCAN-ERR-07: failed to read instance \"{name}\" in namespace \"{namespace}\" \
             (check the instance name and that the namespace exists)"
        ),
    }
}

/// `kubectl-vynil <kind> -n <ns> <inst> scan`: resolve the referenced package and
/// trigger a partial scan on its `JukeBox`.
///
/// # Errors
///
/// Returns an error when the Kubernetes client cannot be created, the annotation cannot
/// be set, or the job/pod wait fails.
pub async fn run_instance_scan(
    info: &InstanceKindInfo,
    namespace: &str,
    name: &str,
    args: &InstanceScanArgs,
    context: Option<&str>,
) -> Result<()> {
    let client = make_client(context)
        .await
        .context("SCAN-ERR-01: failed to create kube client")?;
    let ar = vynil_api_resource(info.kind, info.plural);
    let inst_api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ar);
    let obj = match inst_api.get(name).await {
        Ok(obj) => obj,
        Err(err) => {
            // Un 404 peut venir de l'instance comme du namespace : on vérifie le namespace.
            let namespace_exists = if matches!(&err, kube::Error::Api(status) if status.code == 404) {
                Api::<Namespace>::all(client.clone())
                    .get_opt(namespace)
                    .await
                    .ok()
                    .map(|found| found.is_some())
            } else {
                None
            };
            return Err(err).context(instance_read_error(namespace, name, namespace_exists));
        }
    };

    let spec = obj.data.get("spec");
    let jukebox = spec
        .and_then(|s| s.get("jukebox"))
        .and_then(|v| v.as_str())
        .context("SCAN-ERR-08: instance spec has no .spec.jukebox")?;
    let category = spec
        .and_then(|s| s.get("category"))
        .and_then(|v| v.as_str())
        .context("SCAN-ERR-09: instance spec has no .spec.category")?;
    let package = spec
        .and_then(|s| s.get("package"))
        .and_then(|v| v.as_str())
        .context("SCAN-ERR-10: instance spec has no .spec.package")?;

    let filter = format!("{category}/{package}");
    scan_jukebox_and_wait(
        &client,
        jukebox,
        Some(&filter),
        &args.vynil_namespace,
        args.timeout,
    )
    .await
}

/// `kubectl-vynil <kind> -n <ns> <inst> upgrade`.
///
/// # Errors
///
/// Returns an error when the Kubernetes client cannot be created, the annotation cannot
/// be set, or the job/pod wait fails.
pub async fn run_upgrade(
    info: &InstanceKindInfo,
    namespace: &str,
    name: &str,
    args: &UpgradeArgs,
    context: Option<&str>,
) -> Result<()> {
    let client = make_client(context)
        .await
        .context("UPG-ERR-02: failed to create kube client")?;
    let job_name = format!("{}--{}--{}", info.type_label, namespace, name);

    let ar = vynil_api_resource(info.kind, info.plural);
    let inst_api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ar);
    let job_api: Api<Job> = Api::namespaced(client.clone(), &args.vynil_namespace);

    let old_uid = job_uid(&job_api, &job_name)
        .await
        .context("UPG-ERR-03: failed to read current install job")?;

    eprintln!(
        "annotating {} {}/{} with force-reinstall",
        info.kind, namespace, name
    );
    set_annotation(&inst_api, name, "vynil.solidite.fr/force-reinstall", "true")
        .await
        .context("UPG-ERR-04: failed to trigger reinstall")?;

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(args.timeout))
        .context("UPG-ERR-09: timeout overflow")?;
    eprintln!(
        "waiting for install job {}/{} ...",
        args.vynil_namespace, job_name
    );
    let job = wait_job_recreated(&job_api, &job_name, old_uid.as_deref(), deadline, "UPG-ERR-05").await?;
    println!(
        "Installing version {}",
        job_tag(&job).unwrap_or_else(|| "<unknown>".to_string())
    );

    if args.watch {
        watch_pods(
            &client,
            &args.vynil_namespace,
            info.type_label,
            namespace,
            name,
            deadline,
        )
        .await
    } else {
        match wait_job_terminal(&job_api, &job_name, deadline, "UPG-ERR-06").await? {
            JobOutcome::Complete => {
                println!("upgrade complete: job {job_name} succeeded");
                Ok(())
            }
            JobOutcome::Failed(msg) => bail!("UPG-ERR-07: install job {job_name} failed: {msg}"),
        }
    }
}

/// Streams pod phase changes for the install (like `kubectl get pod -w`), stopping
/// once every matching pod has reached a terminal phase or the deadline elapses.
async fn watch_pods(
    client: &Client,
    vynil_namespace: &str,
    type_label: &str,
    namespace: &str,
    name: &str,
    deadline: Instant,
) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), vynil_namespace);
    let selector = format!("type={type_label},namespace={namespace},instance={name}");
    let lp = ListParams::default().labels(&selector);
    let mut last: HashMap<String, String> = HashMap::new();

    loop {
        let pod_list = pods.list(&lp).await.context("UPG-ERR-08: failed to list pods")?;
        let mut pending = false;
        for pod in &pod_list.items {
            let name = pod.metadata.name.clone().unwrap_or_default();
            let phase = pod
                .status
                .as_ref()
                .and_then(|s| s.phase.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            if last.get(&name) != Some(&phase) {
                println!("{name}\t{phase}");
                last.insert(name.clone(), phase.clone());
            }
            if !matches!(phase.as_str(), "Succeeded" | "Failed") {
                pending = true;
            }
        }
        if !last.is_empty() && !pending {
            return Ok(());
        }
        if Instant::now() >= deadline {
            eprintln!("UPG-ERR-09: watch timed out (pods may still be progressing)");
            return Ok(());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// ── Diagnostic transport ──────────────────────────────────────────────────────

/// Builds the transport mode from the shared transport flags.
///
/// `context` only applies to the aggregation path; direct mode targets the URL as-is.
fn transport_mode(t: &TransportArgs, context: Option<&str>) -> Result<(TransportMode, &'static str)> {
    match &t.server_url {
        Some(url) => {
            let token = match &t.token {
                Some(tok) => tok.clone(),
                None => read_sa_token()
                    .context("DIAG-ERR-01: no --token provided and cannot read in-cluster SA token")?,
            };
            Ok((
                TransportMode::Direct {
                    server_url: url.clone(),
                    token,
                    insecure: t.insecure,
                },
                "direct",
            ))
        }
        None => Ok((
            TransportMode::Aggregation {
                context: context.map(String::from),
            },
            "aggregation",
        )),
    }
}

/// `kubectl-vynil <kind> -n <ns> <inst> diagnostic`: bundle every item into a tar.gz.
///
/// # Errors
///
/// Returns an error when the diagnostic items cannot be fetched or the bundle cannot be
/// written.
pub async fn run_diagnostic(
    info: &InstanceKindInfo,
    namespace: &str,
    name: &str,
    args: &DiagnosticArgs,
    context: Option<&str>,
) -> Result<()> {
    let target = InstanceTarget::new(namespace, info.plural, name);
    let items = resolve_items(args.items.as_deref());
    let (mode, label) = transport_mode(&args.transport, context)?;

    let mut collected = Vec::new();
    for item in &items {
        eprintln!("collecting: {item}");
        collected.push((*item, get_item(&mode, &target, item).await));
    }

    let output = args.output.clone().unwrap_or_else(|| {
        let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        format!("{}-diag-{}.tar.gz", target.name, ts)
    });
    let output_path = PathBuf::from(&output);

    eprintln!("building bundle: {}", output_path.display());
    let summary = build_bundle(&target, label, collected, &output_path).await?;

    println!("Bundle written to: {}", summary.output_path.display());
    println!("Items collected: {}", summary.item_count);
    println!(
        "Redactions: {} distinct values, {} occurrences",
        summary.total_redactions_distinct, summary.total_redactions_occurrences
    );
    if !summary.error_items.is_empty() {
        println!("Items with errors: {}", summary.error_items.join(", "));
    }
    Ok(())
}

/// `kubectl-vynil <kind> -n <ns> <inst> <item>`: print a single diagnostic item to stdout.
///
/// # Errors
///
/// Returns an error when the item cannot be fetched, written to stdout, or returned a
/// failure status.
pub async fn run_item(
    info: &InstanceKindInfo,
    namespace: &str,
    name: &str,
    item: &str,
    transport: &TransportArgs,
    context: Option<&str>,
) -> Result<()> {
    let target = InstanceTarget::new(namespace, info.plural, name);
    let (mode, _label) = transport_mode(transport, context)?;
    let result = get_item(&mode, &target, item).await;

    if let Some((distinct, occurrences)) = result.redactions {
        eprintln!("redactions: {distinct} distinct values, {occurrences} occurrences");
    }
    std::io::stdout()
        .write_all(&result.body)
        .context("DIAG-ERR-02: failed to write item to stdout")?;

    if result.status >= 400 || result.status == 0 {
        bail!("DIAG-ERR-03: item '{}' returned status {}", item, result.status);
    }
    Ok(())
}

/// Reference bundle for the `JukeBox` scan args (avoids cloning the parsed struct).
#[derive(Debug)]
pub struct JukeboxScanArgsRef<'a> {
    pub package: Option<&'a str>,
    pub vynil_namespace: &'a str,
    pub timeout: u64,
}

/// Top-level dispatch for `kubectl-vynil jukebox …`.
///
/// # Errors
///
/// Returns an error when the Kubernetes client cannot be created, the annotation cannot
/// be set, or the job/pod wait fails.
pub async fn run_jukebox(args: &JukeboxArgs, context: Option<&str>) -> Result<()> {
    match &args.verb {
        JukeboxVerb::Scan(s) => {
            run_jukebox_scan(
                &args.name,
                JukeboxScanArgsRef {
                    package: s.package.as_deref(),
                    vynil_namespace: &s.vynil_namespace,
                    timeout: s.timeout,
                },
                context,
            )
            .await
        }
    }
}

/// Top-level dispatch for the instance kinds. Resolves the namespace, defaulting
/// to the selected kubectl context namespace when `-n` is omitted.
///
/// # Errors
///
/// Returns an error when the Kubernetes client cannot be created, the annotation cannot
/// be set, or the job/pod wait fails.
pub async fn run_instance(
    info: &InstanceKindInfo,
    args: &InstanceArgs,
    context: Option<&str>,
    namespace: Option<&str>,
) -> Result<()> {
    use crate::cli::InstanceVerb::{Diagnostic, Scan, Upgrade};
    let namespace = match namespace {
        Some(ns) => ns.to_string(),
        None => make_client(context)
            .await
            .context("INST-ERR-01: failed to create kube client")?
            .default_namespace()
            .to_string(),
    };
    if let Some((item, transport)) = args.verb.as_item() {
        return run_item(info, &namespace, &args.name, item, transport, context).await;
    }
    match &args.verb {
        Upgrade(a) => run_upgrade(info, &namespace, &args.name, a, context).await,
        Scan(a) => run_instance_scan(info, &namespace, &args.name, a, context).await,
        Diagnostic(a) => run_diagnostic(info, &namespace, &args.name, a, context).await,
        _ => unreachable!("item verbs handled above"),
    }
}

#[cfg(test)]
mod tests {
    use super::instance_read_error;

    #[test]
    fn a_missing_namespace_is_named_as_such() {
        let msg = instance_read_error("missing-ns", "my-instance", Some(false));
        assert_eq!(msg, "SCAN-ERR-11: namespace \"missing-ns\" not found");
    }

    #[test]
    fn a_missing_instance_in_an_existing_namespace_names_both() {
        let msg = instance_read_error("apps", "my-instance", Some(true));
        assert_eq!(
            msg,
            "SCAN-ERR-07: failed to read instance \"my-instance\" in namespace \"apps\""
        );
    }

    #[test]
    fn an_unknown_namespace_state_suggests_checking_it() {
        let msg = instance_read_error("apps", "my-instance", None);
        assert!(
            msg.starts_with("SCAN-ERR-07: failed to read instance \"my-instance\" in namespace \"apps\"")
        );
        assert!(
            msg.contains("check the instance name and that the namespace exists"),
            "{msg}"
        );
    }
}
