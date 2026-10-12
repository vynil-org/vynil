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
