use common::rhaihandler::Script;

fn make_tenant_script() -> Script {
    let base = env!("CARGO_MANIFEST_DIR");
    Script::new_mock(
        vec![format!("{base}/scripts/lib"), format!("{base}/scripts/tenant")],
        vec![],
        vec![],
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
    )
}

fn context(namespace: &str, redis_list: &[&str]) -> String {
    let list = redis_list
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("#{{ namespace: \"{namespace}\", redis_list: [{list}] }}")
}

// ─── backup_prepare_redis ────────────────────────────────────────────────────

#[test]
fn backup_redis_empty_list_completes() {
    // Empty redis_list: no shell calls, should complete without errors.
    let mut rhai = make_tenant_script();
    let ctx = context("test-ns", &[]);
    let result = rhai.eval(&format!(
        r#"import "backup_prepare_redis" as bk; bk::run({ctx});"#
    ));
    assert!(result.is_ok(), "Expected success, got: {:?}", result.err());
}

#[test]
fn backup_redis_nominal() {
    // All kubectl commands succeed; expect the script to complete without throwing.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"      { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) { "my-dragonfly-0" }
        fn shell_run(cmd)    { 0 }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(
        r#"import "backup_prepare_redis" as bk; bk::run({ctx});"#
    ));
    assert!(result.is_ok(), "Expected success, got: {:?}", result.err());
}

#[test]
fn backup_redis_pod_not_found_throws() {
    // kubectl get pods returns empty string → script must throw.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) { "" }
        fn shell_run(cmd)    { 0 }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(
        r#"import "backup_prepare_redis" as bk; bk::run({ctx});"#
    ));
    assert!(result.is_err(), "Expected throw when no pod found");
    let msg = format!("{:?}", result.err());
    assert!(
        msg.contains("No running pod found"),
        "Unexpected error message: {msg}"
    );
}

#[test]
fn backup_redis_save_failure_throws() {
    // SAVE command returns non-zero → script must throw.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) { "my-dragonfly-0" }
        fn shell_run(cmd) {
            if cmd.contains("SAVE") { 1 }
            else { 0 }
        }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(
        r#"import "backup_prepare_redis" as bk; bk::run({ctx});"#
    ));
    assert!(result.is_err(), "Expected throw when SAVE fails");
    let msg = format!("{:?}", result.err());
    assert!(
        msg.contains("redis SAVE failed"),
        "Unexpected error message: {msg}"
    );
}

#[test]
fn backup_redis_cp_failure_throws() {
    // kubectl cp returns non-zero → script must throw.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) { "my-dragonfly-0" }
        fn shell_run(cmd) {
            if cmd.contains(" cp ") { 1 }
            else { 0 }
        }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(
        r#"import "backup_prepare_redis" as bk; bk::run({ctx});"#
    ));
    assert!(result.is_err(), "Expected throw when kubectl cp fails");
    let msg = format!("{:?}", result.err());
    assert!(
        msg.contains("kubectl cp failed"),
        "Unexpected error message: {msg}"
    );
}

// ─── restore_redis ────────────────────────────────────────────────────────────

#[test]
fn restore_redis_empty_list_completes() {
    // Empty redis_list: no shell calls, should complete without errors.
    let mut rhai = make_tenant_script();
    let ctx = context("test-ns", &[]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(result.is_ok(), "Expected success, got: {:?}", result.err());
}

#[test]
fn restore_redis_statefulset_single_replica() {
    // StatefulSet with 1 replica: no scale needed, full restore flow completes.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) {
            if cmd.contains("get pods")    { "my-dragonfly-0" }
            else if cmd.contains("replicas") { "1" }
            else { "" }
        }
        fn shell_run(cmd) { 0 }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(result.is_ok(), "Expected success, got: {:?}", result.err());
}

#[test]
fn restore_redis_statefulset_multiple_replicas() {
    // StatefulSet with 2 replicas: scale down to 1 then scale back to 2.
    // All commands succeed; script must complete without error.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) {
            if cmd.contains("get pods")      { "my-dragonfly-0" }
            else if cmd.contains("replicas") { "2" }
            else { "" }
        }
        fn shell_run(cmd) { 0 }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(result.is_ok(), "Expected success, got: {:?}", result.err());
}

#[test]
fn restore_redis_deployment_detected() {
    // When kubectl get statefulset returns non-zero, resource type is "deployment".
    // Script must still complete successfully with all mock commands succeeding.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "myredis_sts"           { "my-redis" }
            else if name == "myredis_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) {
            if cmd.contains("get pods")      { "my-redis-0" }
            else if cmd.contains("replicas") { "1" }
            else { "" }
        }
        fn shell_run(cmd) {
            // Simulate statefulset not found → use deployment
            if cmd.contains("get statefulset") { 1 }
            else { 0 }
        }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["myredis"]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(result.is_ok(), "Expected success, got: {:?}", result.err());
}

#[test]
fn restore_redis_pod_not_found_throws() {
    // kubectl get pods returns empty string → script must throw.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) {
            if cmd.contains("replicas") { "1" }
            else { "" }
        }
        fn shell_run(cmd) { 0 }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(result.is_err(), "Expected throw when no pod found");
    let msg = format!("{:?}", result.err());
    assert!(msg.contains("No pod found"), "Unexpected error message: {msg}");
}

#[test]
fn restore_redis_cp_failure_throws() {
    // kubectl cp fails → script must throw.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) {
            if cmd.contains("get pods")      { "my-dragonfly-0" }
            else if cmd.contains("replicas") { "1" }
            else { "" }
        }
        fn shell_run(cmd) {
            if cmd.contains(" cp ") { 1 }
            else { 0 }
        }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(result.is_err(), "Expected throw when kubectl cp fails");
    let msg = format!("{:?}", result.err());
    assert!(
        msg.contains("kubectl cp failed"),
        "Unexpected error message: {msg}"
    );
}

#[test]
fn restore_redis_scale_down_required_for_multiple_replicas() {
    // Old code had no scale logic: would succeed even when scale fails.
    // New code scales before restore → propagates the failure.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) {
            if cmd.contains("get pods")      { "my-dragonfly-0" }
            else if cmd.contains("replicas") { "2" }
            else { "" }
        }
        fn shell_run(cmd) {
            if cmd.contains("scale") { 1 }
            else { 0 }
        }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(result.is_err(), "Expected throw when scale down fails");
    let msg = format!("{:?}", result.err());
    assert!(
        msg.contains("scale down failed"),
        "Unexpected error message: {msg}"
    );
}

#[test]
fn restore_redis_resource_type_detection_uses_deployment() {
    // Old code hardcoded rollout status to statefulset even for Dragonfly (Deployment).
    // New code detects resource type: if get statefulset fails → use deployment.
    // Test: make rollout status statefulset fail → old code would throw, new code uses deployment.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "myredis_sts"           { "my-redis" }
            else if name == "myredis_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) {
            if cmd.contains("get pods")      { "my-redis-0" }
            else if cmd.contains("replicas") { "1" }
            else { "" }
        }
        fn shell_run(cmd) {
            // "get statefulset" not found → resource is deployment
            // "statefulset/" anywhere else → old code would use it for rollout → fails
            if cmd.contains("get statefulset") { 1 }
            else if cmd.contains("statefulset/") { 1 }
            else { 0 }
        }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["myredis"]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(
        result.is_ok(),
        "Expected success when resource type is detected as deployment, got: {:?}",
        result.err()
    );
}

#[test]
fn restore_redis_rollout_restart_failure_throws() {
    // rollout restart fails → script must throw.
    let mut rhai = make_tenant_script();
    rhai.add_code(
        r#"
        fn get_env(name) {
            if name == "dragonfly_sts"           { "my-dragonfly" }
            else if name == "dragonfly_password" { "secret" }
            else { "" }
        }
        fn shell_output(cmd) {
            if cmd.contains("get pods")      { "my-dragonfly-0" }
            else if cmd.contains("replicas") { "1" }
            else { "" }
        }
        fn shell_run(cmd) {
            if cmd.contains("rollout restart") { 1 }
            else { 0 }
        }
        "#,
    )
    .unwrap();
    let ctx = context("test-ns", &["dragonfly"]);
    let result = rhai.eval(&format!(r#"import "restore_redis" as rs; rs::run({ctx});"#));
    assert!(result.is_err(), "Expected throw when rollout restart fails");
    let msg = format!("{:?}", result.err());
    assert!(
        msg.contains("rollout restart failed"),
        "Unexpected error message: {msg}"
    );
}
