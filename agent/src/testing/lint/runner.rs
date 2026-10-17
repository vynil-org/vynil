//! Exécution du script de lint de la distribution (spec : `runner.sdd`).

use crate::testing::{
    lint::{
        finding::{parse_findings, to_rows, to_suppressions},
        sections::Sections,
    },
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
/// Budget d'opérations d'un appel : `OPERATIONS_BASE` plus `OPERATIONS_PER_OBJECT` par objet
/// reçu. Une charge honnête croît avec la taille du rendu ; la borne ne sert qu'à arrêter un
/// script qui boucle. Valeurs à mesurer sur le script réel de la distribution (décision `[?]`
/// de runner.sdd).
const OPERATIONS_BASE: u64 = 10_000_000;
const OPERATIONS_PER_OBJECT: u64 = 50_000;
/// Bornes mémoire du moteur : le budget d'opérations ne protège pas d'un `s += s`.
const MAX_STRING_SIZE: usize = 16 * 1024 * 1024;
const MAX_ARRAY_SIZE: usize = 1_000_000;
const MAX_MAP_SIZE: usize = 100_000;

/// Budget d'opérations d'un appel qui reçoit `object_count` objets.
const fn operations_budget(object_count: usize) -> u64 {
    OPERATIONS_BASE.saturating_add(OPERATIONS_PER_OBJECT.saturating_mul(object_count as u64))
}

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
fn build_engine(lint_dir: &Path, script_dir: &Path, schemas: &SchemaRegistry, max_operations: u64) -> Engine {
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
    engine.set_max_operations(max_operations);
    engine.set_max_string_size(MAX_STRING_SIZE);
    engine.set_max_array_size(MAX_ARRAY_SIZE);
    engine.set_max_map_size(MAX_MAP_SIZE);
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
                "{}: lint file not found",
                path.display()
            )));
        }
        let engine = build_engine(lint_dir, script_dir, &schemas, operations_budget(0));
        let ast = engine
            .compile_file(path.clone())
            .map_err(|e| common::Error::Other(format!("{}: {e}", path.display())))?;
        let has_test_hook = ast.iter_functions().any(|f| f.name == TEST_HOOK);
        let has_suite_hook = ast.iter_functions().any(|f| f.name == SUITE_HOOK);
        if !has_test_hook && !has_suite_hook {
            return Err(common::Error::Other(format!(
                "{}: neither `{TEST_HOOK}` nor `{SUITE_HOOK}` is defined",
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

    /// Vrai si la distribution définit `lint` (voie de test).
    #[must_use]
    pub const fn has_test_hook(&self) -> bool {
        self.has_test_hook
    }

    /// Appelle `lint(objs, ctx)` ; `ctx.test.name` nomme les suppressions.
    /// Vrai si la distribution définit `lint_suite` (le hook de suite n'a alors qu'un
    /// enregistrement de suite à produire, même sans constat).
    #[must_use]
    pub const fn has_suite_hook(&self) -> bool {
        self.has_suite_hook
    }

    #[must_use]
    pub fn run_test(&self, ctx: &serde_json::Value, objects: &Sections) -> LintOutcome {
        if !self.has_test_hook {
            return LintOutcome::default();
        }
        let test_name = ctx
            .pointer("/test/name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let budget = operations_budget(objects.all.len());
        match self.call(TEST_HOOK, ctx, Some(objects.to_dynamic()), budget) {
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
        match self.call(SUITE_HOOK, ctx, None, operations_budget(0)) {
            Ok(value) => Self::outcome(SUITE_HOOK, &value, SUITE_TEST_NAME),
            Err(message) => execution_outcome(message),
        }
    }

    /// Évalue le script dans un moteur neuf puis appelle `name` avec `objs` (si présent)
    /// puis `ctx`.
    fn call(
        &self,
        name: &str,
        ctx: &serde_json::Value,
        objs: Option<Dynamic>,
        max_operations: u64,
    ) -> Result<Dynamic, String> {
        let ctx = rhai::serde::to_dynamic(ctx).map_err(|e| e.to_string())?;
        let mut args = Vec::with_capacity(2);
        args.extend(objs);
        args.push(ctx);
        let engine = build_engine(&self.lint_dir, &self.script_dir, &self.schemas, max_operations);
        // `call_fn` évalue déjà les instructions de premier niveau (`eval_ast` vrai par défaut) :
        // une évaluation préalable du script les exécuterait une seconde fois.
        let mut scope = Scope::new();
        engine
            .call_fn::<Dynamic>(&mut scope, &self.ast, name, args)
            .map_err(|e| e.to_string())
    }

    fn outcome(hook: &str, value: &Dynamic, test_name: &str) -> LintOutcome {
        if !value.is_array() {
            return execution_outcome(format!(
                "the return value of `{hook}` must be an array, got {}",
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

    fn configmap(name: &str) -> Dynamic {
        common::rhaihandler::to_dynamic(json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {"name": name, "namespace": "app"},
        }))
        .expect("objet Rhai")
    }

    fn deployment(name: &str) -> Dynamic {
        common::rhaihandler::to_dynamic(json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {"name": name, "namespace": "app"},
        }))
        .expect("objet Rhai")
    }

    /// Sections dont seul `all` (et donc `unsectioned`) porte les objets : le cas simple.
    fn plain(objects: Vec<Dynamic>) -> Sections {
        Sections {
            all: objects.clone(),
            unsectioned: objects,
            ..Sections::default()
        }
    }

    fn test_ctx(name: &str) -> serde_json::Value {
        json!({"package": {"name": "demo"}, "test": {"name": name}})
    }

    /// Une règle qui retourne un constat `error` par objet reçu.
    const ONE_ERROR_PER_OBJECT: &str = r#"
        fn lint(objs, ctx) {
            let out = [];
            for obj in objs.all {
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
        let outcome = runner.run_test(&test_ctx("default"), &plain(objects));
        assert_eq!(outcome.rows.len(), 2);
        assert_eq!(outcome.rows[0].name, "lint:latest-tag");
        assert_eq!(outcome.rows[0].description.as_deref(), Some("Deployment app/web"));
        assert_eq!(outcome.rows[1].description.as_deref(), Some("Deployment app/api"));
        assert!(outcome.rows.iter().all(|r| !r.passed));
    }

    #[test]
    fn the_script_sees_every_section() {
        // Le script signale, par clé de `objs` non vide, une ligne ; une clé absente donne `missing`.
        let (_fx, runner) = runner(
            r#"
            fn lint(objs, ctx) {
                let out = [];
                for key in ["all", "crds", "systems", "befores", "vitals", "scalables", "others", "posts", "unsectioned"] {
                    if !objs.contains(key) {
                        out.push(#{ rule: "missing", severity: "error", object: #{ apiVersion: "v1", kind: "Key", name: key }, message: "absente" });
                    } else if objs[key].len() > 0 {
                        out.push(#{ rule: key, severity: "error", object: #{ apiVersion: "v1", kind: "Key", name: key }, message: "remplie" });
                    }
                }
                out
            }
            "#,
        );
        let sections = Sections {
            all: vec![deployment("web"), configmap("cfg")],
            scalables: vec![deployment("web")],
            befores: vec![configmap("cfg")],
            ..Sections::default()
        };
        let outcome = runner.run_test(&test_ctx("default"), &sections);
        let names: Vec<&str> = outcome.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["lint:all", "lint:befores", "lint:scalables"]);
    }

    #[test]
    fn a_runaway_string_is_bounded() {
        let (_fx, runner) = runner(r#"fn lint(objs, ctx) { let s = "x"; loop { s += s; } }"#);
        let outcome = runner.run_test(&test_ctx("default"), &plain(vec![deployment("web")]));
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].name, "lint:execution");
        assert!(!outcome.rows[0].passed);
    }

    #[test]
    fn the_operation_budget_grows_with_the_object_count() {
        assert_eq!(operations_budget(0), OPERATIONS_BASE);
        assert!(operations_budget(1_000) > operations_budget(100));
        assert!(
            operations_budget(usize::MAX) >= OPERATIONS_BASE,
            "pas de débordement"
        );
    }

    #[test]
    fn a_throwing_script_becomes_one_failed_row() {
        let (_fx, runner) = runner(r#"fn lint(objs, ctx) { throw "boom from the rule"; }"#);
        let outcome = runner.run_test(&test_ctx("default"), &plain(vec![deployment("web")]));
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
        let outcome = runner.run_test(&test_ctx("default"), &Sections::default());
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
        let outcome = runner.run_test(&test_ctx("default"), &Sections::default());
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].name, "lint:execution");
        assert!(!outcome.rows[0].passed);
        assert!(
            outcome.rows[0].message.contains("must be an array"),
            "{}",
            outcome.rows[0].message
        );
    }

    #[test]
    fn engine_messages_are_in_english() {
        let (_fx, runner) = runner(r#"fn lint(objs, ctx) { #{ rule: "x" } }"#);
        let outcome = runner.run_test(&test_ctx("default"), &Sections::default());
        assert_eq!(outcome.rows.len(), 1);
        assert!(
            outcome.rows[0].message.contains("must be an array"),
            "{}",
            outcome.rows[0].message
        );

        let fx = fixture(None);
        let registry = SchemaRegistry::new(None, None).expect("registre natif");
        let err = LintRunner::new(&fx.lint_dir, &fx.script_dir, registry)
            .expect_err("sans lint.rhai, la construction échoue");
        let text = err.to_string();
        assert!(text.contains("lint.rhai") && text.contains("not found"), "{text}");
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

    /// Itérations de la boucle de premier niveau : environ 3 opérations chacune, soit près de
    /// 60 % du budget d'un appel sans objet (10 000 000).
    const TOP_LEVEL_ITERATIONS: u64 = 2_000_000;

    /// Garde de non-régression, pas une preuve d'évaluation unique : le nombre d'évaluations du
    /// premier niveau n'est pas observable depuis le script (les compteurs d'opérations étaient
    /// distincts, le budget par appel n'était pas consommé deux fois). Ce test vérifie que la
    /// boucle de premier niveau tient dans le budget et que la liaison `seuil` reste visible au
    /// premier niveau. Une fonction Rhai ne voit pas les liaisons du premier niveau, `lint`
    /// compris : ce n'est pas testable depuis `lint`.
    #[test]
    fn top_level_statements_run_once_per_call() {
        let script = format!(
            "let seuil = 7;\nlet spent = 0;\nfor i in 0..{TOP_LEVEL_ITERATIONS} {{ spent += 1; }}\n\
             if seuil != 7 {{ throw \"liaison de premier niveau invisible\"; }}\n\
             fn lint(objs, ctx) {{ [] }}\n"
        );
        let (_fx, runner) = runner(&script);
        let outcome = runner.run_test(&test_ctx("default"), &Sections::default());
        assert!(outcome.rows.is_empty(), "{:?}", outcome.rows);
    }

    #[test]
    fn the_test_hook_is_exposed() {
        let (_fx, suite_only) = runner("fn lint_suite(ctx) { [] }");
        assert!(!suite_only.has_test_hook());
        assert!(suite_only.has_suite_hook());
        let (_fx2, test_only) = runner("fn lint(objs, ctx) { [] }");
        assert!(test_only.has_test_hook());
        assert!(!test_only.has_suite_hook());
    }

    #[test]
    fn an_endless_loop_is_cut_by_the_operations_limit() {
        let (_fx, runner) = runner(r"fn lint(objs, ctx) { loop {} }");
        let outcome = runner.run_test(&test_ctx("default"), &Sections::default());
        assert_eq!(outcome.rows.len(), 1);
        assert_eq!(outcome.rows[0].name, "lint:execution");
        assert!(!outcome.rows[0].passed);
    }
}
