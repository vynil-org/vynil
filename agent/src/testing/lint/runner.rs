//! Exécution du script de lint de la distribution (spec : `runner.sdd`).

use crate::testing::{
    lint::finding::{parse_findings, to_rows, to_suppressions},
    result::Suppression,
    schema::{registry::SchemaRegistry, rhai::schema_rhai_register},
    vyniltestset::VynilAssertResult,
};
use common::rhaihandler::{AST, Dynamic, Engine, FileModuleResolver, ModuleResolversCollection, Scope};
use rhai::packages::Package;
use std::path::{Path, PathBuf};

const ENTRY_FILE: &str = "lint.rhai";
const TEST_HOOK: &str = "lint";
const SUITE_HOOK: &str = "lint_suite";
const SUITE_TEST_NAME: &str = "(suite)";
const EXECUTION_RULE: &str = "lint:execution";
/// Borne d'opérations par appel du script. Valeur provisoire : à mesurer sur le script réel
/// de la distribution (décision `[?]` de runner.sdd).
const MAX_OPERATIONS: u64 = 10_000_000;

/// Résultat d'un appel du script : lignes de verdict et inventaire de suppressions.
#[derive(Clone, Debug, Default)]
pub struct LintOutcome {
    pub rows: Vec<VynilAssertResult>,
    pub suppressions: Vec<Suppression>,
}

/// Charge le script de lint d'une distribution et l'appelle par test ou par suite.
#[derive(Clone, Debug)]
pub struct LintRunner {
    lint_dir: PathBuf,
    script_dir: PathBuf,
    schemas: SchemaRegistry,
    ast: AST,
    has_test_hook: bool,
    has_suite_hook: bool,
}

/// Moteur neuf : bibliothèques de données de base, sans horloge, système de fichiers,
/// réseau ni cluster ; fonctions `schema_*` seulement. Les imports suivent `lint_dir` puis
/// `{script_dir}/lib`. Les sorties (`print`, `debug`) sont avalées pour ne pas polluer le
/// rapport du test.
fn build_engine(lint_dir: &Path, script_dir: &Path, schemas: &SchemaRegistry) -> Engine {
    let mut engine = Engine::new_raw();
    engine.register_global_module(rhai::packages::CorePackage::new().as_shared_module());
    engine.register_global_module(rhai::packages::LogicPackage::new().as_shared_module());
    engine.register_global_module(rhai::packages::BasicMathPackage::new().as_shared_module());
    engine.register_global_module(rhai::packages::BasicArrayPackage::new().as_shared_module());
    engine.register_global_module(rhai::packages::BasicMapPackage::new().as_shared_module());
    engine.register_global_module(rhai::packages::BasicStringPackage::new().as_shared_module());
    engine.register_global_module(rhai::packages::MoreStringPackage::new().as_shared_module());
    engine.register_global_module(rhai::packages::BasicFnPackage::new().as_shared_module());
    engine.register_global_module(rhai::packages::BasicIteratorPackage::new().as_shared_module());
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_expr_depths(256, 128);
    engine.set_max_call_levels(512);

    let mut resolver = ModuleResolversCollection::new();
    resolver.push(FileModuleResolver::new_with_path(lint_dir));
    resolver.push(FileModuleResolver::new_with_path(script_dir.join("lib")));
    engine.set_module_resolver(resolver);

    schema_rhai_register(&mut engine, schemas.clone());
    engine
}

fn execution_outcome(message: String) -> LintOutcome {
    LintOutcome {
        rows: vec![VynilAssertResult {
            name: EXECUTION_RULE.to_string(),
            description: Some("Lint script execution".to_string()),
            passed: false,
            message,
        }],
        suppressions: Vec::new(),
    }
}

impl LintRunner {
    /// # Errors
    ///
    /// Returns a [`common::Error`] when `lint.rhai` is missing, does not compile, or defines
    /// neither `lint` nor `lint_suite`.
    pub fn new(lint_dir: &Path, script_dir: &Path, schemas: SchemaRegistry) -> common::Result<Self> {
        let path = lint_dir.join(ENTRY_FILE);
        if !path.is_file() {
            return Err(common::Error::Other(format!(
                "{}: fichier de lint introuvable",
                path.display()
            )));
        }
        let engine = build_engine(lint_dir, script_dir, &schemas);
        let ast = engine
            .compile_file(path.clone())
            .map_err(|e| common::Error::Other(format!("{}: {e}", path.display())))?;
        let has_test_hook = ast.iter_functions().any(|f| f.name == TEST_HOOK);
        let has_suite_hook = ast.iter_functions().any(|f| f.name == SUITE_HOOK);
        if !has_test_hook && !has_suite_hook {
            return Err(common::Error::Other(format!(
                "{}: ni `{TEST_HOOK}` ni `{SUITE_HOOK}` n'est défini",
                path.display()
            )));
        }
        Ok(Self {
            lint_dir: lint_dir.to_path_buf(),
            script_dir: script_dir.to_path_buf(),
            schemas,
            ast,
            has_test_hook,
            has_suite_hook,
        })
    }

    /// Appelle `lint(objs, ctx)` ; `ctx.test.name` nomme les suppressions.
    #[must_use]
    pub fn run_test(&self, ctx: &serde_json::Value, objects: &[Dynamic]) -> LintOutcome {
        if !self.has_test_hook {
            return LintOutcome::default();
        }
        let test_name = ctx
            .pointer("/test/name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let objs = Dynamic::from_array(objects.to_vec());
        match self.call(TEST_HOOK, ctx, Some(objs)) {
            Ok(value) => Self::outcome(TEST_HOOK, &value, test_name),
            Err(message) => execution_outcome(message),
        }
    }

    /// Appelle `lint_suite(ctx)` ; les suppressions sont nommées `(suite)`.
    #[must_use]
    pub fn run_suite(&self, ctx: &serde_json::Value) -> LintOutcome {
        if !self.has_suite_hook {
            return LintOutcome::default();
        }
        match self.call(SUITE_HOOK, ctx, None) {
            Ok(value) => Self::outcome(SUITE_HOOK, &value, SUITE_TEST_NAME),
            Err(message) => execution_outcome(message),
        }
    }

    /// Évalue le script dans un moteur neuf puis appelle `name` avec `objs` (si présent)
    /// puis `ctx`.
    fn call(&self, name: &str, ctx: &serde_json::Value, objs: Option<Dynamic>) -> Result<Dynamic, String> {
        let ctx = rhai::serde::to_dynamic(ctx).map_err(|e| e.to_string())?;
        let mut args = Vec::with_capacity(2);
        args.extend(objs);
        args.push(ctx);
        let engine = build_engine(&self.lint_dir, &self.script_dir, &self.schemas);
        let mut scope = Scope::new();
        engine
            .run_ast_with_scope(&mut scope, &self.ast)
            .map_err(|e| e.to_string())?;
        engine
            .call_fn::<Dynamic>(&mut scope, &self.ast, name, args)
            .map_err(|e| e.to_string())
    }

    fn outcome(hook: &str, value: &Dynamic, test_name: &str) -> LintOutcome {
        if !value.is_array() {
            return execution_outcome(format!(
                "le retour de `{hook}` doit être un tableau, reçu {}",
                value.type_name()
            ));
        }
        let parsed = parse_findings(value);
        LintOutcome {
            rows: to_rows(&parsed.findings, &parsed.invalid),
            suppressions: to_suppressions(test_name, &parsed.findings),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, path::PathBuf};

    /// Répertoires de test : `lint/` (lint.rhai) et `scripts/lib/` (modules de la bibliothèque).
    struct Fixture {
        _root: tempfile::TempDir,
        lint_dir: PathBuf,
        script_dir: PathBuf,
    }

    fn fixture(lint_script: Option<&str>) -> Fixture {
        let root = tempfile::tempdir().expect("répertoire temporaire");
        let lint_dir = root.path().join("distro");
        let script_dir = root.path().join("scripts");
        fs::create_dir_all(&lint_dir).expect("lint dir");
        fs::create_dir_all(script_dir.join("lib")).expect("lib dir");
        if let Some(src) = lint_script {
            fs::write(lint_dir.join("lint.rhai"), src).expect("lint.rhai");
        }
        Fixture {
            _root: root,
            lint_dir,
            script_dir,
        }
    }

    fn runner(script: &str) -> (Fixture, LintRunner) {
        let fx = fixture(Some(script));
        let registry = SchemaRegistry::new(None, None).expect("registre natif");
        let runner = LintRunner::new(&fx.lint_dir, &fx.script_dir, registry).expect("runner");
        (fx, runner)
    }

    fn deployment(name: &str) -> Dynamic {
        common::rhaihandler::to_dynamic(json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {"name": name, "namespace": "app"},
        }))
        .expect("objet Rhai")
    }

    fn test_ctx(name: &str) -> serde_json::Value {
        json!({"package": {"name": "demo"}, "test": {"name": name}})
    }

    /// Une règle qui retourne un constat `error` par objet reçu.
    const ONE_ERROR_PER_OBJECT: &str = r#"
        fn lint(objs, ctx) {
            let out = [];
            for obj in objs {
                out.push(#{
                    rule: "latest-tag", severity: "error",
                    object: #{ apiVersion: obj.apiVersion, kind: obj.kind, namespace: obj.metadata.namespace, name: obj.metadata.name },
                    message: "image uses the latest tag"
                });
            }
            out
        }
    "#;

    #[test]
    fn missing_entry_point_fails_construction() {
        let fx = fixture(None);
        let registry = SchemaRegistry::new(None, None).expect("registre natif");
        let err = LintRunner::new(&fx.lint_dir, &fx.script_dir, registry)
            .expect_err("sans lint.rhai, la construction échoue");
        let expected = fx.lint_dir.join("lint.rhai").display().to_string();
        assert!(err.to_string().contains(&expected), "{err}");
    }

    #[test]
    fn script_returning_findings_yields_rows() {
        let (_fx, runner) = runner(ONE_ERROR_PER_OBJECT);
        let objects = vec![deployment("web"), deployment("api")];
        let outcome = runner.run_test(&test_ctx("default"), &objects);
        assert_eq!(outcome.rows.len(), 2);
        assert_eq!(outcome.rows[0].name, "lint:latest-tag");
        assert_eq!(outcome.rows[0].description.as_deref(), Some("Deployment app/web"));
        assert_eq!(outcome.rows[1].description.as_deref(), Some("Deployment app/api"));
        assert!(outcome.rows.iter().all(|r| !r.passed));
    }

    #[test]
    fn a_throwing_script_becomes_one_failed_row() {
        let (_fx, runner) = runner(r#"fn lint(objs, ctx) { throw "boom from the rule"; }"#);
        let outcome = runner.run_test(&test_ctx("default"), &[deployment("web")]);
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].name, "lint:execution");
        assert!(!outcome.rows[0].passed);
        assert!(
            outcome.rows[0].message.contains("boom from the rule"),
            "{}",
            outcome.rows[0].message
        );
    }

    #[test]
    fn a_script_that_touches_the_cluster_fails() {
        let (_fx, runner) = runner(r#"fn lint(objs, ctx) { k8s_resource("Deployment", "web", "app"); [] }"#);
        let outcome = runner.run_test(&test_ctx("default"), &[]);
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].name, "lint:execution");
        assert!(!outcome.rows[0].passed);
        assert!(
            outcome.rows[0].message.contains("not found"),
            "{}",
            outcome.rows[0].message
        );
    }

    #[test]
    fn a_non_array_return_is_a_failed_row() {
        let (_fx, runner) = runner(r#"fn lint(objs, ctx) { #{ rule: "x" } }"#);
        let outcome = runner.run_test(&test_ctx("default"), &[]);
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].name, "lint:execution");
        assert!(!outcome.rows[0].passed);
        assert!(
            outcome.rows[0].message.contains("tableau"),
            "{}",
            outcome.rows[0].message
        );
    }

    #[test]
    fn suite_hook_runs_once() {
        let (_fx, runner) = runner(
            r#"
            fn lint(objs, ctx) { throw "lint must not run for the suite"; }
            fn lint_suite(ctx) {
                [#{
                    rule: "suite-size", severity: "error",
                    object: #{ apiVersion: "v1", kind: "Suite", name: "all" },
                    message: "suite checked"
                }]
            }
            "#,
        );
        let outcome = runner.run_suite(&json!({"suite": {"executed": ["a"]}}));
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].name, "lint:suite-size");
        assert_eq!(outcome.rows[0].description.as_deref(), Some("Suite all"));
    }

    #[test]
    fn an_endless_loop_is_cut_by_the_operations_limit() {
        let (_fx, runner) = runner(r"fn lint(objs, ctx) { loop {} }");
        let outcome = runner.run_test(&test_ctx("default"), &[]);
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].name, "lint:execution");
        assert!(!outcome.rows[0].passed);
    }
}
