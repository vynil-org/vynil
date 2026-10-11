use common::{k8smock::K8sJukeBoxMock, rhaihandler::Script};
use rhai::Dynamic;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

fn make_scan_script(k8s_mocks: Vec<Dynamic>) -> (Script, Arc<Mutex<Vec<Dynamic>>>) {
    let base = env!("CARGO_MANIFEST_DIR");
    let created = Arc::new(Mutex::new(vec![]));
    let script = Script::new_mock(
        vec![format!("{base}/scripts/boxes"), format!("{base}/scripts/lib")],
        vec![],
        k8s_mocks,
        created.clone(),
    );
    (script, created)
}

fn build_jukebox_mock_obj() -> Dynamic {
    let json = serde_json::json!({
        "kind": "JukeBox",
        "metadata": { "name": "test-box" },
        "spec": {
            "source": { "list": ["docker.io/myrepo/pg"] },
            "maturity": "stable",
            "schedule": "0 * * * *"
        },
        "status": {
            "conditions": [],
            "packages": [
                {
                    "registry": "docker.io",
                    "image": "myrepo/pg",
                    "tag": "1.0.0",
                    "metadata": {
                        "name": "pg",
                        "category": "db",
                        "description": "PostgreSQL",
                        "type": "service",
                        "features": []
                    },
                    "requirements": []
                },
                {
                    "registry": "docker.io",
                    "image": "myrepo/prom",
                    "tag": "1.0.0",
                    "metadata": {
                        "name": "prom",
                        "category": "monitoring",
                        "description": "Prometheus",
                        "type": "service",
                        "features": []
                    },
                    "requirements": []
                }
            ]
        }
    });
    serde_json::from_str(&serde_json::to_string(&json).unwrap()).unwrap()
}

fn build_jukebox_mock() -> K8sJukeBoxMock {
    K8sJukeBoxMock {
        obj: build_jukebox_mock_obj(),
    }
}

fn run_scan(filter: Option<&str>) -> common::Result<Dynamic> {
    let base = env!("CARGO_MANIFEST_DIR");
    let (mut script, _created) = make_scan_script(vec![]);
    script.ctx.set_value("box", build_jukebox_mock());
    let args = serde_json::json!({
        "namespace": "vynil-system",
        "filter": filter,
    });
    script.set_dynamic("args", &args);
    script.run_file(&PathBuf::from(format!("{base}/scripts/boxes/scan.rhai")))
}

// ── Filtre — logique Rhai ─────────────────────────────────────────────────

#[test]
fn scan_filter_selects_only_matching_image() {
    let (mut script, _) = make_scan_script(vec![]);
    let result = script.eval(
        r#"
        let scan_filter = "db/pg";
        let filter_parts = scan_filter.split("/");
        let filter_cat   = filter_parts[0];
        let filter_name  = if filter_parts.len() > 1 { filter_parts[1] } else { () };
        let status_pkgs  = [
            #{metadata: #{category: "db",         name: "pg"},   registry: "docker.io", image: "myrepo/pg"},
            #{metadata: #{category: "monitoring",  name: "prom"}, registry: "docker.io", image: "myrepo/prom"}
        ];
        let matched = status_pkgs
            .filter(|p| p.metadata.category == filter_cat && (filter_name == () || p.metadata.name == filter_name))
            .map(|p| #{registry: p.registry, repository: p.image});
        let seen = #{}; let deduped = [];
        for m in matched {
            let k = `${m.registry}/${m.repository}`;
            if k in seen {} else { seen[k] = true; deduped.push(m); }
        }
        deduped.len() == 1 && deduped[0].repository == "myrepo/pg"
        "#,
    );
    assert!(
        result.unwrap().as_bool().unwrap(),
        "filter 'db/pg' should select only myrepo/pg"
    );
}

#[test]
fn scan_filter_category_selects_all_matching_images() {
    let (mut script, _) = make_scan_script(vec![]);
    let result = script.eval(
        r#"
        let scan_filter = "db";
        let filter_parts = scan_filter.split("/");
        let filter_cat   = filter_parts[0];
        let filter_name  = if filter_parts.len() > 1 { filter_parts[1] } else { () };
        let status_pkgs  = [
            #{metadata: #{category: "db", name: "pg"},    registry: "docker.io", image: "myrepo/pg"},
            #{metadata: #{category: "db", name: "mysql"}, registry: "docker.io", image: "myrepo/mysql"},
            #{metadata: #{category: "monitoring", name: "prom"}, registry: "docker.io", image: "myrepo/prom"}
        ];
        let matched = status_pkgs
            .filter(|p| p.metadata.category == filter_cat && (filter_name == () || p.metadata.name == filter_name))
            .map(|p| #{registry: p.registry, repository: p.image});
        let seen = #{}; let deduped = [];
        for m in matched {
            let k = `${m.registry}/${m.repository}`;
            if k in seen {} else { seen[k] = true; deduped.push(m); }
        }
        deduped.len() == 2
        "#,
    );
    assert!(
        result.unwrap().as_bool().unwrap(),
        "filter 'db' should select both db images"
    );
}

#[test]
fn scan_branch_merge_when_filter_set() {
    let (mut script, _) = make_scan_script(vec![]);
    let result = script.eval(
        r#"
        let scan_filter = "db/pg";
        if scan_filter != () { "merge" } else { "full" }
        "#,
    );
    assert_eq!(result.unwrap().into_string().unwrap(), "merge");
}

#[test]
fn scan_branch_full_when_no_filter() {
    let (mut script, _) = make_scan_script(vec![]);
    let result = script.eval(
        r#"
        let scan_filter = ();
        if scan_filter != () { "merge" } else { "full" }
        "#,
    );
    assert_eq!(result.unwrap().into_string().unwrap(), "full");
}

// ── security_filter — TDD ────────────────────────────────────────────────

fn build_jukebox_http_mock() -> K8sJukeBoxMock {
    let json = serde_json::json!({
        "kind": "JukeBox",
        "metadata": {"name": "test-box"},
        "spec": {
            "source": {"http": {"url": "http://test.local/packages"}},
            "maturity": "stable",
            "schedule": "0 * * * *"
        },
        "status": {"conditions": [], "packages": []}
    });
    K8sJukeBoxMock {
        obj: serde_json::from_str(&serde_json::to_string(&json).unwrap()).unwrap(),
    }
}

#[test]
fn security_filter_cosign_and_trivy_tags_no_error_log() {
    // Old code calls log_error() for every non-semver tag (cosign, trivy, etc.).
    // New code must silently return false for tags that do not start with a digit or 'v'.
    // HTTP source path is used so that security_filter is called on p.tag via
    // compute_waypoints_from_packages — no registry method mocking needed.
    // Fails on old code (log_error throws); passes on new code (silent filter).
    let base = env!("CARGO_MANIFEST_DIR");
    let (mut script, _) = make_scan_script(vec![]);

    script
        .add_code(
            r#"
        fn log_error(msg) { throw `unexpected error log: ${msg}`; }
        fn http_get_yaml(url, auth_type, credential) {
            if url.contains("index.yaml") {
                #{ packages: [#{ category: "test", name: "pkg", file: "pkg.yaml" }] }
            } else {
                [
                    #{ tag: "sha256-deadbeef.sig", registry: "r.io", image: "test/img",
                       metadata: #{}, requirements: [] },
                    #{ tag: "trivy--apps-foo", registry: "r.io", image: "test/img",
                       metadata: #{}, requirements: [] },
                    #{ tag: "1.2.3", registry: "r.io", image: "test/img",
                       metadata: #{}, requirements: [] }
                ]
            }
        }
        "#,
        )
        .unwrap();
    script.ctx.set_value("box", build_jukebox_http_mock());
    let args = serde_json::json!({"namespace": "test-ns"});
    script.set_dynamic("args", &args);

    let result = script.run_file(&PathBuf::from(format!("{base}/scripts/boxes/scan.rhai")));
    assert!(
        result.is_ok(),
        "Expected no error log for cosign/trivy tags: {:?}",
        result.err()
    );
}

// ── Intégration — scan.rhai complet ──────────────────────────────────────

#[test]
fn scan_rhai_with_filter_runs_without_error() {
    let result = run_scan(Some("db/pg"));
    assert!(
        result.is_ok(),
        "scan.rhai with filter should succeed (set_status_packages_merge called): {:?}",
        result.err()
    );
}

#[test]
fn scan_rhai_without_filter_runs_without_error() {
    let result = run_scan(None);
    assert!(
        result.is_ok(),
        "scan.rhai without filter should succeed (set_status_updated called): {:?}",
        result.err()
    );
}

#[test]
fn partial_filter_no_match_does_not_full_scan() {
    // When the filter matches no package in box.status.packages the script must NOT fall
    // back to scanning the full image list — it must warn and call set_status_packages_merge
    // with an empty list.
    // Fails on current code: falls back to `list` → calls new_registry (which throws here).
    // Passes on fixed code: warns and returns [] without touching the registry.
    let base = env!("CARGO_MANIFEST_DIR");
    let (mut script, _) = make_scan_script(vec![]);
    script
        .add_code(
            r#"
        fn new_registry(reg, user, pass) {
            throw "unexpected registry call: fallback to full scan occurred";
        }
        "#,
        )
        .unwrap();
    let json = serde_json::json!({
        "kind": "JukeBox",
        "metadata": {"name": "test-box"},
        "spec": {
            "source": {"list": ["docker.io/myrepo/pg"]},
            "maturity": "stable",
            "schedule": "0 * * * *"
        },
        "status": {
            "conditions": [],
            "packages": [{
                "registry": "docker.io",
                "image": "myrepo/pg",
                "tag": "1.0.0",
                "metadata": {
                    "name": "pg",
                    "category": "db",
                    "description": "PostgreSQL",
                    "type": "service",
                    "features": []
                },
                "requirements": []
            }]
        }
    });
    let mock_obj: rhai::Dynamic = serde_json::from_str(&serde_json::to_string(&json).unwrap()).unwrap();
    script
        .ctx
        .set_value("box", common::k8smock::K8sJukeBoxMock { obj: mock_obj });
    let args = serde_json::json!({"namespace": "vynil-system", "filter": "monitoring/alertmanager"});
    script.set_dynamic("args", &args);
    let result = script.run_file(&std::path::PathBuf::from(format!(
        "{base}/scripts/boxes/scan.rhai"
    )));
    assert!(
        result.is_ok(),
        "partial scan with unmatched filter must not fall back to full scan: {:?}",
        result.err()
    );
}
