use common::{
    httphandler::http_rhai_register,
    k8smock::{K8sJukeBoxMock, k8smock_rhai_register, oci_mock_rhai_register},
    rhaihandler::Script,
};
use rhai::Dynamic;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const INDEX_ONE_ENTRY: &str = "packages:\n  - category: db\n    name: pg\n    file: db_pg.yaml\n";

const INDEX_TWO_ENTRIES: &str = "packages:\n  - category: db\n    name: pg\n    file: db_pg.yaml\n  - category: monitoring\n    name: prom\n    file: monitoring_prom.yaml\n";

const DB_PG_PACKAGES: &str = "- registry: docker.io\n  image: myrepo/pg\n  tag: \"1.0.0\"\n  metadata:\n    name: pg\n    category: db\n    description: PostgreSQL\n    type: service\n    features: []\n  requirements: []\n";

fn make_http_scan_script() -> (Script, Arc<Mutex<Vec<Dynamic>>>) {
    common::set_client_name(common::get_client_name);
    let base = env!("CARGO_MANIFEST_DIR");
    let created = Arc::new(Mutex::new(vec![]));
    let mut script = Script::new_core(vec![
        format!("{base}/scripts/boxes"),
        format!("{base}/scripts/lib"),
    ]);
    oci_mock_rhai_register(&mut script.engine);
    http_rhai_register(&mut script.engine);
    k8smock_rhai_register(&mut script.engine, vec![], created.clone());
    (script, created)
}

fn build_http_jukebox_mock(url: &str) -> K8sJukeBoxMock {
    let json = serde_json::json!({
        "kind": "JukeBox",
        "metadata": { "name": "test-http-box" },
        "spec": {
            "source": { "http": { "url": url } },
            "maturity": "stable",
            "schedule": "0 * * * *"
        }
    });
    K8sJukeBoxMock {
        obj: serde_json::from_str(&serde_json::to_string(&json).unwrap()).unwrap(),
    }
}

fn run_http_scan(script: &mut Script, url: &str, filter: Option<&str>) -> common::Result<Dynamic> {
    let base = env!("CARGO_MANIFEST_DIR");
    script.ctx.set_value("box", build_http_jukebox_mock(url));
    let args = serde_json::json!({
        "namespace": "vynil-system",
        "filter": filter,
    });
    script.set_dynamic("args", &args);
    script.run_file(&PathBuf::from(format!("{base}/scripts/boxes/scan.rhai")))
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_http_source_finds_package() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/index.yaml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(INDEX_ONE_ENTRY))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/db_pg.yaml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(DB_PG_PACKAGES))
        .mount(&server)
        .await;

    let (mut script, _) = make_http_scan_script();
    let result = run_http_scan(&mut script, &server.uri(), None);
    assert!(
        result.is_ok(),
        "scan.rhai with Http source should succeed: {:?}",
        result.err()
    );

    let pkg_count = script.eval("box.status.packages.len()").unwrap();
    assert_eq!(
        pkg_count.as_int().unwrap(),
        1,
        "expected 1 package found via Http source"
    );
    let pkg_tag = script.eval(r"box.status.packages[0].tag").unwrap();
    assert_eq!(pkg_tag.into_string().unwrap(), "1.0.0", "expected tag 1.0.0");
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_http_source_filter_skips_other_categories() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/index.yaml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(INDEX_TWO_ENTRIES))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/db_pg.yaml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(DB_PG_PACKAGES))
        .mount(&server)
        .await;
    // monitoring_prom.yaml intentionally NOT mocked — fetching it would return 404 and fail the script

    let (mut script, _) = make_http_scan_script();
    let result = run_http_scan(&mut script, &server.uri(), Some("db"));
    assert!(
        result.is_ok(),
        "scan.rhai with Http source + filter 'db' should succeed (monitoring entry skipped): {:?}",
        result.err()
    );

    let pkg_count = script.eval("box.status.packages.len()").unwrap();
    assert_eq!(
        pkg_count.as_int().unwrap(),
        1,
        "filter 'db' should produce exactly 1 package"
    );
}
