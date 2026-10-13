//! Tenue en charge de la bibliothèque `render_lint` sur le vrai moteur du lint (spec :
//! `scripts/lib/render_lint.sdd`, scénario « a large render stays within the operation budget »).
//!
//! Les tests de `render_lint.rs` tournent sur le moteur complet, sans borne d'opérations : un
//! dépassement du budget y est invisible. Celui-ci passe par `LintRunner`, donc par le moteur
//! restreint, le budget proportionnel et les bornes mémoire de la production.

use std::fs;

use client::testing::{
    lint::{runner::LintRunner, sections::Sections},
    schema::registry::SchemaRegistry,
};
use common::rhaihandler::{Dynamic, to_dynamic};
use serde_json::json;

const LINT_SCRIPT: &str = r#"
import "render_lint" as render_lint;

fn lint(objs, ctx) {
    render_lint::all(objs.all, ctx, #{ include_optional: true })
}
"#;

/// Un rendu réaliste : par groupe de quatre objets, un Deployment qui référence un Secret et un
/// `ServiceAccount` du même rendu, le Service qui le sélectionne, le compte et le secret.
fn render(objects: usize) -> Vec<Dynamic> {
    let mut out = Vec::with_capacity(objects);
    for i in 0..objects {
        let value = match i % 4 {
            0 => json!({
                "apiVersion": "apps/v1", "kind": "Deployment",
                "metadata": {"name": format!("d{i}"), "namespace": "app", "labels": {"app": format!("d{i}")}},
                "spec": {
                    "replicas": 3,
                    "selector": {"matchLabels": {"app": format!("d{i}")}},
                    "template": {
                        "metadata": {"labels": {"app": format!("d{i}")}},
                        "spec": {
                            "serviceAccountName": format!("sa{}", i + 2),
                            "containers": [{
                                "name": "web", "image": "registry.example.test/web:1",
                                "env": [{"name": "A", "valueFrom": {"secretKeyRef": {"name": format!("s{}", i + 3), "key": "k"}}}],
                                "ports": [{"name": "http", "containerPort": 80}]
                            }]
                        }
                    }
                }
            }),
            1 => json!({
                "apiVersion": "v1", "kind": "Service",
                "metadata": {"name": format!("d{}", i - 1), "namespace": "app"},
                "spec": {"selector": {"app": format!("d{}", i - 1)}, "ports": [{"port": 80, "targetPort": "http"}]}
            }),
            2 => json!({
                "apiVersion": "v1", "kind": "ServiceAccount",
                "metadata": {"name": format!("sa{i}"), "namespace": "app"}
            }),
            _ => json!({
                "apiVersion": "v1", "kind": "Secret",
                "metadata": {"name": format!("s{i}"), "namespace": "app"}
            }),
        };
        out.push(to_dynamic(value).expect("objet Rhai"));
    }
    out
}

#[test]
fn a_large_render_stays_within_the_operation_budget() {
    let root = tempfile::tempdir().expect("répertoire temporaire");
    fs::write(root.path().join("lint.rhai"), LINT_SCRIPT).expect("lint.rhai");
    let script_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let registry = SchemaRegistry::new(None, None).expect("registre natif");
    let runner = LintRunner::new(root.path(), &script_dir, registry).expect("runner");

    let objects = render(400);
    let sections = Sections {
        all: objects.clone(),
        unsectioned: objects,
        ..Sections::default()
    };
    let ctx = json!({"package": {"name": "demo"}, "test": {"name": "load"}});
    let outcome = runner.run_test(&ctx, &sections);

    let failed_execution: Vec<&str> = outcome
        .rows
        .iter()
        .filter(|r| r.name == "lint:execution")
        .map(|r| r.message.as_str())
        .collect();
    assert!(
        failed_execution.is_empty(),
        "le lint d'un rendu de 400 objets doit tenir dans le budget : {failed_execution:?}"
    );
    assert!(
        !outcome.rows.is_empty(),
        "le rendu de test doit produire des constats"
    );
}
