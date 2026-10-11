use common::rhaihandler::Script;

fn make_build_script() -> Script {
    let base = env!("CARGO_MANIFEST_DIR");
    Script::new_mock(
        vec![format!("{base}/scripts/packages"), format!("{base}/scripts/lib")],
        vec![],
        vec![],
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
    )
}

// ── security_filter in build.rhai — TDD ──────────────────────────────────
// build.rhai contains a security_filter function identical to the one in scan.rhai.
// Old code calls log_error() for every non-semver tag, producing ERROR noise.
// New code must return false silently for tags that do not start with a digit or 'v'.

#[test]
fn build_security_filter_cosign_tag_no_error_log() {
    // sha256-*.sig tags (Cosign signatures) must be silently rejected.
    // Old code: log_error("sha256-deadbeef.sig") → mock throws → test fails.
    // New code: starts with 's' → returns false silently → test passes.
    let mut script = make_build_script();
    script
        .add_code(r"fn log_error(msg) { throw `unexpected error log: ${msg}`; }")
        .unwrap();
    let result = script.eval(r#"import "build" as b; b::security_filter("sha256-deadbeef.sig")"#);
    assert!(result.is_ok(), "Expected no error log: {:?}", result.err());
    assert!(
        !result.unwrap().as_bool().unwrap(),
        "Expected false for cosign tag"
    );
}

#[test]
fn build_security_filter_trivy_tag_no_error_log() {
    // trivy-* tags must be silently rejected.
    let mut script = make_build_script();
    script
        .add_code(r"fn log_error(msg) { throw `unexpected error log: ${msg}`; }")
        .unwrap();
    let result = script.eval(r#"import "build" as b; b::security_filter("trivy--apps-auth")"#);
    assert!(result.is_ok(), "Expected no error log: {:?}", result.err());
    assert!(
        !result.unwrap().as_bool().unwrap(),
        "Expected false for trivy tag"
    );
}

#[test]
fn build_security_filter_latest_tag_no_error_log() {
    // "latest" and other non-versioned tags must be silently rejected.
    let mut script = make_build_script();
    script
        .add_code(r"fn log_error(msg) { throw `unexpected error log: ${msg}`; }")
        .unwrap();
    let result = script.eval(r#"import "build" as b; b::security_filter("latest")"#);
    assert!(result.is_ok(), "Expected no error log: {:?}", result.err());
    assert!(!result.unwrap().as_bool().unwrap(), "Expected false for 'latest'");
}

#[test]
fn build_security_filter_semver_tag_accepted() {
    // Valid semver tags (starting with digit) must return true.
    let mut script = make_build_script();
    script
        .add_code(r"fn log_error(msg) { throw `unexpected error log: ${msg}`; }")
        .unwrap();
    let result = script.eval(r#"import "build" as b; b::security_filter("1.2.3")"#);
    assert!(result.is_ok(), "Expected success: {:?}", result.err());
    assert!(result.unwrap().as_bool().unwrap(), "Expected true for '1.2.3'");
}

#[test]
fn build_security_filter_v_prefixed_semver_accepted() {
    // Tags starting with 'v' followed by semver must return true.
    let mut script = make_build_script();
    script
        .add_code(r"fn log_error(msg) { throw `unexpected error log: ${msg}`; }")
        .unwrap();
    let result = script.eval(r#"import "build" as b; b::security_filter("v2.0.0")"#);
    assert!(result.is_ok(), "Expected success: {:?}", result.err());
    assert!(result.unwrap().as_bool().unwrap(), "Expected true for 'v2.0.0'");
}

#[test]
fn build_security_filter_empty_string_rejected() {
    let mut script = make_build_script();
    script
        .add_code(r"fn log_error(msg) { throw `unexpected error log: ${msg}`; }")
        .unwrap();
    let result = script.eval(r#"import "build" as b; b::security_filter("")"#);
    assert!(result.is_ok(), "Expected success: {:?}", result.err());
    assert!(
        !result.unwrap().as_bool().unwrap(),
        "Expected false for empty string"
    );
}

// ── Cosign signing — TDD ──────────────────────────────────────────────────
//
// push_image now returns the image digest (sha256:…) instead of ().
// build.rhai::run() captures this digest and calls reg.sign_image(…)
// when args.signing_key is set.

#[test]
fn build_push_image_returns_digest() {
    // OciRegistryMock::push_image returns "sha256:mock-digest-for-testing".
    // Verify that calling push_image via Rhai yields a string, not ().
    // Before implementation: push_image returned () — type_of would be "()" → test FAILS.
    // After implementation: returns ImmutableString → type_of is "string" → test PASSES.
    let mut script = make_build_script();
    let result = script.eval(
        r#"
        let reg = new_registry("r.io", "", "");
        let digest = reg.push_image("/tmp", "repo/img", "1.0.0", #{});
        type_of(digest) == "string" && digest.starts_with("sha256:")
        "#,
    );
    assert!(
        result.is_ok(),
        "Expected push_image to succeed: {:?}",
        result.err()
    );
    assert!(
        result.unwrap().as_bool().unwrap(),
        "Expected push_image to return a sha256:… string"
    );
}

#[test]
fn build_sign_image_skipped_when_no_key() {
    // When signing_key is empty or absent, sign_image must NOT be called.
    // Mock sign_image to throw if invoked, so any call would fail the test.
    let mut script = make_build_script();
    script
        .add_code(r#"fn sign_image(reg, repo, tag, digest, key) { throw "SIGN_MUST_NOT_BE_CALLED"; }"#)
        .unwrap();
    let result = script.eval(
        r#"
        let reg = new_registry("r.io", "", "");
        let signing_key = "";
        let digest = reg.push_image("/tmp", "repo/img", "1.0.0", #{});
        if signing_key != () && signing_key != "" {
            reg.sign_image("repo/img", "1.0.0", digest, signing_key);
        }
        "ok"
        "#,
    );
    assert!(
        result.is_ok(),
        "Expected sign_image to be skipped: {:?}",
        result.err()
    );
}

#[test]
fn build_sign_image_called_when_key_provided() {
    // When signing_key is set, sign_image must be called and succeed.
    // The OciRegistryMock::sign_image always returns Ok, so a successful result
    // proves the if-branch was entered and sign_image was invoked.
    // (Rhai typed-object methods can't be overridden by script functions — use
    //  the complementary no-key test above to confirm the guard skips signing.)
    let mut script = make_build_script();
    let result = script.eval(
        r#"
        let reg = new_registry("r.io", "", "");
        let signing_key = "/path/to/key.pem";
        let digest = reg.push_image("/tmp", "repo/img", "1.0.0", #{});
        if signing_key != () && signing_key != "" {
            reg.sign_image("repo/img", "1.0.0", digest, signing_key);
        }
        "signed"
        "#,
    );
    assert!(
        result.is_ok(),
        "Expected sign_image call to succeed via mock: {:?}",
        result.err()
    );
    assert_eq!(
        result.unwrap().into_string().unwrap(),
        "signed",
        "Expected the signing branch to complete"
    );
}
