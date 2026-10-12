//! Tests d'intégration de la bibliothèque `render_lint` (noyau et schéma) : un test par
//! scénario de `scripts/lib/render_lint.sdd`, `render_lint_schema.sdd` et des scénarios de
//! `render_lint_pod.sdd` portés par les vérifications de C0. Aucun binaire externe.

use std::sync::{Arc, Mutex};

use client::testing::{
    lint::patterns,
    schema::{registry::SchemaRegistry, rhai::schema_rhai_register},
};
use common::rhaihandler::Script;
use serde_json::{Value, json};

/// Moteur de la bibliothèque avec `regex_is_match` câblée comme dans le runner de lint.
fn lib_script() -> Script {
    let base = env!("CARGO_MANIFEST_DIR");
    let mut rhai = Script::new_mock(
        vec![format!("{base}/scripts/lib")],
        vec![],
        vec![],
        Arc::new(Mutex::new(Vec::new())),
    );
    patterns::register(&mut rhai.engine);
    rhai
}

/// Moteur de la bibliothèque avec `schema_known` / `schema_validate` câblés comme dans le
/// runner de lint de la distribution.
fn lib_script_with_schema() -> Script {
    let mut rhai = lib_script();
    let registry = SchemaRegistry::new(None, None).expect("registre de schémas natif");
    schema_rhai_register(&mut rhai.engine, registry);
    rhai
}

/// Évalue `script` et convertit le résultat en JSON.
fn eval_json(rhai: &mut Script, script: &str) -> Value {
    let value = rhai.eval(script).expect("le script doit s'évaluer");
    rhai::serde::from_dynamic::<Value>(&value).expect("résultat convertible en JSON")
}

fn deployment(namespace: &str, name: &str, image: &str) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": { "name": name, "namespace": namespace },
        "spec": {
            "replicas": 3,
            "selector": { "matchLabels": { "app": name } },
            "template": {
                "metadata": { "labels": { "app": name } },
                "spec": { "containers": [ { "name": "web", "image": image } ] }
            }
        }
    })
}

fn set_objs(rhai: &mut Script, objs: &Value) {
    rhai.set_dynamic("objs", objs);
}

fn set_obj(rhai: &mut Script, obj: &Value) {
    rhai.set_dynamic("obj", obj);
}

fn rules(findings: &Value) -> Vec<String> {
    findings
        .as_array()
        .expect("tableau de constats")
        .iter()
        .map(|f| f["rule"].as_str().unwrap_or_default().to_string())
        .collect()
}

// ===== render_lint.sdd =====

#[test]
fn finding_carries_the_object_identity() {
    let mut rhai = lib_script();
    set_obj(
        &mut rhai,
        &deployment("app", "web", "registry.example.test/web:latest"),
    );
    let f = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint;
           render_lint::finding("latest-tag", "error", obj, "image uses latest")"#,
    );
    assert_eq!(f["rule"], "latest-tag");
    assert_eq!(f["severity"], "error");
    assert_eq!(f["message"], "image uses latest");
    assert_eq!(f["object"]["kind"], "Deployment");
    assert_eq!(f["object"]["namespace"], "app");
    assert_eq!(f["object"]["name"], "web");
    assert_eq!(f["object"]["apiVersion"], "apps/v1");
}

#[test]
fn pod_templates_cover_every_workload_shape() {
    let mut rhai = lib_script();
    let cron = json!({
        "apiVersion": "batch/v1", "kind": "CronJob",
        "metadata": { "name": "c", "namespace": "app" },
        "spec": { "schedule": "* * * * *", "jobTemplate": { "spec": {
            "template": { "spec": { "containers": [ { "name": "t", "image": "alpine:3" } ] } }
        } } }
    });
    let pod = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "p" },
        "spec": { "containers": [ { "name": "x", "image": "nginx:1" } ] }
    });
    let configmap = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": { "name": "cm" }, "data": { "a": "b" }
    });
    set_obj(&mut rhai, &deployment("app", "web", "nginx:1"));
    let dep_path = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint; render_lint::pod_templates(obj)[0].path"#,
    );
    assert_eq!(dep_path, "spec.template");

    set_obj(&mut rhai, &cron);
    let cron_path = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint; render_lint::pod_templates(obj)[0].path"#,
    );
    assert_eq!(cron_path, "spec.jobTemplate.spec.template");

    set_obj(&mut rhai, &pod);
    let pod_path = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint; render_lint::pod_templates(obj)[0].path"#,
    );
    assert_eq!(pod_path, "");

    set_obj(&mut rhai, &configmap);
    let none = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint; render_lint::pod_templates(obj)"#,
    );
    assert_eq!(none, json!([]));
}

/// Objet portant un gabarit de pod (`Pod`, `CronJob` ou genre à `spec.template`) avec un
/// conteneur `nginx:latest`, pour `apiVersion` et `kind` donnés.
fn measured_object(api_version: &str, kind: &str) -> Value {
    let pod_spec = || json!({ "containers": [ { "name": "web", "image": "nginx:latest" } ] });
    let template = json!({ "metadata": { "labels": { "app": "m" } }, "spec": pod_spec() });
    let spec = match kind {
        "Pod" => pod_spec(),
        "CronJob" => json!({
            "schedule": "0 3 * * *",
            "jobTemplate": { "spec": { "template": template } }
        }),
        _ => json!({
            "replicas": 3,
            "selector": { "matchLabels": { "app": "m" } },
            "template": template
        }),
    };
    json!({
        "apiVersion": api_version,
        "kind": kind,
        "metadata": { "name": "m", "namespace": "m" },
        "spec": spec
    })
}

/// Nombre de gabarits de pod rendus par `pod_templates` pour `obj`.
fn pod_template_count(rhai: &mut Script, obj: &Value) -> usize {
    set_obj(rhai, obj);
    eval_json(
        rhai,
        r#"import "render_lint" as render_lint; render_lint::pod_templates(obj)"#,
    )
    .as_array()
    .map_or(0, Vec::len)
}

#[test]
fn a_kind_is_only_recognised_in_its_own_api_group() {
    let mut rhai = lib_script();
    assert_eq!(
        pod_template_count(&mut rhai, &measured_object("apps/v1", "Deployment")),
        1
    );
    assert_eq!(
        pod_template_count(&mut rhai, &measured_object("extensions/v1beta1", "Deployment")),
        0
    );
    assert_eq!(
        pod_template_count(&mut rhai, &measured_object("example.io/v1", "Deployment")),
        0
    );
}

/// Table mesurée contre kube-linter v0.8.3 (`latest-tag`, image `nginx:latest`) : pour chaque
/// `apiVersion` et `kind`, `true` quand l'outil signale l'objet, donc quand il porte un gabarit.
#[test]
fn pod_templates_follow_the_tool_on_every_measured_group_and_kind() {
    let table = [
        ("apps/v1", "Deployment", true),
        ("apps/v1beta1", "Deployment", true),
        ("apps/v1beta2", "Deployment", true),
        ("extensions/v1beta1", "Deployment", false),
        ("apps/v1", "StatefulSet", true),
        ("apps/v1beta1", "StatefulSet", true),
        ("apps/v1", "DaemonSet", true),
        ("extensions/v1beta1", "DaemonSet", false),
        ("apps/v1", "ReplicaSet", true),
        ("extensions/v1beta1", "ReplicaSet", false),
        ("v1", "ReplicationController", true),
        ("batch/v1", "Job", true),
        ("argoproj.io/v1alpha1", "Job", false),
        ("batch/v1", "CronJob", true),
        ("batch/v1beta1", "CronJob", true),
        ("v1", "Pod", true),
        ("v1beta1", "Pod", false),
        ("example.io/v1", "Deployment", false),
        ("example.io/v1", "Pod", false),
    ];
    let mut rhai = lib_script();
    for (api, kind, signalled) in table {
        let count = pod_template_count(&mut rhai, &measured_object(api, kind));
        assert_eq!(count > 0, signalled, "{api} {kind}");
    }
}

#[test]
fn containers_include_init_containers_with_full_paths() {
    let mut rhai = lib_script();
    let dep = json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": { "name": "web", "namespace": "app" },
        "spec": { "template": { "metadata": {}, "spec": {
            "containers": [ { "name": "main", "image": "nginx:1" } ],
            "initContainers": [ { "name": "init", "image": "busybox:1" } ]
        } } }
    });
    set_obj(&mut rhai, &dep);
    let entries = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint;
           render_lint::containers(render_lint::pod_templates(obj)[0])"#,
    );
    let entries = entries.as_array().expect("tableau");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["path"], "spec.template.spec.containers[0]");
    assert_eq!(entries[0]["init"], false);
    assert_eq!(entries[1]["path"], "spec.template.spec.initContainers[0]");
    assert_eq!(entries[1]["init"], true);
}

fn annotated_latest(value: &str) -> Value {
    let mut dep = deployment("app", "web", "registry.example.test/web:latest");
    dep["metadata"]["annotations"] = json!({ "ignore-check.example/latest-tag": value });
    dep
}

#[test]
fn no_prefix_means_no_suppression() {
    let mut rhai = lib_script();
    set_objs(&mut rhai, &json!([annotated_latest("legacy")]));
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint; render_lint::all(objs, #{}, #{only: ["latest-tag"]})"#,
    );
    let findings = findings.as_array().expect("tableau");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["rule"], "latest-tag");
    assert!(findings[0].get("suppressed").is_none_or(|s| s == &json!(false)));
}

#[test]
fn annotation_suppresses_with_its_reason() {
    let mut rhai = lib_script();
    set_objs(&mut rhai, &json!([annotated_latest("legacy")]));
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint;
           render_lint::all(objs, #{}, #{ only: ["latest-tag"], ignore_prefix: "ignore-check.example/" })"#,
    );
    let findings = findings.as_array().expect("tableau");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["suppressed"], true);
    assert_eq!(findings[0]["reason"], "legacy");
    assert_eq!(
        findings[0]["origin"],
        "annotation:ignore-check.example/latest-tag"
    );
}

#[test]
fn an_empty_reason_is_refused_when_required() {
    let mut rhai = lib_script();
    // Deux conteneurs en `latest` sur le même objet : un seul constat d'absence de raison.
    let mut dep = deployment("app", "web", "registry.example.test/web:latest");
    dep["spec"]["template"]["spec"]["containers"] = json!([
        { "name": "a", "image": "registry.example.test/a:latest" },
        { "name": "b", "image": "registry.example.test/b:latest" }
    ]);
    dep["metadata"]["annotations"] = json!({ "ignore-check.example/latest-tag": "" });
    set_objs(&mut rhai, &json!([dep]));
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint;
           render_lint::all(objs, #{}, #{ only: ["latest-tag"], ignore_prefix: "ignore-check.example/", require_reason: true })"#,
    );
    let findings = findings.as_array().expect("tableau");
    let latest: Vec<&Value> = findings.iter().filter(|f| f["rule"] == "latest-tag").collect();
    assert_eq!(latest.len(), 2, "les deux constats d'origine restent");
    assert!(
        latest
            .iter()
            .all(|f| f.get("suppressed").is_none_or(|s| s == &json!(false)))
    );
    let missing: Vec<&Value> = findings
        .iter()
        .filter(|f| f["rule"] == "suppression-without-reason")
        .collect();
    assert_eq!(missing.len(), 1, "un seul constat par objet et règle");
    assert_eq!(missing[0]["severity"], "error");
    assert!(
        missing[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ignore-check.example/latest-tag"),
        "le message nomme l'annotation"
    );
}

#[test]
fn exclude_and_only_select_checks() {
    let mut rhai = lib_script();
    let mut dep = deployment("app", "web", "registry.example.test/web:latest");
    dep["spec"]["template"]["spec"]["containers"][0]["securityContext"] = json!({});
    set_objs(&mut rhai, &json!([dep]));
    let excluded = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint; render_lint::all(objs, #{}, #{exclude: ["latest-tag"]})"#,
    );
    // Le jeu de vérifications grandit à mesure des lots : on ne vérifie que la sélection.
    assert!(!rules(&excluded).contains(&"latest-tag".to_string()));
    assert!(rules(&excluded).contains(&"no-read-only-root-fs".to_string()));
    let only = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint; render_lint::all(objs, #{}, #{only: ["latest-tag"]})"#,
    );
    assert_eq!(rules(&only), vec!["latest-tag".to_string()]);
}

#[test]
fn severity_override_applies_after_the_check() {
    let mut rhai = lib_script();
    set_objs(
        &mut rhai,
        &json!([deployment("app", "web", "registry.example.test/web:latest")]),
    );
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint;
           render_lint::all(objs, #{}, #{severity: #{"latest-tag": "warn"}})"#,
    );
    let findings = findings.as_array().expect("tableau");
    let latest = findings.iter().find(|f| f["rule"] == "latest-tag");
    assert_eq!(latest.map(|f| f["severity"].clone()), Some(json!("warn")));
    let ro = findings.iter().find(|f| f["rule"] == "no-read-only-root-fs");
    assert_eq!(ro.map(|f| f["severity"].clone()), Some(json!("error")));
}

// ===== patterns.sdd =====

#[test]
fn the_library_engine_offers_regex_is_match() {
    let mut rhai = lib_script();
    let found = eval_json(&mut rhai, r#"regex_is_match("^app$", "app")"#);
    assert_eq!(found, json!(true));
}

// ===== render_lint_pod.sdd (vérifications portées en C0) =====

#[test]
fn latest_tag_honours_the_block_list() {
    // Scénario de render_lint_pod.sdd : un BlockList qui ne couvre pas l'image ne rend aucun
    // constat, même si l'image est en `:latest`. (Le scénario d'origine parlait d'un
    // `allowList` : kube-linter n'a que `BlockList`, il n'y a pas d'écart à porter.)
    let mut rhai = lib_script();
    set_obj(&mut rhai, &deployment("app", "web", "registry/app:latest"));
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint_pod" as pod;
           pod::latest_tag(obj, [obj], #{}, #{BlockList: ["^forbidden/.*"]})"#,
    );
    assert_eq!(findings, json!([]));
}

#[test]
fn latest_tag_flags_an_image_the_custom_block_list_covers() {
    // Cas inverse : un BlockList personnalisé qui couvre l'image produit un constat.
    let mut rhai = lib_script();
    set_obj(&mut rhai, &deployment("app", "web", "registry/app:latest"));
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint_pod" as pod;
           pod::latest_tag(obj, [obj], #{}, #{BlockList: ["^registry/.*"]})"#,
    );
    assert_eq!(rules(&findings), vec!["latest-tag".to_string()]);
}

#[test]
fn non_workload_object_is_silent() {
    let mut rhai = lib_script();
    let configmap = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": { "name": "cm", "namespace": "app" },
        "data": { "image": "nginx:latest" }
    });
    set_obj(&mut rhai, &configmap);
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint_pod" as pod;
           [pod::latest_tag(obj, [obj], #{}, #{}), pod::no_read_only_root_fs(obj, [obj], #{}, #{})]"#,
    );
    assert_eq!(findings, json!([[], []]));
}

// ===== render_lint_schema.sdd =====

fn invented_field_deployment() -> Value {
    let mut dep = deployment("app", "web", "registry.example.test/web:1.0");
    dep["spec"]["template"]["spec"]["containers"][0]["inventedField"] = json!(true);
    dep
}

#[test]
fn an_invented_field_is_one_invalid_finding_with_a_dotted_path() {
    let mut rhai = lib_script_with_schema();
    set_obj(&mut rhai, &invented_field_deployment());
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint_schema" as schema; schema::check(obj)"#,
    );
    let findings = findings.as_array().expect("tableau");
    assert_eq!(findings.len(), 1, "un seul constat : {findings:?}");
    assert_eq!(findings[0]["rule"], "schema/invalid");
    assert_eq!(findings[0]["severity"], "error");
    assert_eq!(findings[0]["path"], "spec.template.spec.containers[0]");
    assert_ne!(findings[0]["message"].as_str().unwrap_or_default(), "");
}

fn certificate() -> Value {
    json!({
        "apiVersion": "cert-manager.io/v1", "kind": "Certificate",
        "metadata": { "name": "tls", "namespace": "app" },
        "spec": { "secretName": "tls" }
    })
}

#[test]
fn unknown_kind_is_a_warning_by_default() {
    let mut rhai = lib_script_with_schema();
    set_obj(&mut rhai, &certificate());
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint_schema" as schema; schema::check(obj)"#,
    );
    let findings = findings.as_array().expect("tableau");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["rule"], "schema/unknown-kind");
    assert_eq!(findings[0]["severity"], "warn");
    assert_eq!(
        findings[0]["message"],
        "no schema for cert-manager.io/v1 Certificate"
    );
}

#[test]
fn unknown_kind_can_be_an_error_or_ignored() {
    let mut rhai = lib_script_with_schema();
    set_obj(&mut rhai, &certificate());
    let as_error = eval_json(
        &mut rhai,
        r#"import "render_lint_schema" as schema; schema::check(obj, #{unknown: "error"})"#,
    );
    assert_eq!(as_error[0]["severity"], "error");
    assert_eq!(as_error[0]["rule"], "schema/unknown-kind");
    let ignored = eval_json(
        &mut rhai,
        r#"import "render_lint_schema" as schema; schema::check(obj, #{unknown: "ignore"})"#,
    );
    assert_eq!(ignored, json!([]));
}

#[test]
fn non_strict_accepts_a_field_the_schema_does_not_declare() {
    let mut rhai = lib_script_with_schema();
    set_obj(&mut rhai, &invented_field_deployment());
    let findings = eval_json(
        &mut rhai,
        r#"import "render_lint_schema" as schema; schema::check(obj, #{strict: false})"#,
    );
    assert_eq!(findings, json!([]));
}

// ===== render_lint_pod.sdd (vérifications de pod portées en C1a) =====

/// Constats de la fonction `function` du thème pod sur `obj`, avec les paramètres `params`
/// (expression Rhai, par exemple `#{}`).
fn pod_findings(rhai: &mut Script, function: &str, obj: &Value, params: &str) -> Value {
    set_obj(rhai, obj);
    eval_json(
        rhai,
        &format!(r#"import "render_lint_pod" as pod; pod::{function}(obj, [obj], #{{}}, {params})"#),
    )
}

/// Pod nu avec un volume hostPath `path` monté dans le conteneur principal.
fn pod_with_host_mount(path: &str) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "hote", "namespace": "app" },
        "spec": {
            "volumes": [ { "name": "hote", "hostPath": { "path": path } } ],
            "containers": [ {
                "name": "web", "image": "registry.example.test/web:1.4.2",
                "volumeMounts": [ { "name": "hote", "mountPath": "/hote" } ]
            } ]
        }
    })
}

/// Pod nu dont le conteneur d'initialisation porte les capacités `add` / `drop`.
fn pod_with_init_capabilities(add: &[&str], drop: &[&str]) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "init", "namespace": "app" },
        "spec": {
            "initContainers": [ {
                "name": "prepare", "image": "registry.example.test/tools:2.0.0",
                "securityContext": { "capabilities": { "add": add, "drop": drop } }
            } ],
            "containers": [ { "name": "web", "image": "registry.example.test/web:1.4.2" } ]
        }
    })
}

/// Objet de charge `kind` avec `hostIPC` / `hostNetwork` ou sans, selon `flag`.
fn host_flag_object(kind: &str, flag: &str, value: bool) -> Value {
    let pod_spec = json!({ flag: value, "containers": [ { "name": "web", "image": "registry.example.test/web:1.4.2" } ] });
    match kind {
        "CronJob" => json!({
            "apiVersion": "batch/v1", "kind": "CronJob",
            "metadata": { "name": "cron", "namespace": "app" },
            "spec": { "schedule": "0 3 * * *", "jobTemplate": { "spec": { "template": {
                "spec": pod_spec
            } } } }
        }),
        _ => json!({
            "apiVersion": "apps/v1", "kind": kind,
            "metadata": { "name": "charge", "namespace": "app" },
            "spec": { "selector": { "matchLabels": { "app": "charge" } }, "template": {
                "metadata": { "labels": { "app": "charge" } }, "spec": pod_spec
            } }
        }),
    }
}

#[test]
fn docker_sock_honours_a_custom_dirs_parameter() {
    let mut rhai = lib_script();
    let obj = pod_with_host_mount("/var/run/docker.sock");
    assert_eq!(rules(&pod_findings(&mut rhai, "docker_sock", &obj, "#{}")), vec![
        "docker-sock"
    ]);
    let custom = pod_findings(&mut rhai, "docker_sock", &obj, r#"#{dirs: ["^/custom$"]}"#);
    assert_eq!(custom, json!([]));
    let on_custom = pod_with_host_mount("/custom");
    let found = pod_findings(&mut rhai, "docker_sock", &on_custom, r#"#{dirs: ["^/custom$"]}"#);
    assert_eq!(rules(&found), vec!["docker-sock"]);
}

#[test]
fn docker_sock_flags_a_mount_in_an_init_container() {
    let mut rhai = lib_script();
    let obj = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "init", "namespace": "app" },
        "spec": {
            "volumes": [ { "name": "docker", "hostPath": { "path": "/var/run/docker.sock" } } ],
            "initContainers": [ { "name": "prepare", "image": "x:1",
                "volumeMounts": [ { "name": "docker", "mountPath": "/var/run/docker.sock" } ] } ],
            "containers": [ { "name": "web", "image": "x:1" } ]
        }
    });
    let found = pod_findings(&mut rhai, "docker_sock", &obj, "#{}");
    assert_eq!(rules(&found), vec!["docker-sock"]);
    assert_eq!(found[0]["path"], json!("spec.initContainers[0].volumeMounts"));
}

#[test]
fn drop_net_raw_honours_a_custom_forbidden_capabilities_parameter() {
    let mut rhai = lib_script();
    let obj = pod_with_init_capabilities(&["NET_RAW", "SYS_ADMIN"], &["ALL"]);
    let found = pod_findings(
        &mut rhai,
        "drop_net_raw_capability",
        &obj,
        r#"#{forbiddenCapabilities: ["SYS_ADMIN"]}"#,
    );
    assert_eq!(rules(&found), vec!["drop-net-raw-capability"]);
    assert!(
        found[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("SYS_ADMIN")
    );
}

#[test]
fn drop_net_raw_checks_an_init_container_like_a_container() {
    let mut rhai = lib_script();
    let obj = pod_with_init_capabilities(&["NET_RAW"], &[]);
    let found = pod_findings(&mut rhai, "drop_net_raw_capability", &obj, "#{}");
    // Une capacité ajoutée interdite et un retrait absent : deux constats, tous deux sur l'init.
    assert_eq!(found.as_array().map(Vec::len), Some(2));
    for f in found.as_array().into_iter().flatten() {
        assert!(
            f["path"]
                .as_str()
                .unwrap_or_default()
                .starts_with("spec.initContainers[0]")
        );
    }
}

#[test]
fn host_ipc_and_host_network_flag_the_pod_spec_of_a_cronjob() {
    let mut rhai = lib_script();
    let cron_ipc = host_flag_object("CronJob", "hostIPC", true);
    let found = pod_findings(&mut rhai, "host_ipc", &cron_ipc, "#{}");
    assert_eq!(rules(&found), vec!["host-ipc"]);
    assert_eq!(
        found[0]["path"],
        json!("spec.jobTemplate.spec.template.spec.hostIPC")
    );
    let cron_net = host_flag_object("CronJob", "hostNetwork", true);
    let found = pod_findings(&mut rhai, "host_network", &cron_net, "#{}");
    assert_eq!(rules(&found), vec!["host-network"]);
    assert_eq!(
        found[0]["path"],
        json!("spec.jobTemplate.spec.template.spec.hostNetwork")
    );
}

#[test]
fn host_flags_set_to_false_are_silent() {
    let mut rhai = lib_script();
    let off = host_flag_object("Deployment", "hostIPC", false);
    assert_eq!(pod_findings(&mut rhai, "host_ipc", &off, "#{}"), json!([]));
    let off = host_flag_object("Deployment", "hostNetwork", false);
    assert_eq!(pod_findings(&mut rhai, "host_network", &off, "#{}"), json!([]));
}

#[test]
fn host_checks_are_silent_on_non_workloads() {
    let mut rhai = lib_script();
    let service = json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": "svc", "namespace": "app" },
        "spec": { "hostIPC": true, "hostNetwork": true, "ports": [ { "port": 80 } ] }
    });
    assert_eq!(pod_findings(&mut rhai, "host_ipc", &service, "#{}"), json!([]));
    assert_eq!(
        pod_findings(&mut rhai, "host_network", &service, "#{}"),
        json!([])
    );
    let configmap = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": { "name": "cm", "namespace": "app" },
        "data": { "hostIPC": "true" }
    });
    assert_eq!(
        pod_findings(&mut rhai, "drop_net_raw_capability", &configmap, "#{}"),
        json!([])
    );
    assert_eq!(
        pod_findings(&mut rhai, "docker_sock", &configmap, "#{}"),
        json!([])
    );
}

#[test]
fn init_containers_are_checked_like_containers() {
    // Scénario de render_lint_pod.sdd : un conteneur d'init sans runAsNonRoot est signalé, et
    // son chemin finit par `initContainers[0]`.
    let mut rhai = lib_script();
    let obj = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "init", "namespace": "app" },
        "spec": {
            "initContainers": [ { "name": "prepare", "image": "x:1" } ],
            "containers": [ { "name": "web", "image": "x:1", "securityContext": { "runAsNonRoot": true } } ]
        }
    });
    let found = pod_findings(&mut rhai, "run_as_non_root", &obj, "#{}");
    assert_eq!(rules(&found), vec!["run-as-non-root"]);
    assert!(
        found[0]["path"]
            .as_str()
            .unwrap_or_default()
            .ends_with("initContainers[0]")
    );
}

#[test]
fn run_as_non_root_inherits_the_pod_setting_and_the_container_overrides_it() {
    let mut rhai = lib_script();
    let inherits = json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": { "name": "herite", "namespace": "app" },
        "spec": { "selector": { "matchLabels": { "app": "herite" } }, "template": {
            "metadata": { "labels": { "app": "herite" } },
            "spec": {
                "securityContext": { "runAsNonRoot": true },
                "containers": [ { "name": "web", "image": "x:1" } ]
            }
        } }
    });
    assert_eq!(
        pod_findings(&mut rhai, "run_as_non_root", &inherits, "#{}"),
        json!([])
    );
    let overridden = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "surcharge", "namespace": "app" },
        "spec": {
            "securityContext": { "runAsNonRoot": false },
            "containers": [ { "name": "web", "image": "x:1", "securityContext": { "runAsNonRoot": true } } ]
        }
    });
    assert_eq!(
        pod_findings(&mut rhai, "run_as_non_root", &overridden, "#{}"),
        json!([])
    );
}

#[test]
fn run_as_non_root_uses_the_inherited_user_when_the_container_sets_none() {
    // runAsNonRoot vrai au pod, runAsUser 0 au pod : le conteneur en hérite et est signalé.
    let mut rhai = lib_script();
    let obj = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "racine", "namespace": "app" },
        "spec": {
            "securityContext": { "runAsNonRoot": true, "runAsUser": 0 },
            "containers": [ { "name": "web", "image": "x:1" } ]
        }
    });
    let found = pod_findings(&mut rhai, "run_as_non_root", &obj, "#{}");
    assert_eq!(rules(&found), vec!["run-as-non-root"]);
    assert!(
        found[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("runAsUser")
    );
}

#[test]
fn privilege_escalation_matches_sys_admin_exactly_and_init_containers() {
    let mut rhai = lib_script();
    let prefixed = pod_with_init_capabilities(&["CAP_SYS_ADMIN"], &["ALL"]);
    assert_eq!(
        pod_findings(&mut rhai, "privilege_escalation_container", &prefixed, "#{}"),
        json!([])
    );
    let init_escalation = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "init", "namespace": "app" },
        "spec": {
            "initContainers": [ { "name": "prepare", "image": "x:1",
                "securityContext": { "allowPrivilegeEscalation": true } } ],
            "containers": [ { "name": "web", "image": "x:1" } ]
        }
    });
    let found = pod_findings(
        &mut rhai,
        "privilege_escalation_container",
        &init_escalation,
        "#{}",
    );
    assert_eq!(rules(&found), vec!["privilege-escalation-container"]);
    assert!(
        found[0]["path"]
            .as_str()
            .unwrap_or_default()
            .ends_with("initContainers[0].securityContext")
    );
}

#[test]
fn privilege_escalation_is_silent_when_the_field_is_unset() {
    let mut rhai = lib_script();
    let obj = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "nu", "namespace": "app" },
        "spec": { "containers": [ { "name": "web", "image": "x:1", "securityContext": { "runAsNonRoot": true } } ] }
    });
    assert_eq!(
        pod_findings(&mut rhai, "privilege_escalation_container", &obj, "#{}"),
        json!([])
    );
}

#[test]
fn privileged_container_flags_an_init_container_and_ignores_false() {
    let mut rhai = lib_script();
    let obj = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "init", "namespace": "app" },
        "spec": {
            "initContainers": [ { "name": "prepare", "image": "x:1",
                "securityContext": { "privileged": true } } ],
            "containers": [ { "name": "web", "image": "x:1", "securityContext": { "privileged": false } } ]
        }
    });
    let found = pod_findings(&mut rhai, "privileged_container", &obj, "#{}");
    assert_eq!(rules(&found), vec!["privileged-container"]);
    assert!(
        found[0]["path"]
            .as_str()
            .unwrap_or_default()
            .starts_with("spec.initContainers[0]")
    );
}

#[test]
fn host_pid_flags_the_pod_spec_of_a_daemonset_and_ignores_false() {
    let mut rhai = lib_script();
    let on = host_flag_object("DaemonSet", "hostPID", true);
    let found = pod_findings(&mut rhai, "host_pid", &on, "#{}");
    assert_eq!(rules(&found), vec!["host-pid"]);
    assert_eq!(found[0]["path"], json!("spec.template.spec.hostPID"));
    let off = host_flag_object("DaemonSet", "hostPID", false);
    assert_eq!(pod_findings(&mut rhai, "host_pid", &off, "#{}"), json!([]));
}

#[test]
fn pod_checks_of_this_batch_are_silent_on_non_workloads() {
    let mut rhai = lib_script();
    let configmap = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": { "name": "cm", "namespace": "app" },
        "data": { "privileged": "true", "runAsNonRoot": "true", "hostPID": "true" }
    });
    for function in [
        "host_pid",
        "privilege_escalation_container",
        "privileged_container",
        "run_as_non_root",
    ] {
        assert_eq!(
            pod_findings(&mut rhai, function, &configmap, "#{}"),
            json!([]),
            "{function}"
        );
    }
}

#[test]
fn sensitive_host_mounts_honours_a_custom_dirs_parameter_and_anchors_patterns() {
    let mut rhai = lib_script();
    let etc = pod_with_host_mount("/etc");
    assert_eq!(
        rules(&pod_findings(&mut rhai, "sensitive_host_mounts", &etc, "#{}")),
        vec!["sensitive-host-mounts"]
    );
    // Motif ancré : `^/etc$` ne vise pas `/etc/foo`.
    let etc_sub = pod_with_host_mount("/etc/foo");
    assert_eq!(
        pod_findings(&mut rhai, "sensitive_host_mounts", &etc_sub, "#{}"),
        json!([])
    );
    // Paramètre personnalisé : `/etc` n'est plus sensible, `/opt` l'est.
    assert_eq!(
        pod_findings(&mut rhai, "sensitive_host_mounts", &etc, r#"#{dirs: ["^/opt$"]}"#),
        json!([])
    );
    let opt = pod_with_host_mount("/opt");
    assert_eq!(
        rules(&pod_findings(
            &mut rhai,
            "sensitive_host_mounts",
            &opt,
            r#"#{dirs: ["^/opt$"]}"#
        )),
        vec!["sensitive-host-mounts"]
    );
}

#[test]
fn sensitive_host_mounts_flags_an_init_container_mount() {
    let mut rhai = lib_script();
    let obj = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "init", "namespace": "app" },
        "spec": {
            "volumes": [ { "name": "proc", "hostPath": { "path": "/proc" } } ],
            "initContainers": [ { "name": "prepare", "image": "x:1",
                "volumeMounts": [ { "name": "proc", "mountPath": "/host-proc" } ] } ],
            "containers": [ { "name": "web", "image": "x:1" } ]
        }
    });
    let found = pod_findings(&mut rhai, "sensitive_host_mounts", &obj, "#{}");
    assert_eq!(rules(&found), vec!["sensitive-host-mounts"]);
    assert_eq!(found[0]["path"], json!("spec.initContainers[0].volumeMounts"));
}

#[test]
fn unsafe_proc_mount_ignores_default_and_matches_unmasked_without_case() {
    let mut rhai = lib_script();
    let with = |value: &str| {
        json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": { "name": "proc", "namespace": "app" },
            "spec": { "containers": [ { "name": "web", "image": "x:1",
                "securityContext": { "procMount": value } } ] }
        })
    };
    assert_eq!(
        pod_findings(&mut rhai, "unsafe_proc_mount", &with("Default"), "#{}"),
        json!([])
    );
    let found = pod_findings(&mut rhai, "unsafe_proc_mount", &with("UnMasked"), "#{}");
    assert_eq!(rules(&found), vec!["unsafe-proc-mount"]);
    assert_eq!(
        found[0]["path"],
        json!("spec.containers[0].securityContext.procMount")
    );
}

#[test]
fn unsafe_sysctls_honours_a_custom_list_and_reports_each_entry_path() {
    let mut rhai = lib_script();
    let obj = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "sysctl", "namespace": "app" },
        "spec": {
            "securityContext": { "sysctls": [
                { "name": "net.core.somaxconn", "value": "1024" },
                { "name": "vm.max_map_count", "value": "262144" }
            ] },
            "containers": [ { "name": "web", "image": "x:1" } ]
        }
    });
    let found = pod_findings(&mut rhai, "unsafe_sysctls", &obj, "#{}");
    assert_eq!(rules(&found), vec!["unsafe-sysctls"]);
    assert_eq!(found[0]["path"], json!("spec.securityContext.sysctls[0]"));
    let custom = pod_findings(&mut rhai, "unsafe_sysctls", &obj, r#"#{unsafeSysCtls: ["vm."]}"#);
    assert_eq!(rules(&custom), vec!["unsafe-sysctls"]);
    assert_eq!(custom[0]["path"], json!("spec.securityContext.sysctls[1]"));
}

#[test]
fn sysctl_and_proc_mount_checks_are_silent_on_non_workloads() {
    let mut rhai = lib_script();
    let configmap = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": { "name": "cm", "namespace": "app" },
        "data": { "procMount": "Unmasked", "sysctls": "net.core.somaxconn", "dirs": "/etc" }
    });
    for function in ["sensitive_host_mounts", "unsafe_proc_mount", "unsafe_sysctls"] {
        assert_eq!(
            pod_findings(&mut rhai, function, &configmap, "#{}"),
            json!([]),
            "{function}"
        );
    }
}

// ===== render_lint_pod.sdd (vérifications de pod portées en C1b) =====

/// Constats (objet, chemin) de `function` du thème pod sur chaque objet de `objs`, avec les
/// paramètres `params` (expression Rhai).
fn pod_findings_pairs(
    rhai: &mut Script,
    function: &str,
    objs: &[Value],
    params: &str,
) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for obj in objs {
        let found = pod_findings(rhai, function, obj, params);
        for f in found.as_array().expect("tableau de constats") {
            pairs.push((
                obj["metadata"]["name"].as_str().unwrap_or_default().to_string(),
                f["path"].as_str().unwrap_or_default().to_string(),
            ));
        }
    }
    pairs
}

/// Deployment d'un seul conteneur `web` portant `resources` (JSON).
fn deployment_with_resources(name: &str, resources: &Value) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": { "name": name, "namespace": "app" },
        "spec": {
            "replicas": 1,
            "selector": { "matchLabels": { "app": name } },
            "template": {
                "metadata": { "labels": { "app": name } },
                "spec": { "containers": [ { "name": "web", "image": "x:1", "resources": resources.clone() } ] }
            }
        }
    })
}

#[test]
fn unset_cpu_requirements_honours_any_and_custom_bounds_like_kube_linter() {
    // Paramètres non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{requirementsType: any, lowerBoundMillis: 100, upperBoundMillis: 1000}` sur ces cinq
    // Deployments : `p-100-1000` (request 100m, limit 1) et `p-500-100` (request 500m, limit
    // 100m) sont signalés, un constat par type ; `p-aucun`, `p-50` et `p-2` ne le sont pas.
    let objs = vec![
        deployment_with_resources("p-aucun", &json!({})),
        deployment_with_resources(
            "p-50",
            &json!({ "requests": { "cpu": "50m" }, "limits": { "cpu": "50m" } }),
        ),
        deployment_with_resources(
            "p-100-1000",
            &json!({ "requests": { "cpu": "100m" }, "limits": { "cpu": "1" } }),
        ),
        deployment_with_resources(
            "p-500-100",
            &json!({ "requests": { "cpu": "500m" }, "limits": { "cpu": "100m" } }),
        ),
        deployment_with_resources(
            "p-2",
            &json!({ "requests": { "cpu": 2 }, "limits": { "cpu": 2 } }),
        ),
    ];
    let mut rhai = lib_script();
    let got = pod_findings_pairs(
        &mut rhai,
        "unset_cpu_requirements",
        &objs,
        r#"#{requirementsType: "any", lowerBoundMillis: 100, upperBoundMillis: 1000}"#,
    );
    let want: Vec<(String, String)> = [
        (
            "p-100-1000",
            "spec.template.spec.containers[0].resources.requests.cpu",
        ),
        (
            "p-100-1000",
            "spec.template.spec.containers[0].resources.limits.cpu",
        ),
        (
            "p-500-100",
            "spec.template.spec.containers[0].resources.requests.cpu",
        ),
        (
            "p-500-100",
            "spec.template.spec.containers[0].resources.limits.cpu",
        ),
    ]
    .into_iter()
    .map(|(n, p)| (n.to_string(), p.to_string()))
    .collect();
    assert_eq!(got, want);
}

/// Deployment d'un seul conteneur `web` décrit par `container` (JSON).
fn deployment_with_container(name: &str, container: &Value) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": { "name": name, "namespace": "app" },
        "spec": {
            "replicas": 1,
            "selector": { "matchLabels": { "app": name } },
            "template": {
                "metadata": { "labels": { "app": name } },
                "spec": { "containers": [ container ] }
            }
        }
    })
}

#[test]
fn unset_memory_requirements_honours_any_and_custom_bounds_like_kube_linter() {
    // Paramètres non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{requirementsType: any, lowerBoundMB: 96, upperBoundMB: 1024}` : `m-a-1024` (limite 1Gi,
    // borne haute incluse) et `m-c-128Mi` (demande 1G = 953,67 MB, limite 128Mi) sont signalés.
    // `m-b` (100M = 95,37 MB et 1500k = 1,43 MB) est sous la borne basse et `m-d` n'a rien.
    let objs = vec![
        deployment_with_resources("m-a-1024", &json!({ "limits": { "memory": "1Gi" } })),
        deployment_with_resources(
            "m-b",
            &json!({ "limits": { "memory": "100M" }, "requests": { "memory": "1500k" } }),
        ),
        deployment_with_resources(
            "m-c-128Mi",
            &json!({ "requests": { "memory": "1G" }, "limits": { "memory": "128Mi" } }),
        ),
        deployment_with_resources("m-d", &json!({})),
    ];
    let mut rhai = lib_script();
    let got = pod_findings_pairs(
        &mut rhai,
        "unset_memory_requirements",
        &objs,
        r#"#{requirementsType: "any", lowerBoundMB: 96, upperBoundMB: 1024}"#,
    );
    let want: Vec<(String, String)> = [
        (
            "m-a-1024",
            "spec.template.spec.containers[0].resources.limits.memory",
        ),
        (
            "m-c-128Mi",
            "spec.template.spec.containers[0].resources.requests.memory",
        ),
        (
            "m-c-128Mi",
            "spec.template.spec.containers[0].resources.limits.memory",
        ),
    ]
    .into_iter()
    .map(|(n, p)| (n.to_string(), p.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn ssh_port_honours_a_custom_port_and_protocol_like_kube_linter() {
    // Paramètres non par défaut. Sorties de kube-linter v0.8.3 :
    // - `{port: 2222, protocol: TCP}` : seul `s-pod-2222-23` (2222 TCP) ;
    // - `{port: 22, protocol: ""}` (sans filtre de protocole) : `s-tcp-22` et `s-udp-22` ;
    // - `{port: 53, protocol: UDP}` : seul le port 53 UDP de `s-udp-53`, pas son port 53 TCP.
    let tcp22 = deployment_with_container(
        "s-tcp-22",
        &json!({ "name": "web", "image": "x:1", "ports": [ { "containerPort": 22 } ] }),
    );
    let udp22 = deployment_with_container(
        "s-udp-22",
        &json!({ "name": "web", "image": "x:1", "ports": [ { "containerPort": 22, "protocol": "UDP" } ] }),
    );
    let pod = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "s-pod-2222-23", "namespace": "app" },
        "spec": { "containers": [ { "name": "web", "image": "x:1",
            "ports": [ { "containerPort": 2222 }, { "containerPort": 23 } ] } ] }
    });
    let udp53 = deployment_with_container(
        "s-udp-53",
        &json!({ "name": "web", "image": "x:1", "ports": [
            { "containerPort": 53, "protocol": "UDP" }, { "containerPort": 53 } ] }),
    );
    let objs = vec![tcp22, udp22, pod, udp53];
    let mut rhai = lib_script();

    let got = pod_findings_pairs(&mut rhai, "ssh_port", &objs, r#"#{port: 2222, protocol: "TCP"}"#);
    assert_eq!(got, vec![(
        "s-pod-2222-23".to_string(),
        "spec.containers[0].ports[0]".to_string()
    )]);

    let got = pod_findings_pairs(&mut rhai, "ssh_port", &objs, r#"#{port: 22, protocol: ""}"#);
    assert_eq!(got, vec![
        (
            "s-tcp-22".to_string(),
            "spec.template.spec.containers[0].ports[0]".to_string()
        ),
        (
            "s-udp-22".to_string(),
            "spec.template.spec.containers[0].ports[0]".to_string()
        ),
    ]);

    let got = pod_findings_pairs(&mut rhai, "ssh_port", &objs, r#"#{port: 53, protocol: "UDP"}"#);
    assert_eq!(got, vec![(
        "s-udp-53".to_string(),
        "spec.template.spec.containers[0].ports[0]".to_string()
    )]);
}

// ===== lot C1c : vérifications du thème pod (paramètres non par défaut, formes inattendues) =====

/// Constats (nom de l'objet, chemin) de `function` du thème pod, appelée sur chaque objet de
/// `objs` avec la liste entière comme `objs` : les vérifications qui consultent d'autres objets
/// (références à des `Secret` ou `ConfigMap`) en ont besoin.
fn pod_pairs_in_list(
    rhai: &mut Script,
    function: &str,
    objs: &[Value],
    params: &str,
) -> Vec<(String, String)> {
    set_objs(rhai, &Value::Array(objs.to_vec()));
    let mut pairs = Vec::new();
    for obj in objs {
        set_obj(rhai, obj);
        let found = eval_json(
            rhai,
            &format!(r#"import "render_lint_pod" as pod; pod::{function}(obj, objs, #{{}}, {params})"#),
        );
        for f in found.as_array().expect("tableau de constats") {
            pairs.push((
                obj["metadata"]["name"].as_str().unwrap_or_default().to_string(),
                f["path"].as_str().unwrap_or_default().to_string(),
            ));
        }
    }
    pairs
}

/// Deployment `name` dont le spec du gabarit de pod est `spec` (JSON).
fn deployment_with_pod_spec(name: &str, spec: &Value) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": { "name": name, "namespace": "app" },
        "spec": {
            "replicas": 1,
            "selector": { "matchLabels": { "app": name } },
            "template": { "metadata": { "labels": { "app": name } }, "spec": spec.clone() }
        }
    })
}

/// Conteneur `web` de l'image de référence des fixtures.
fn web_container() -> Value {
    json!({ "name": "web", "image": "registry.example.test/web:1.4.2" })
}

#[test]
fn default_service_account_honours_a_custom_pattern_like_kube_linter() {
    // Paramètre non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{serviceAccount: "^app-sa$"}` : `sa-app-sa` (serviceAccountName) et le pod `pod-app-sa`
    // sont signalés ; `sa-default`, `sa-absent` et `sa-autre` ne le sont pas.
    let objs = vec![
        deployment_with_pod_spec(
            "sa-app-sa",
            &json!({ "serviceAccountName": "app-sa", "containers": [web_container()] }),
        ),
        deployment_with_pod_spec(
            "sa-default",
            &json!({ "serviceAccountName": "default", "containers": [web_container()] }),
        ),
        deployment_with_pod_spec("sa-absent", &json!({ "containers": [web_container()] })),
        deployment_with_pod_spec(
            "sa-autre",
            &json!({ "serviceAccountName": "autre-sa", "containers": [web_container()] }),
        ),
        json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": { "name": "pod-app-sa", "namespace": "app" },
            "spec": { "serviceAccountName": "app-sa", "containers": [web_container()] }
        }),
    ];
    let mut rhai = lib_script();
    let got = pod_pairs_in_list(
        &mut rhai,
        "default_service_account",
        &objs,
        r#"#{serviceAccount: "^app-sa$"}"#,
    );
    let want: Vec<(String, String)> = [
        ("sa-app-sa", "spec.template.spec.serviceAccountName"),
        ("pod-app-sa", "spec.serviceAccountName"),
    ]
    .into_iter()
    .map(|(n, p)| (n.to_string(), p.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn dnsconfig_options_honours_a_custom_key_and_value_like_kube_linter() {
    // Paramètres non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{Key: "timeout", Value: "1"}` : `dns-t2` (timeout 2), `dns-absent` (pas de dnsConfig) et
    // `dns-ndots2` (ndots 2 seulement) sont signalés ; `dns-t1` et `dns-timeout-5-puis-1`
    // (timeout 1 présent parmi d'autres options) ne le sont pas.
    let dns =
        |options: Value| json!({ "dnsConfig": { "options": options }, "containers": [web_container()] });
    let objs = vec![
        deployment_with_pod_spec("dns-t1", &dns(json!([{ "name": "timeout", "value": "1" }]))),
        deployment_with_pod_spec("dns-t2", &dns(json!([{ "name": "timeout", "value": "2" }]))),
        deployment_with_pod_spec("dns-absent", &json!({ "containers": [web_container()] })),
        deployment_with_pod_spec("dns-ndots2", &dns(json!([{ "name": "ndots", "value": "2" }]))),
        deployment_with_pod_spec(
            "dns-timeout-5-puis-1",
            &dns(json!([{ "name": "timeout", "value": "5" }, { "name": "timeout", "value": "1" }])),
        ),
    ];
    let mut rhai = lib_script();
    let got = pod_pairs_in_list(
        &mut rhai,
        "dnsconfig_options",
        &objs,
        r#"#{Key: "timeout", Value: "1"}"#,
    );
    let want: Vec<(String, String)> = [
        ("dns-t2", "spec.template.spec.dnsConfig"),
        ("dns-absent", "spec.template.spec.dnsConfig"),
        ("dns-ndots2", "spec.template.spec.dnsConfig"),
    ]
    .into_iter()
    .map(|(n, p)| (n.to_string(), p.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn priority_class_name_honours_a_custom_accepted_list_like_kube_linter() {
    // Paramètre non par défaut. Sortie de kube-linter v0.8.3 avec
    // `{acceptedPriorityClassNames: ["high", "app-critical"]}` : `prio-system-node` et
    // `prio-high-x` sont signalés ; `prio-high`, `prio-app-critical` et `prio-absent` non. La
    // correspondance est exacte : `high-x` ne vaut pas `high`.
    let spec = |class: &str| json!({ "priorityClassName": class, "containers": [web_container()] });
    let objs = vec![
        deployment_with_pod_spec("prio-high", &spec("high")),
        deployment_with_pod_spec("prio-app-critical", &spec("app-critical")),
        deployment_with_pod_spec("prio-system-node", &spec("system-node-critical")),
        deployment_with_pod_spec("prio-absent", &json!({ "containers": [web_container()] })),
        deployment_with_pod_spec("prio-high-x", &spec("high-x")),
    ];
    let mut rhai = lib_script();
    let got = pod_pairs_in_list(
        &mut rhai,
        "priority_class_name",
        &objs,
        r#"#{acceptedPriorityClassNames: ["high", "app-critical"]}"#,
    );
    assert_eq!(got, vec![
        (
            "prio-system-node".to_string(),
            "spec.template.spec.priorityClassName".to_string()
        ),
        (
            "prio-high-x".to_string(),
            "spec.template.spec.priorityClassName".to_string()
        ),
    ]);
}

#[test]
fn env_var_secret_honours_a_custom_name_and_value_pattern_like_kube_linter() {
    // Paramètres non par défaut. Sortie de kube-linter v0.8.3 avec `{name: "^DB_", value: "^s"}` :
    // `DB_TOKEN_STORE` (conteneur web) et `DB_KEY` (conteneur side) sont signalés. `DB_PASSWORD`
    // (valeur « p »), `db_secret` (casse), `XDB_SECRET` (nom) et `DB_EMPTY_SECRET` (vide) ne le sont pas.
    let objs = vec![deployment_with_pod_spec(
        "env-db",
        &json!({
            "containers": [
                { "name": "web", "image": "registry.example.test/web:1.4.2", "env": [
                    { "name": "DB_PASSWORD", "value": "p" },
                    { "name": "DB_TOKEN_STORE", "value": "s1" },
                    { "name": "db_secret", "value": "s" },
                    { "name": "XDB_SECRET", "value": "t" },
                    { "name": "DB_EMPTY_SECRET", "value": "" },
                    { "name": "DB_SECRETREF", "valueFrom": { "secretKeyRef": { "name": "x", "key": "k" } } }
                ] },
                { "name": "side", "image": "registry.example.test/side:1.0.0", "env": [
                    { "name": "DB_KEY", "value": "s2" }
                ] }
            ]
        }),
    )];
    let mut rhai = lib_script();
    let got = pod_pairs_in_list(
        &mut rhai,
        "env_var_secret",
        &objs,
        r#"#{name: "^DB_", value: "^s"}"#,
    );
    let want: Vec<(String, String)> = [
        ("env-db", "spec.template.spec.containers[0].env[1].value"),
        ("env-db", "spec.template.spec.containers[1].env[0].value"),
    ]
    .into_iter()
    .map(|(n, p)| (n.to_string(), p.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn env_value_from_honours_custom_ignored_secrets_and_configmaps_like_kube_linter() {
    // Paramètres non par défaut. Sortie de kube-linter v0.8.3 avec `{IgnoredSecrets: ["ign-s"],
    // IgnoredConfigMaps: ["ign-cm"]}` : les motifs sont des expressions régulières non ancrées.
    // `ign-s-autre` est ignoré par « ign-s » ; `ev-ignored` ne produit rien ; `ev-manquant`
    // produit deux constats (le ConfigMap `miss-cm` de l'initialisation, le Secret `miss-s`).
    let ignored = deployment_with_pod_spec(
        "ev-ignored",
        &json!({ "containers": [ { "name": "web", "image": "registry.example.test/web:1.4.2", "env": [
            { "name": "T", "valueFrom": { "secretKeyRef": { "name": "ign-s", "key": "k" } } },
            { "name": "U", "valueFrom": { "configMapKeyRef": { "name": "ign-cm", "key": "k" } } }
        ] } ] }),
    );
    let missing = deployment_with_pod_spec(
        "ev-manquant",
        &json!({
            "initContainers": [ { "name": "init", "image": "registry.example.test/init:1.0.0", "env": [
                { "name": "V", "valueFrom": { "configMapKeyRef": { "name": "miss-cm", "key": "k" } } }
            ] } ],
            "containers": [ { "name": "web", "image": "registry.example.test/web:1.4.2", "env": [
                { "name": "W", "valueFrom": { "secretKeyRef": { "name": "miss-s", "key": "k" } } },
                { "name": "X", "valueFrom": { "secretKeyRef": { "name": "ign-s-autre", "key": "k" } } }
            ] } ]
        }),
    );
    let objs = vec![ignored, missing];
    let mut rhai = lib_script();
    let mut got = pod_pairs_in_list(
        &mut rhai,
        "env_value_from",
        &objs,
        r#"#{IgnoredSecrets: ["ign-s"], IgnoredConfigMaps: ["ign-cm"]}"#,
    );
    got.sort();
    let mut want: Vec<(String, String)> = [
        (
            "ev-manquant",
            "spec.template.spec.containers[0].env[0].valueFrom.secretKeyRef",
        ),
        (
            "ev-manquant",
            "spec.template.spec.initContainers[0].env[0].valueFrom.configMapKeyRef",
        ),
    ]
    .into_iter()
    .map(|(n, p)| (n.to_string(), p.to_string()))
    .collect();
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn the_pod_batch_does_not_raise_on_unexpected_shapes() {
    // Une forme inattendue (spec non objet, conteneurs ou variables de mauvais type) ne lève pas
    // et ne produit aucun constat : la vérification retourne un tableau vide.
    let functions = [
        "default_service_account",
        "dnsconfig_options",
        "duplicate_env_var",
        "env_value_from",
        "env_var_secret",
        "priority_class_name",
        "read_secret_from_env_var",
        "restart_policy",
    ];
    let mut rhai = lib_script();
    let odd_spec = vec![deployment_with_pod_spec("forme-spec", &json!("oops"))];
    for function in functions {
        assert!(
            pod_pairs_in_list(&mut rhai, function, &odd_spec, "#{}").is_empty(),
            "{function} sur une spec non objet"
        );
    }
    let odd_env = vec![deployment_with_pod_spec(
        "forme-env",
        &json!({
            "serviceAccountName": "app-sa",
            "restartPolicy": "Always",
            "priorityClassName": 7,
            "dnsConfig": "oops",
            "containers": [
                { "name": 3, "image": "x:1", "env": "oops" },
                { "name": "b", "image": "x:1", "env": [
                    "x",
                    { "name": 5, "value": 6 },
                    { "name": "T", "valueFrom": "x" },
                    { "name": "U", "valueFrom": { "secretKeyRef": 3, "configMapKeyRef": "y" } }
                ] }
            ]
        }),
    )];
    for function in functions {
        assert!(
            pod_pairs_in_list(&mut rhai, function, &odd_env, "#{}").is_empty(),
            "{function} sur des conteneurs ou variables de forme inattendue"
        );
    }
}

// ===== render_lint_workload.sdd (vérifications de workload portées en C2a) =====

/// Objets du manifeste de la fixture `name` (multi-documents, sans les documents vides).
fn fixture_objects(name: &str) -> Vec<Value> {
    use serde::Deserialize as _;
    let path = format!(
        "{}/tests/fixtures/render_lint/{name}/manifests.yaml",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).expect("manifestes lisibles");
    serde_yaml::Deserializer::from_str(&text)
        .map(|doc| Value::deserialize(doc).expect("manifeste YAML valide"))
        .filter(|value: &Value| !value.is_null())
        .collect()
}

/// Constats de la vérification `check` sur `objs`, avec la carte de paramètres `params`
/// (expression Rhai), par le point d'entrée `render_lint::all`.
fn check_findings(check: &str, objs: &[Value], params: &str) -> Vec<String> {
    let mut rhai = lib_script();
    set_objs(&mut rhai, &Value::Array(objs.to_vec()));
    let found = eval_json(
        &mut rhai,
        &format!(
            r#"import "render_lint" as render_lint;
               render_lint::all(objs, #{{}}, #{{only: ["{check}"], params: #{{"{check}": {params}}}}})"#
        ),
    );
    let mut names: Vec<String> = found
        .as_array()
        .expect("tableau de constats")
        .iter()
        .map(|f| {
            format!(
                "{}/{}",
                f["object"]["kind"].as_str().unwrap_or_default(),
                f["object"]["name"].as_str().unwrap_or_default()
            )
        })
        .collect();
    names.sort();
    names
}

fn single_replica_deployment() -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": { "name": "web", "namespace": "app" },
        "spec": {
            "replicas": 1,
            "selector": { "matchLabels": { "app": "web" } },
            "template": { "metadata": { "labels": { "app": "web" } }, "spec": { "containers": [ web_container() ] } }
        }
    })
}

#[test]
fn single_replica_deployment_is_reported() {
    let obj = single_replica_deployment();
    let mut rhai = lib_script();
    set_obj(&mut rhai, &obj);
    let found = eval_json(
        &mut rhai,
        "import \"render_lint_workload\" as workload; workload::minimum_three_replicas(obj, [obj], #{}, #{})",
    );
    assert_eq!(rules(&found), vec!["minimum-three-replicas".to_string()]);
    assert_eq!(found[0]["object"]["name"], "web");
    assert_eq!(found[0]["object"]["kind"], "Deployment");
}

#[test]
fn minimum_three_replicas_honours_min_replicas_like_kube_linter() {
    // Paramètre non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{minReplicas: 2}` sur la fixture minimum-three-replicas : `two-dep` et `rc-two` (deux
    // réplicas) ne sont plus signalés ; `one-dep`, `default-dep`, `zero-dep`, `db-sts`, `rs-one`
    // et `beta-dep` le sont encore.
    let objs = fixture_objects("minimum-three-replicas");
    let got = check_findings("minimum-three-replicas", &objs, r"#{minReplicas: 2}");
    let want = vec![
        "Deployment/beta-dep",
        "Deployment/default-dep",
        "Deployment/one-dep",
        "Deployment/zero-dep",
        "ReplicaSet/rs-one",
        "StatefulSet/db-sts",
    ];
    assert_eq!(got, want);
}

fn hpa(name: &str, target: &Value) -> Value {
    json!({
        "apiVersion": "autoscaling/v2", "kind": "HorizontalPodAutoscaler",
        "metadata": { "name": name, "namespace": "app" },
        "spec": { "scaleTargetRef": target, "minReplicas": 3, "maxReplicas": 9 }
    })
}

#[test]
fn cross_object_check_uses_the_object_list() {
    // Scénario de render_lint_workload.sdd : la cible du HPA n'est pas dans la liste d'objets.
    let hpa = hpa(
        "web",
        &json!({ "apiVersion": "apps/v1", "kind": "Deployment", "name": "missing" }),
    );
    let got = check_findings("dangling-horizontalpodautoscaler", &[hpa], "#{}");
    assert_eq!(got, vec!["HorizontalPodAutoscaler/web"]);
}

#[test]
fn a_matching_target_is_not_dangling() {
    let target = single_replica_deployment();
    let hpa = hpa(
        "web",
        &json!({ "apiVersion": "apps/v1", "kind": "Deployment", "name": "web" }),
    );
    let got = check_findings("dangling-horizontalpodautoscaler", &[target, hpa], "#{}");
    assert!(got.is_empty(), "cible présente : {got:?}");
}

#[test]
fn dangling_horizontalpodautoscaler_resolves_by_group_version_and_kind_like_kube_linter() {
    // Sortie de kube-linter v0.8.3 sur la fixture dangling-horizontalpodautoscaler : le groupe
    // (example.io Deployment ne résout pas apps/v1), la version (apps/v1beta2 ne résout pas
    // apps/v1), le genre (Service, ReplicationController, Pod ne sont pas des cibles) et le
    // namespace (h-other-ns-target résolu sur une cible d'un autre namespace) sont mesurés.
    let objs = fixture_objects("dangling-horizontalpodautoscaler");
    let got = check_findings("dangling-horizontalpodautoscaler", &objs, "#{}");
    let want = vec![
        "HorizontalPodAutoscaler/h-ext",
        "HorizontalPodAutoscaler/h-kind-mismatch",
        "HorizontalPodAutoscaler/h-missing",
        "HorizontalPodAutoscaler/h-noapi",
        "HorizontalPodAutoscaler/h-other-ns-missing",
        "HorizontalPodAutoscaler/h-pod",
        "HorizontalPodAutoscaler/h-rc",
        "HorizontalPodAutoscaler/h-svc",
        "HorizontalPodAutoscaler/h-tiers-group",
        "HorizontalPodAutoscaler/h-v1-missing",
        "HorizontalPodAutoscaler/h-version-mismatch",
    ];
    assert_eq!(got, want);
}

#[test]
fn hpa_minimum_replicas_honours_min_replicas_like_kube_linter() {
    // Paramètre non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{minReplicas: 2}` (template hpa-minimum-replicas) sur la fixture hpa-minimum-three-replicas :
    // absent, null, one et zero restent signalés ; two, v1 et v2beta2 (deux réplicas) ne le sont plus.
    let objs = fixture_objects("hpa-minimum-three-replicas");
    let got = check_findings("hpa-minimum-three-replicas", &objs, "#{minReplicas: 2}");
    let want = vec![
        "HorizontalPodAutoscaler/absent-hpa",
        "HorizontalPodAutoscaler/null-hpa",
        "HorizontalPodAutoscaler/one-hpa",
        "HorizontalPodAutoscaler/zero-hpa",
    ];
    assert_eq!(got, want);
}

#[test]
fn required_annotation_email_honours_key_and_value_like_kube_linter() {
    // Paramètres non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{key: owner, value: "example\\.com$"}` (template required-annotation) sur la fixture
    // required-annotation-email : la paire owner/example.com est reconnue, et seuls les objets
    // sans annotation owner valide sont signalés (bad-mixed-pair et bad-other-key ne le sont plus).
    let objs = fixture_objects("required-annotation-email");
    let got = check_findings(
        "required-annotation-email",
        &objs,
        r#"#{key: "owner", value: "example\\.com$"}"#,
    );
    let want = vec![
        "CronJob/bad-cron",
        "DaemonSet/bad-ds",
        "Deployment/bad-absent",
        "Deployment/bad-beta",
        "Deployment/bad-empty",
        "Deployment/bad-template-only",
        "Deployment/bad-value",
        "Deployment/ok-dep",
        "Deployment/ok-key-contains",
        "Deployment/ok-value-contains",
        "Job/bad-job",
        "Pod/bad-pod",
        "ReplicaSet/bad-rs",
        "ReplicationController/bad-rc",
        "StatefulSet/bad-sts",
    ];
    assert_eq!(got, want);
}

#[test]
fn metadata_labels_and_annotations_are_silent_when_malformed() {
    // Métadonnées de mauvais type (annotations et labels qui ne sont pas des cartes) : aucune
    // levée, aucun constat des deux vérifications d'étiquettes.
    let odd = vec![json!({ "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": { "name": "m", "namespace": "app", "annotations": "x", "labels": [ 1 ] },
        "spec": { "replicas": 3 } })];
    let mut rhai = lib_script();
    set_objs(&mut rhai, &Value::Array(odd));
    let found = eval_json(
        &mut rhai,
        r#"import "render_lint" as render_lint; render_lint::all(objs, #{}, #{only: ["required-annotation-email", "required-label-owner"]})"#,
    );
    assert_eq!(found, json!([]), "métadonnées malformées : aucun constat");
}

#[test]
fn required_label_owner_honours_key_like_kube_linter() {
    // Paramètre non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{key: team}` (template required-label) sur la fixture required-label-owner : ok-key-prefix
    // (label owner-team, clé qui contient team) n'est plus signalé ; ok-dep (label owner seul)
    // l'est, la clé team étant absente.
    let objs = fixture_objects("required-label-owner");
    let got = check_findings("required-label-owner", &objs, r#"#{key: "team"}"#);
    let want = vec![
        "CronJob/bad-cron",
        "DaemonSet/bad-ds",
        "Deployment/bad-absent",
        "Deployment/bad-beta",
        "Deployment/bad-case",
        "Deployment/bad-template-only",
        "Deployment/ok-dep",
        "Deployment/ok-empty-value",
        "Deployment/ok-key-suffix",
        "Job/bad-job",
        "Pod/bad-pod",
        "ReplicaSet/bad-rs",
        "ReplicationController/bad-rc",
        "StatefulSet/bad-sts",
    ];
    assert_eq!(got, want);
}

#[test]
fn no_anti_affinity_honours_min_replicas_like_kube_linter() {
    // Paramètre non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{minReplicas: 3}` sur la fixture no-anti-affinity : seul `none-dep` (trois réplicas sans
    // anti-affinité) est signalé ; les deux réplicas de `empty-aa`, `db-sts` et `rc-two` ne le
    // sont plus.
    let objs = fixture_objects("no-anti-affinity");
    let got = check_findings("no-anti-affinity", &objs, r"#{minReplicas: 3}");
    assert_eq!(got, vec!["Deployment/none-dep"]);
}

#[test]
fn no_rolling_update_strategy_honours_the_regex_like_kube_linter() {
    // Paramètre non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{strategyTypeRegex: "^Recreate$"}` sur la fixture no-rolling-update-strategy : les
    // stratégies RollingUpdate deviennent des constats (`ok-rolling`, `ok-sts`, `ok-ds`), et
    // `dep-recreate` ne l'est plus.
    let objs = fixture_objects("no-rolling-update-strategy");
    let got = check_findings(
        "no-rolling-update-strategy",
        &objs,
        r#"#{strategyTypeRegex: "^Recreate$"}"#,
    );
    let want = vec![
        "DaemonSet/ds-none",
        "DaemonSet/ok-ds",
        "Deployment/beta-dep",
        "Deployment/dep-none",
        "Deployment/ok-rolling",
        "StatefulSet/ok-sts",
        "StatefulSet/sts-ondelete",
    ];
    assert_eq!(got, want);
}

#[test]
fn no_extensions_v1beta_honours_group_and_version_like_kube_linter() {
    // Paramètres non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `{group: apps, version: "v1beta.+"}` sur la fixture no-extensions-v1beta : seul le
    // Deployment apps/v1beta1 est signalé ; les objets extensions/v1beta1 ne le sont plus.
    let objs = fixture_objects("no-extensions-v1beta");
    let got = check_findings(
        "no-extensions-v1beta",
        &objs,
        r#"#{group: "apps", version: "v1beta.+"}"#,
    );
    assert_eq!(got, vec!["Deployment/beta-apps-dep"]);
}

#[test]
fn the_workload_batch_does_not_raise_on_unexpected_shapes() {
    // Formes inattendues (spec, gabarit, sélecteur, affinité, stratégie, champs de mauvais type) :
    // aucune levée, et aucun constat du thème workload.
    let functions = [
        "minimum_three_replicas",
        "no_anti_affinity",
        "no_extensions_v1beta",
        "no_node_affinity",
        "no_rolling_update_strategy",
        "job_ttl_seconds_after_finished",
        "mismatching_selector",
        "deprecated_service_account_field",
        "use_namespace",
        "dangling_horizontalpodautoscaler",
        "hpa_minimum_three_replicas",
        "pdb_max_unavailable",
        "pdb_min_available",
        "pdb_unhealthy_pod_eviction_policy",
    ];
    let odd = vec![
        json!({ "apiVersion": "autoscaling/v2", "kind": "HorizontalPodAutoscaler", "metadata": { "name": "h", "namespace": "app" },
                "spec": { "scaleTargetRef": "y", "minReplicas": "x" } }),
        json!({ "apiVersion": "autoscaling/v2", "kind": "HorizontalPodAutoscaler", "metadata": { "name": "i", "namespace": "app" }, "spec": 4 }),
        json!({ "apiVersion": "policy/v1", "kind": "PodDisruptionBudget", "metadata": { "name": "j", "namespace": "app" },
                "spec": { "maxUnavailable": [ 1 ], "minAvailable": { "x": 1 }, "selector": "x", "unhealthyPodEvictionPolicy": 3 } }),
        json!({ "apiVersion": "policy/v1", "kind": "PodDisruptionBudget", "metadata": { "name": "k", "namespace": "app" },
                "spec": { "minAvailable": "50%", "unhealthyPodEvictionPolicy": "AlwaysAllow", "selector": { "matchLabels": "x", "matchExpressions": "y" } } }),
        json!({ "apiVersion": "policy/v1", "kind": "PodDisruptionBudget", "metadata": { "name": "l", "namespace": "app" }, "spec": "oops" }),
        json!({ "apiVersion": "apps/v1", "kind": "Deployment", "metadata": { "name": "a", "namespace": "app" }, "spec": "oops" }),
        json!({ "apiVersion": "apps/v1", "kind": "Deployment", "metadata": { "name": "b", "namespace": "app" },
                "spec": { "replicas": "trois", "selector": "x", "strategy": 3, "template": "y" } }),
        json!({ "apiVersion": "apps/v1", "kind": "StatefulSet", "metadata": { "name": "c", "namespace": "app" },
                "spec": { "replicas": "uno", "updateStrategy": "z", "selector": { "matchLabels": "x", "matchExpressions": "y" },
                          "template": { "metadata": { "labels": { "app": "c" } }, "spec": { "affinity": "z", "serviceAccount": 4, "containers": "w" } } } }),
        json!({ "apiVersion": "batch/v1", "kind": "CronJob", "metadata": { "name": "d", "namespace": "app" },
                "spec": { "jobTemplate": "oops" } }),
        json!({ "apiVersion": "batch/v1", "kind": "Job", "metadata": { "name": "e", "namespace": "app" },
                "spec": { "ttlSecondsAfterFinished": "100", "selector": { "matchExpressions": [ "x" ] },
                          "template": { "spec": { "affinity": { "nodeAffinity": 5 }, "containers": [ "x" ] } } } }),
        json!({ "apiVersion": "v1", "kind": "Pod", "metadata": { "name": "f", "namespace": "app" }, "spec": 7 }),
        json!({ "apiVersion": "v1", "kind": "ReplicationController", "metadata": { "name": "g", "namespace": "app" },
                "spec": { "replicas": 2.5, "template": { "spec": { "affinity": "z" } } } }),
    ];
    let mut rhai = lib_script();
    set_objs(&mut rhai, &Value::Array(odd));
    let only = functions
        .iter()
        .map(|f| format!("\"{}\"", f.replace('_', "-")))
        .collect::<Vec<_>>()
        .join(", ");
    let found = eval_json(
        &mut rhai,
        &format!(
            r#"import "render_lint" as render_lint; render_lint::all(objs, #{{}}, #{{only: [{only}]}})"#
        ),
    );
    assert_eq!(found, json!([]), "aucun constat sur des formes inattendues");
}

#[test]
fn a_secret_or_configmap_reference_resolves_only_to_the_core_kind() {
    // Un ConfigMap d'un groupe tiers, portant le nom référencé, n'est pas le ConfigMap du noyau :
    // la référence reste sans objet et doit être signalée. Le ConfigMap du noyau la résout.
    let consumer = deployment_with_pod_spec(
        "ev-tiers",
        &json!({ "containers": [ { "name": "web", "image": "registry.example.test/web:1.4.2", "env": [
            { "name": "U", "valueFrom": { "configMapKeyRef": { "name": "cfg", "key": "k" } } }
        ] } ] }),
    );
    let tiers_configmap = json!({
        "apiVersion": "example.io/v1", "kind": "ConfigMap",
        "metadata": { "name": "cfg", "namespace": "app" }, "data": { "k": "v" }
    });
    let core_configmap = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": { "name": "cfg", "namespace": "app" }, "data": { "k": "v" }
    });
    let unresolved = check_findings("env-value-from", &[consumer.clone(), tiers_configmap], "#{}");
    assert_eq!(unresolved, vec!["Deployment/ev-tiers".to_string()]);
    let resolved = check_findings("env-value-from", &[consumer, core_configmap], "#{}");
    assert!(resolved.is_empty(), "ConfigMap du noyau : {resolved:?}");
}

// ===== Thème network : dangling-service, exposed-services =====

#[test]
fn service_selecting_no_pod_is_dangling() {
    // Scénario de render_lint_network.sdd : aucun gabarit de pod ne correspond au sélecteur.
    let svc = json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": "web", "namespace": "app" },
        "spec": { "selector": { "app": "web" }, "ports": [ { "port": 80 } ] }
    });
    let got = check_findings("dangling-service", &[svc], "#{}");
    assert_eq!(got, vec!["Service/web"]);
}

#[test]
fn dangling_service_resolves_like_kube_linter() {
    // Sortie de kube-linter v0.8.3 sur la fixture dangling-service : le groupe retenu dépend du
    // genre (Deployment apps et extensions, ReplicaSet et DaemonSet apps et extensions, StatefulSet
    // apps seulement ; Deployment du noyau ou example.io ignoré), le namespace est comparé, les labels
    // lus sont ceux du gabarit (pas metadata), un sélecteur vide ne correspond à rien, ExternalName
    // exact est exempté et un Service mal formé est ignoré.
    let objs = fixture_objects("dangling-service");
    let got = check_findings("dangling-service", &objs, "#{}");
    let want = vec![
        "Service/svc-core-dep",
        "Service/svc-cross-ns",
        "Service/svc-empty-selector",
        "Service/svc-ext-sts",
        "Service/svc-externalname-lower",
        "Service/svc-extra-key",
        "Service/svc-metadata-only",
        "Service/svc-no-match",
        "Service/svc-no-selector",
        "Service/svc-nodeport",
        "Service/svc-nons-ns",
        "Service/svc-pod-other-ns",
        "Service/svc-tiers",
    ];
    assert_eq!(got, want);
}

#[test]
fn exposed_services_honours_forbidden_types_like_kube_linter() {
    // Paramètre non par défaut. Sortie de kube-linter v0.8.3 avec le check personnalisé
    // `forbiddenServiceTypes` sur la fixture exposed-services : `["ClusterIP", "ExternalName"]`
    // signale clusterip et externalname ; `["nodeport"]` signale les trois nodeport (casse ignorée) ;
    // une liste vide ne signale rien.
    let objs = fixture_objects("exposed-services");
    let got = check_findings(
        "exposed-services",
        &objs,
        r#"#{forbiddenServiceTypes: ["ClusterIP", "ExternalName"]}"#,
    );
    assert_eq!(got, vec!["Service/clusterip", "Service/externalname"]);
    let got = check_findings(
        "exposed-services",
        &objs,
        r#"#{forbiddenServiceTypes: ["nodeport"]}"#,
    );
    assert_eq!(got, vec![
        "Service/nodeport",
        "Service/nodeport-lower",
        "Service/nodeport-other",
    ]);
    let got = check_findings("exposed-services", &objs, "#{forbiddenServiceTypes: []}");
    assert!(got.is_empty(), "liste vide : {got:?}");
}

#[test]
fn the_network_batch_does_not_raise_on_unexpected_shapes() {
    // Formes inattendues (spec, sélecteur, type, ports, gabarit, jobTemplate) : aucune levée,
    // aucun constat du thème network.
    let functions = [
        "dangling_service",
        "exposed_services",
        "invalid_target_ports",
        "dangling_ingress",
        "dangling_networkpolicy",
        "dangling_networkpolicypeer_podselector",
        "non_isolated_pod",
    ];
    let odd = vec![
        json!({ "apiVersion": "v1", "kind": "Service", "metadata": { "name": "a", "namespace": "app" }, "spec": "oops" }),
        json!({ "apiVersion": "v1", "kind": "Service", "metadata": { "name": "b", "namespace": "app" }, "spec": { "selector": "x", "type": 4 } }),
        json!({ "apiVersion": "v1", "kind": "Service", "metadata": { "name": "c", "namespace": "app" }, "spec": { "selector": { "app": 5 }, "ports": "x" } }),
        json!({ "apiVersion": "v1", "kind": "Service", "metadata": { "name": "d", "namespace": "app" }, "spec": { "ports": [ "x", { "port": "80", "targetPort": [1] } ] } }),
        json!({ "apiVersion": "v1", "kind": "Service", "metadata": "x", "spec": 7 }),
        json!({ "apiVersion": "v1", "kind": "Pod", "metadata": { "name": "e", "namespace": "app", "labels": { "app": 3 } }, "spec": 4 }),
        json!({ "apiVersion": "apps/v1", "kind": "Deployment", "metadata": { "name": "f", "namespace": "app" }, "spec": { "template": "y" } }),
        json!({ "apiVersion": "batch/v1", "kind": "CronJob", "metadata": { "name": "g", "namespace": "app" }, "spec": { "jobTemplate": "z" } }),
        json!({ "apiVersion": "networking.k8s.io/v1", "kind": "Ingress", "metadata": { "name": "h", "namespace": "app" }, "spec": { "rules": "oops", "defaultBackend": 3 } }),
        json!({ "apiVersion": "networking.k8s.io/v1", "kind": "Ingress", "metadata": { "name": "i", "namespace": "app" }, "spec": { "rules": [ { "http": { "paths": [ { "backend": { "service": "junk" } } ] } } ] } }),
        json!({ "apiVersion": "networking.k8s.io/v1", "kind": "Ingress", "metadata": { "name": "j", "namespace": "app" }, "spec": "x" }),
        json!({ "apiVersion": "v1", "kind": "Pod", "metadata": { "name": "k", "namespace": "app" }, "spec": { "containers": [ { "name": "c", "ports": "oops" } ] } }),
        json!({ "apiVersion": "v1", "kind": "Pod", "metadata": { "name": "l", "namespace": "app" }, "spec": { "containers": [ { "name": "c", "ports": [ { "name": "Bad_Name", "containerPort": "80" } ] } ] } }),
        json!({ "apiVersion": "v1", "kind": "Service", "metadata": { "name": "m", "namespace": "app" }, "spec": { "ports": [ { "port": 80, "targetPort": { "x": 1 } } ] } }),
        json!({ "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy", "metadata": { "name": "n", "namespace": "app" }, "spec": "x" }),
        json!({ "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy", "metadata": { "name": "o", "namespace": "app" }, "spec": { "podSelector": { "matchExpressions": "x" }, "ingress": [ { "from": [ { "podSelector": { "matchLabels": { "app": 5 } } } ] } ] } }),
        json!({ "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy", "metadata": { "name": "q", "namespace": "app" }, "spec": { "podSelector": {}, "egress": [ { "to": [ { "namespaceSelector": 3 } ] } ] } }),
        json!({ "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy", "metadata": { "name": "r", "namespace": "app" }, "spec": { "podSelector": { "matchExpressions": [ { "key": "app", "operator": "In", "values": "web" } ] } } }),
    ];
    let mut rhai = lib_script();
    set_objs(&mut rhai, &Value::Array(odd));
    let only = functions
        .iter()
        .map(|f| format!("\"{}\"", f.replace('_', "-")))
        .collect::<Vec<_>>()
        .join(", ");
    let found = eval_json(
        &mut rhai,
        &format!(
            r#"import "render_lint" as render_lint; render_lint::all(objs, #{{}}, #{{only: [{only}], params: #{{"exposed-services": #{{forbiddenServiceTypes: "NodePort"}}}}}})"#
        ),
    );
    assert_eq!(found, json!([]), "aucun constat sur des formes inattendues");
}

#[test]
fn ingress_without_backend_service_is_dangling() {
    // Scénario de render_lint_network.sdd : le backend nomme un Service absent de la liste.
    let ingress = json!({
        "apiVersion": "networking.k8s.io/v1", "kind": "Ingress",
        "metadata": { "name": "web", "namespace": "app" },
        "spec": { "rules": [ { "host": "a.example.test", "http": { "paths": [ {
            "path": "/", "pathType": "Prefix",
            "backend": { "service": { "name": "web", "port": { "number": 80 } } }
        } ] } } ] }
    });
    let got = check_findings("dangling-ingress", &[ingress], "#{}");
    assert_eq!(got, vec!["Ingress/web"]);
}

#[test]
fn a_named_target_port_missing_from_the_containers_is_not_reported_like_kube_linter() {
    // Le scénario « named target port must exist » de render_lint_network.sdd ne décrit pas
    // l'outil : kube-linter v0.8.3 ne vérifie pas qu'un targetPort nommé existe dans les
    // conteneurs (mesuré : targetPort "nope" absent du gabarit, aucun constat). Seul le format IANA
    // du nom est examiné. Ce test fige le comportement mesuré.
    let dep = deployment_with_pod_spec(
        "web",
        &json!({ "containers": [ { "name": "web", "image": "registry.example.test/web:1.4.2",
            "ports": [ { "name": "http", "containerPort": 8080 } ] } ] }),
    );
    let svc = json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": "web", "namespace": "app" },
        "spec": { "selector": { "app": "web" }, "ports": [ { "port": 80, "targetPort": "nope" } ] }
    });
    let got = check_findings("invalid-target-ports", &[dep, svc], "#{}");
    assert!(got.is_empty(), "targetPort absent des conteneurs : {got:?}");
}

#[test]
fn invalid_target_ports_resolves_like_kube_linter() {
    // Sortie de kube-linter v0.8.3 sur la fixture invalid-target-ports : un constat par règle IANA
    // violée (un nom peut en violer plusieurs, ex. HTTP : caractères et lettre), par port et par
    // conteneur (init inclus) ; genres apps (toutes versions), batch, noyau RC et Pod ; le groupe
    // extensions et les groupes tiers sont ignorés ; un nom de port vide n'est pas examiné ; un
    // Service mal formé est ignoré.
    let objs = fixture_objects("invalid-target-ports");
    let got = check_findings("invalid-target-ports", &objs, "#{}");
    let want = vec![
        "CronJob/cron-bad",
        "DaemonSet/ds-bad",
        "Deployment/dep-bad",
        "Deployment/dep-beta-bad",
        "Job/job-bad",
        "Job/job-bad",
        "Pod/pod-bad",
        "Pod/pod-multi",
        "Pod/pod-multi",
        "Pod/pod-multi",
        "Pod/pod-multi",
        "ReplicaSet/rs-bad",
        "ReplicationController/rc-bad",
        "Service/t-digits",
        "Service/t-double",
        "Service/t-empty",
        "Service/t-empty",
        "Service/t-long",
        "Service/t-space",
        "Service/t-trail",
        "Service/t-upper",
        "Service/t-upper",
        "StatefulSet/sts-bad",
    ];
    assert_eq!(got, want);
}

#[test]
fn dangling_ingress_resolves_like_kube_linter() {
    // Sortie de kube-linter v0.8.3 sur la fixture dangling-ingress : seul networking.k8s.io/v1 est
    // examiné ; un constat par backend non résolu (defaultBackend et chaque chemin) ; le namespace
    // est comparé ; le port est comparé au nom (prioritaire) ou au numéro, un port absent valant 0 ;
    // un Service mal formé ou d'un groupe tiers n'est pas une cible ; un Ingress mal formé est
    // ignoré en entier.
    let objs = fixture_objects("dangling-ingress");
    let got = check_findings("dangling-ingress", &objs, "#{}");
    let want = vec![
        "Ingress/ko-default",
        "Ingress/ko-default-and-rule",
        "Ingress/ko-default-and-rule",
        "Ingress/ko-empty-name-named",
        "Ingress/ko-empty-svc-name",
        "Ingress/ko-malformed-svc",
        "Ingress/ko-missing",
        "Ingress/ko-multi",
        "Ingress/ko-multi",
        "Ingress/ko-no-port-field",
        "Ingress/ko-noport-name",
        "Ingress/ko-noport-number",
        "Ingress/ko-number-null",
        "Ingress/ko-number-zero",
        "Ingress/ko-other-ns",
        "Ingress/ko-port-name",
        "Ingress/ko-port-number",
        "Ingress/ko-target-port-number",
        "Ingress/ko-tiers-svc",
    ];
    assert_eq!(got, want);
}

#[test]
fn dangling_networkpolicy_resolves_like_kube_linter() {
    // Sortie de kube-linter v0.8.3 sur la fixture dangling-networkpolicy : seules les NetworkPolicy
    // networking.k8s.io/v1 sont examinées ; les cibles sont celles de dangling-service (Deployment
    // apps et extensions, ReplicaSet et DaemonSet apps et extensions, StatefulSet apps, Job, CronJob,
    // RC et Pod du noyau), le namespace est comparé, un sélecteur vide atteint toute cible de son
    // namespace (labels absents compris) ; les opérateurs In, NotIn, Exists et DoesNotExist suivent
    // les règles mesurées, un opérateur inconnu ne satisfait rien.
    let objs = fixture_objects("dangling-networkpolicy");
    let got = check_findings("dangling-networkpolicy", &objs, "#{}");
    let want = vec![
        "NetworkPolicy/ko-apps-job-group",
        "NetworkPolicy/ko-core-dep",
        "NetworkPolicy/ko-exists-values",
        "NetworkPolicy/ko-expr-nomatch",
        "NetworkPolicy/ko-ext-sts-group",
        "NetworkPolicy/ko-in-novalues",
        "NetworkPolicy/ko-labels-and-expression",
        "NetworkPolicy/ko-metadata-only",
        "NetworkPolicy/ko-nomatch",
        "NetworkPolicy/ko-notin-novalues",
        "NetworkPolicy/ko-other-nomatch",
        "NetworkPolicy/ko-other-ns-target",
        "NetworkPolicy/ko-solo-empty",
        "NetworkPolicy/ko-superset",
        "NetworkPolicy/ko-tiers-group",
        "NetworkPolicy/ko-tiers-pod",
        "NetworkPolicy/ko-two-expressions",
        "NetworkPolicy/ko-unknown-operator",
    ];
    assert_eq!(got, want);
}

#[test]
fn an_empty_selector_reaches_only_targets_of_its_own_namespace() {
    // Mesuré : une NetworkPolicy à sélecteur vide dans un namespace sans cible est dangling, même si
    // d'autres namespaces contiennent des cibles.
    let pod = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "p", "namespace": "app" },
        "spec": { "containers": [ web_container() ] }
    });
    let np = json!({
        "apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy",
        "metadata": { "name": "np", "namespace": "solo" },
        "spec": { "podSelector": {} }
    });
    let got = check_findings("dangling-networkpolicy", &[pod, np], "#{}");
    assert_eq!(got, vec!["NetworkPolicy/np"]);
}

#[test]
fn dangling_networkpolicypeer_podselector_resolves_like_kube_linter() {
    // Sortie de kube-linter v0.8.3 sur la fixture dangling-networkpolicypeer-podselector : un constat
    // par pair fautive, ingress (from) et egress (to) ; une pair avec namespaceSelector (même vide),
    // une pair ipBlock, une pair sans podSelector ou une pair sans from/to ne sont pas examinées ;
    // un podSelector vide atteint toute cible du namespace de la policy.
    let objs = fixture_objects("dangling-networkpolicypeer-podselector");
    let got = check_findings("dangling-networkpolicypeer-podselector", &objs, "#{}");
    let want = vec![
        "NetworkPolicy/ko-from-nomatch",
        "NetworkPolicy/ko-metadata-only",
        "NetworkPolicy/ko-operator",
        "NetworkPolicy/ko-other-namespace-target",
        "NetworkPolicy/ko-solo-empty-peer",
        "NetworkPolicy/ko-tiers-group",
        "NetworkPolicy/ko-two-ingress-peers",
        "NetworkPolicy/ko-two-ingress-peers",
        "NetworkPolicy/ko-two-ingress-peers",
    ];
    assert_eq!(got, want);
}

#[test]
fn non_isolated_pod_resolves_like_kube_linter() {
    // Sortie de kube-linter v0.8.3 sur la fixture non-isolated-pod : les genres examinés sont ceux de
    // pod_templates (apps toutes versions hors extensions, batch, RC et Pod du noyau) ; le groupe
    // extensions n'est PAS examiné ici, à la différence de dangling-service. Un workload est isolé par
    // une NetworkPolicy networking.k8s.io/v1 de son namespace dont le sélecteur atteint ses labels de
    // gabarit ; un sélecteur vide isole tout le namespace, y compris un pod sans labels.
    let objs = fixture_objects("non-isolated-pod");
    let got = check_findings("non-isolated-pod", &objs, "#{}");
    let want = vec![
        "CronJob/iso-cron",
        "CronJob/open-cron",
        "CronJob/open-cron-beta",
        "DaemonSet/open-ds",
        "Deployment/iso-meta",
        "Deployment/open-beta",
        "Deployment/open-dep",
        "Job/open-job",
        "Pod/open-bare",
        "Pod/open-pod",
        "ReplicaSet/open-rs",
        "ReplicationController/open-rc",
        "StatefulSet/open-sts",
    ];
    assert_eq!(got, want);
}

#[test]
fn a_malformed_pod_is_not_a_selector_target_like_kube_linter() {
    // Mesuré sur kube-linter v0.8.3 : un Pod dont les ports de conteneur sont mal typés est ignoré
    // en entier ; un Service qui le vise est donc dangling.
    let pod = json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": { "name": "p", "namespace": "app", "labels": { "app": "x" } },
        "spec": { "containers": [ { "name": "web", "image": "registry.example.test/web:1.4.2", "ports": "oops" } ] }
    });
    let svc = json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": "s", "namespace": "app" },
        "spec": { "selector": { "app": "x" }, "ports": [ { "port": 80 } ] }
    });
    let got = check_findings("dangling-service", &[pod, svc], "#{}");
    assert_eq!(got, vec!["Service/s"]);
}
