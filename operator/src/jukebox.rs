use crate::{
    Error, JukeBox, Reconciler, Result, get_client_name, instance_common::is_job_terminal, manager::Context,
    telemetry,
};
use async_trait::async_trait;
use chrono::Utc;
use common::handlebarshandler::HandleBars;
use k8s_openapi::api::batch::v1::{CronJob, Job};
use kube::{
    api::{Api, DeleteParams, Patch, PatchParams, PostParams, ResourceExt},
    runtime::{
        conditions,
        controller::Action,
        finalizer::{Event as Finalizer, finalizer},
        wait::await_condition,
    },
};
use serde_json::Value;
use std::sync::Arc;
use tokio::time::Duration;
use tracing::{Span, field, instrument};

static JUKEBOX_FINALIZER: &str = "jukeboxes.vynil.solidite.fr";

#[instrument(skip(ctx, dist), fields(trace_id))]
/// Reconcile entry point used by the controller runtime.
///
/// # Errors
///
/// Returns [`Error::FinalizerError`] when the finalizer transaction itself fails, or the
/// underlying [`Error`] raised while applying or cleaning the [`JukeBox`].
pub async fn reconcile(dist: Arc<JukeBox>, ctx: Arc<Context>) -> Result<Action> {
    let trace_id = telemetry::get_trace_id();
    Span::current().record("trace_id", field::display(&trace_id));
    if trace_id != opentelemetry::trace::TraceId::INVALID {
        Span::current().record("trace_id", field::display(&trace_id));
    }
    let _mes = ctx.metrics.jukebox.count_and_measure(&dist, &trace_id);
    ctx.diagnostics.write().await.last_event = Utc::now();
    let dists: Api<JukeBox> = Api::all(ctx.client.clone());

    finalizer(&dists, JUKEBOX_FINALIZER, dist, |event| async {
        match event {
            Finalizer::Apply(dist) => dist.reconcile(ctx.clone()).await,
            Finalizer::Cleanup(dist) => dist.cleanup(ctx.clone()).await,
        }
    })
    .await
    .map_err(|e| Error::FinalizerError(Box::new(e)))
}

#[async_trait]
impl Reconciler for JukeBox {
    // Reconcile (for non-finalizer related changes)
    async fn reconcile(&self, ctx: Arc<Context>) -> Result<Action> {
        tracing::debug!("Reconcilling JukeBox {}", self.name_any());
        ctx.diagnostics.write().await.last_event = Utc::now();
        let mut hbs = ctx.renderer.clone();
        let client = ctx.client.clone();
        let ns = ctx.client.default_namespace();
        let job_name = format!("scan-{}", self.name_any());
        let job_api: Api<Job> = Api::namespaced(client.clone(), ns);
        let force_scan = self.annotations().get("vynil.solidite.fr/force-scan").cloned();

        // Upsert cache from current status before any guard — this ensures the cache
        // is updated on every status change regardless of Job state (fixes latency and
        // cron scan cases).
        if ctx.cache_needs_update(self).await {
            ctx.upsert_jukebox_cache(self).await;
        }

        // Guard: if scan job exists and is still running, requeue.
        // Track whether a terminal job exists so we can skip re-creating it later.
        let job_is_terminal = if let Ok(Some(job)) = job_api.get_opt(&job_name).await {
            if job.metadata.deletion_timestamp.is_some() {
                return Ok(Action::requeue(Duration::from_mins(1)));
            }
            if !is_job_terminal(&job) {
                tracing::info!(
                    "JukeBox {} scan job is still running, requeuing in 1 minute",
                    self.name_any()
                );
                return Ok(Action::requeue(Duration::from_mins(1)));
            }
            true
        } else {
            false
        };

        let mut context = ctx.base_context.clone();
        if let Some(obj) = context.as_object_mut() {
            obj.insert("name".to_string(), self.name_any().into());
            obj.insert("job_name".to_string(), job_name.clone().into());
            obj.insert("schedule".to_string(), self.spec.schedule.clone().into());
        }

        // CronJob is always maintained.
        let cj_def_str = hbs.render("{{> cronscan.yaml }}", &context)?;
        let cj_def: Value = common::yamlhandler::yaml_str_to_json(&cj_def_str)?;
        let cron_api: Api<CronJob> = Api::namespaced(client.clone(), ns);
        cron_api
            .patch(
                &job_name,
                &PatchParams::apply(&get_client_name()).force(),
                &Patch::Apply(cj_def),
            )
            .await
            .map_err(Error::from)?;

        // Only create/recreate the direct Job when:
        //  - no terminal job exists (initial creation), or
        //  - force-scan annotation is present (explicit user request).
        // A terminal job without force-scan is left untouched; periodic rescans are
        // the CronJob's responsibility.
        if should_create_scan_job(job_is_terminal, force_scan.as_ref()) {
            // force-scan: delete the known terminal job and inject the package filter.
            // Annotation is removed ONLY after successful job creation so that a creation
            // failure retries on the next reconcile.
            if let Some(ref filter_value) = force_scan {
                if job_is_terminal
                    && let Ok(either::Left(j)) = job_api.delete(&job_name, &DeleteParams::foreground()).await
                {
                    wait_job_deleted(&job_api, &job_name, j).await?;
                }
                inject_package_filter(&mut context, filter_value);
            }

            // Create the Job (server-side apply; fallback to delete+create on spec conflict).
            Box::pin(upsert_scan_job(&mut hbs, &context, &job_api, &job_name)).await?;

            // Remove force-scan annotation only after successful job creation.
            if force_scan.is_some() {
                remove_force_scan_annotation(&client, &self.name_any()).await?;
            }
        }

        tracing::debug!("Reconcilling JukeBox {} Done", self.name_any());
        Ok(Action::requeue(Duration::from_mins(15)))
    }

    // Reconcile with finalize cleanup (the object was deleted)
    async fn cleanup(&self, ctx: Arc<Context>) -> Result<Action> {
        ctx.diagnostics.write().await.last_event = Utc::now();
        ctx.remove_jukebox_cache(&self.name_any()).await;
        let client = ctx.client.clone();
        let ns = ctx.client.default_namespace();
        let job_name = format!("scan-{}", self.name_any());
        let cron_api: Api<CronJob> = Api::namespaced(client.clone(), ns);
        let cron = cron_api.get_metadata_opt(&job_name).await;
        if matches!(cron, Ok(Some(_))) {
            match cron_api.delete(&job_name, &DeleteParams::foreground()).await {
                Ok(_) => {}
                Err(e) => tracing::warn!("Deleting CronJob {} failed with: {e}", &job_name),
            }
        }
        let job_api: Api<Job> = Api::namespaced(client.clone(), ns);
        let job = job_api.get_metadata_opt(&job_name).await;
        if matches!(job, Ok(Some(_))) {
            match job_api.delete(&job_name, &DeleteParams::foreground()).await {
                Ok(_) => {}
                Err(e) => tracing::warn!("Deleting Job {} failed with: {e}", &job_name),
            }
        }
        Ok(Action::await_change())
    }
}

const fn should_create_scan_job(job_is_terminal: bool, force_scan: Option<&String>) -> bool {
    !job_is_terminal || force_scan.is_some()
}

fn inject_package_filter(context: &mut Value, filter_value: &str) {
    if filter_value != "true"
        && !filter_value.is_empty()
        && let Some(obj) = context.as_object_mut()
    {
        obj.insert("package_filter".to_string(), filter_value.to_string().into());
    }
}

/// Wait until the given deleted Job object is actually gone from the API (bounded by 20s).
///
/// # Errors
///
/// Returns [`Error::Elapsed`] when the deletion was not observed within the deadline, or
/// [`Error::KubeWaitError`] when the watch failed.
async fn wait_job_deleted(job_api: &Api<Job>, job_name: &str, job: Job) -> Result<()> {
    let uid = job.metadata.uid.unwrap_or_default();
    let cond = await_condition(job_api.clone(), job_name, conditions::is_deleted(&uid));
    tokio::time::timeout(std::time::Duration::from_secs(20), cond)
        .await
        .map_err(Error::Elapsed)?
        .map_err(Error::from)
        .map(|_| ())
}

/// Render the scan job and apply it; on server-side-apply conflict, delete-then-create.
///
/// # Errors
///
/// Returns the rendering, serialization or Kubernetes error raised while upserting the Job.
async fn upsert_scan_job(
    hbs: &mut HandleBars<'_>,
    context: &Value,
    job_api: &Api<Job>,
    job_name: &str,
) -> Result<()> {
    let job_def_str = hbs.render("{{> scan.yaml }}", context)?;
    let job_def: Value = common::yamlhandler::yaml_str_to_json(&job_def_str)?;
    let patch_res = job_api
        .patch(
            job_name,
            &PatchParams::apply(&get_client_name()).force(),
            &Patch::Apply(job_def.clone()),
        )
        .await;
    if patch_res.is_err()
        && let Ok(either::Left(j)) = job_api.delete(job_name, &DeleteParams::foreground()).await
    {
        wait_job_deleted(job_api, job_name, j).await?;
        job_api
            .create(
                &PostParams::default(),
                &serde_json::from_value(job_def).map_err(Error::SerializationError)?,
            )
            .await
            .map_err(Error::from)?;
    }
    Ok(())
}

/// Remove the force-scan annotation from the [`JukeBox`] after a successful rescan kickoff.
///
/// # Errors
///
/// Returns the Kubernetes error raised by the JSON-patch call.
async fn remove_force_scan_annotation(client: &kube::Client, name: &str) -> Result<()> {
    let stms = Api::<JukeBox>::all(client.clone());
    let patch = Patch::Json::<()>(
        serde_json::from_value(serde_json::json!([
            {"op": "remove", "path": "/metadata/annotations/vynil.solidite.fr~1force-scan"}
        ]))
        .map_err(Error::SerializationError)?,
    );
    stms.patch(name, &PatchParams::default(), &patch)
        .await
        .map_err(Error::from)?;
    Ok(())
}

#[must_use]
pub fn error_policy(dist: Arc<JukeBox>, error: &Error, ctx: Arc<Context>) -> Action {
    tracing::warn!(
        "reconcile failed for JukeBox {:?}: {:?}",
        dist.metadata.name,
        error
    );
    ctx.metrics.jukebox.reconcile_failure(&dist, error);
    drop(dist);
    drop(ctx);
    Action::requeue(Duration::from_mins(5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{Context, Diagnostics, JukeCacheItem};
    use common::{
        handlebarshandler::HandleBars,
        jukebox::{JukeBoxSpec, JukeBoxStatus},
        vynilpackage::{VynilPackage, VynilPackageMeta, VynilPackageType},
    };
    use http::{Request, Response, StatusCode};
    use kube::{api::ObjectMeta, client::Body};
    use serde_json::json;
    use std::{collections::BTreeMap, pin::pin, sync::Arc};
    use tokio::sync::RwLock;

    fn make_pkg(category: &str, name: &str) -> VynilPackage {
        VynilPackage {
            registry: String::new(),
            image: String::new(),
            tag: String::new(),
            metadata: VynilPackageMeta {
                name: name.to_string(),
                category: category.to_string(),
                description: String::new(),
                app_version: None,
                usage: VynilPackageType::default(),
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: None,
            value_script: None,
        }
    }

    fn make_jukebox(name: &str, packages: Vec<VynilPackage>) -> JukeBox {
        JukeBox {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                ..Default::default()
            },
            spec: JukeBoxSpec {
                schedule: "0 * * * *".to_string(),
                pull_secret: None,
                source: None,
                maturity: None,
            },
            status: Some(JukeBoxStatus {
                conditions: vec![],
                packages,
            }),
        }
    }

    fn make_test_ctx(client: kube::Client, packages: BTreeMap<String, JukeCacheItem>) -> Arc<Context> {
        Arc::new(Context {
            client,
            diagnostics: Arc::new(RwLock::new(Diagnostics::default())),
            metrics: Arc::new(crate::Metrics::default()),
            renderer: HandleBars::new(),
            base_context: json!({}),
            packages: Arc::new(RwLock::new(packages)),
        })
    }

    fn running_job_body(name: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "apiVersion": "batch/v1",
            "kind": "Job",
            "metadata": { "name": name, "namespace": "default" },
            "status": { "active": 1 }
        }))
        .unwrap()
    }

    fn not_found_body(resource: &str, name: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "kind": "Status",
            "apiVersion": "v1",
            "status": "Failure",
            "message": format!("{} \"{}\" not found", resource, name),
            "reason": "NotFound",
            "code": 404
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn cache_updated_from_status_while_job_running() {
        let pkg = make_pkg("db", "pg");
        let jb = make_jukebox("box-a", vec![pkg.clone()]);
        let (mock_svc, handle) = tower_test::mock::pair::<Request<Body>, Response<Body>>();
        let spawned = tokio::spawn(async move {
            let mut h = pin!(handle);
            let (_req, send) = h.next_request().await.expect("expected job GET request");
            send.send_response(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/json")
                    .body(Body::from(running_job_body("scan-box-a")))
                    .unwrap(),
            );
        });
        let client = kube::Client::new(mock_svc, "default");
        let ctx = make_test_ctx(client, BTreeMap::new());
        let action = jb.reconcile(ctx.clone()).await.unwrap();
        assert_eq!(action, Action::requeue(Duration::from_mins(1)));
        let cache = ctx.packages.read().await;
        assert!(
            cache.contains_key("box-a"),
            "cache must be updated despite running job"
        );
        assert_eq!(cache["box-a"].packages, vec![pkg]);
        drop(cache);
        spawned.await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_removes_cache_entry() {
        let pkg = make_pkg("db", "pg");
        let jb = make_jukebox("box-a", vec![pkg.clone()]);
        let mut initial_cache = BTreeMap::new();
        initial_cache.insert("box-a".to_string(), JukeCacheItem {
            pull_secret: None,
            packages: vec![pkg],
        });
        let (mock_svc, handle) = tower_test::mock::pair::<Request<Body>, Response<Body>>();
        let spawned = tokio::spawn(async move {
            let mut h = pin!(handle);
            // GET CronJob scan-box-a → 404
            let (_req, send) = h.next_request().await.expect("expected CronJob GET");
            send.send_response(
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header("Content-Type", "application/json")
                    .body(Body::from(not_found_body("cronjobs", "scan-box-a")))
                    .unwrap(),
            );
            // GET Job scan-box-a → 404
            let (_req, send) = h.next_request().await.expect("expected Job GET");
            send.send_response(
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header("Content-Type", "application/json")
                    .body(Body::from(not_found_body("jobs", "scan-box-a")))
                    .unwrap(),
            );
        });
        let client = kube::Client::new(mock_svc, "default");
        let ctx = make_test_ctx(client, initial_cache);
        let _ = jb.cleanup(ctx.clone()).await.unwrap();
        let cache = ctx.packages.read().await;
        assert!(
            !cache.contains_key("box-a"),
            "cleanup must remove jukebox from cache"
        );
        drop(cache);
        spawned.await.unwrap();
    }

    use super::{inject_package_filter, should_create_scan_job};

    #[test]
    fn no_job_always_creates_scan_job() {
        assert!(should_create_scan_job(false, None));
        assert!(should_create_scan_job(false, Some(&"apps/pkg".to_string())));
    }

    #[test]
    fn terminal_job_without_force_scan_skips_job_creation() {
        assert!(!should_create_scan_job(true, None));
    }

    #[test]
    fn terminal_job_with_force_scan_recreates_job() {
        assert!(should_create_scan_job(true, Some(&"apps/monappli".to_string())));
        assert!(should_create_scan_job(true, Some(&"true".to_string())));
    }

    #[test]
    fn partial_filter_injects_package_filter() {
        let mut ctx = json!({});
        inject_package_filter(&mut ctx, "database/postgresql");
        assert_eq!(ctx["package_filter"], "database/postgresql");
    }

    #[test]
    fn true_value_does_not_inject_package_filter() {
        let mut ctx = json!({});
        inject_package_filter(&mut ctx, "true");
        assert!(ctx.get("package_filter").is_none());
    }

    #[test]
    fn empty_value_does_not_inject_package_filter() {
        let mut ctx = json!({});
        inject_package_filter(&mut ctx, "");
        assert!(ctx.get("package_filter").is_none());
    }

    #[test]
    fn category_only_filter_injects_package_filter() {
        let mut ctx = json!({});
        inject_package_filter(&mut ctx, "database");
        assert_eq!(ctx["package_filter"], "database");
    }
}
