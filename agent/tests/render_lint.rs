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
    assert_eq!(rules(&excluded), vec!["no-read-only-root-fs".to_string()]);
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
