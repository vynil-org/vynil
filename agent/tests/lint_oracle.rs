//! Harnais d'oracle de la bibliothèque `render_lint` (spec : `tests/lint_oracle.sdd`).
//!
//! Les tests ordinaires comparent chaque vérification portée à son verdict consigné
//! (`fixtures/render_lint/{nom}/expected.json`) et contrôlent la cohérence du catalogue, sans
//! binaire externe. Les tests marqués `#[ignore]` interrogent l'outil de référence
//! (`KUBE_LINTER_BIN`) ; la régénération des verdicts est un test explicite, gardé par
//! `RENDER_LINT_REGENERATE=1`, jamais un effet de bord.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use client::testing::lint::{multimap, patterns};
use common::rhaihandler::Script;
use serde::Deserialize;
use serde_json::{Value, json};

const ENV_BIN: &str = "KUBE_LINTER_BIN";
const ENV_REGENERATE: &str = "RENDER_LINT_REGENERATE";
const THEMES: [&str; 4] = ["pod", "workload", "network", "rbac"];

/// Clé d'un constat, telle que la compare le verdict : `(rule, kind, namespace, name)`,
/// `namespace` vide quand l'objet n'en porte pas.
type Key = (String, String, String, String);

#[derive(Deserialize)]
struct Catalogue {
    oracle_version: String,
    checks: Vec<Entry>,
}

#[derive(Deserialize, Clone)]
struct Entry {
    name: String,
    theme: String,
    status: String,
    #[serde(default)]
    default: bool,
    reason: Option<String>,
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn lib_dir() -> PathBuf {
    crate_dir().join("scripts").join("lib")
}

fn fixtures_dir() -> PathBuf {
    crate_dir().join("tests").join("fixtures").join("render_lint")
}

fn catalogue_path() -> PathBuf {
    fixtures_dir().join("catalogue.yaml")
}

fn read_catalogue(path: &Path) -> Catalogue {
    let text = fs::read_to_string(path).expect("catalogue lisible");
    serde_yaml::from_str(&text).expect("catalogue valide")
}

/// Moteur de la bibliothèque avec `regex_is_match` câblée comme dans le runner de lint.
fn library(lib: &Path) -> Script {
    let mut rhai = Script::new_mock(
        vec![lib.display().to_string()],
        vec![],
        vec![],
        Arc::new(Mutex::new(Vec::new())),
    );
    patterns::register(&mut rhai.engine);
    multimap::register(&mut rhai.engine);
    rhai
}

/// Objets d'un fichier multi-documents, sans les documents vides.
fn load_objects(path: &Path) -> Vec<Value> {
    let text = fs::read_to_string(path).expect("manifestes lisibles");
    let mut objects = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(&text) {
        let value = Value::deserialize(doc).expect("manifeste YAML valide");
        if !value.is_null() {
            objects.push(value);
        }
    }
    objects
}

/// Noms des fonctions publiques d'un module Rhai, dans l'ordre du fichier.
fn public_functions(path: &Path) -> Vec<String> {
    // Mêmes profondeurs que le runner de lint (`agent/src/testing/lint/runner.rs`).
    let mut engine = rhai::Engine::new();
    engine.set_max_expr_depths(256, 128);
    let ast = engine
        .compile_file(path.to_path_buf())
        .expect("module Rhai compilable");
    ast.iter_functions()
        .filter(|f| f.access == rhai::FnAccess::Public)
        .map(|f| f.name.to_string())
        .collect()
}

fn function_name(check: &str) -> String {
    check.replace('-', "_")
}

fn check_name(function: &str) -> String {
    function.replace('_', "-")
}

/// Problèmes de cohérence du catalogue, un message par problème, nommant l'entrée ou la
/// fonction en cause. Ne lance aucun binaire ; ne lit que les fichiers passés.
fn catalogue_problems(catalogue: &Path, lib: &Path, fixtures: &Path) -> Vec<String> {
    let catalogue = read_catalogue(catalogue);
    let mut problems = Vec::new();
    for theme in THEMES {
        let module = lib.join(format!("render_lint_{theme}.rhai"));
        if !module.is_file() {
            problems.push(format!("module `render_lint_{theme}.rhai` absent"));
            continue;
        }
        // `index` est le constructeur d'index du thème, pas une vérification.
        for function in public_functions(&module).into_iter().filter(|f| f != "index") {
            let listed = catalogue
                .checks
                .iter()
                .any(|e| e.name == check_name(&function) && e.theme == theme && e.status == "ported");
            if !listed {
                problems.push(format!(
                    "fonction `{function}` dans render_lint_{theme}.rhai sans entrée `ported` du thème `{theme}`"
                ));
            }
        }
    }
    for entry in &catalogue.checks {
        match entry.status.as_str() {
            "ported" => {
                let module = lib.join(format!("render_lint_{}.rhai", entry.theme));
                let defined =
                    module.is_file() && public_functions(&module).contains(&function_name(&entry.name));
                if !defined {
                    problems.push(format!(
                        "vérification `{}` portée (thème `{}`) : fonction `{}` absente de render_lint_{}.rhai",
                        entry.name,
                        entry.theme,
                        function_name(&entry.name),
                        entry.theme
                    ));
                }
                let dir = fixtures.join(&entry.name);
                for file in ["manifests.yaml", "expected.json"] {
                    if !dir.join(file).is_file() {
                        problems.push(format!(
                            "vérification `{}` portée : fixture `{file}` absente",
                            entry.name
                        ));
                    }
                }
            }
            "deferred" | "excluded" => {
                let has_reason = entry.reason.as_deref().is_some_and(|r| !r.trim().is_empty());
                if !has_reason {
                    problems.push(format!(
                        "vérification `{}` ({}) sans `reason`",
                        entry.name, entry.status
                    ));
                }
            }
            other => problems.push(format!(
                "vérification `{}` : statut `{other}` inconnu",
                entry.name
            )),
        }
    }
    problems
}

/// Clés d'un verdict de l'oracle, triées et sans doublon.
fn verdict_keys(findings: &[Value]) -> Vec<Key> {
    let mut keys: Vec<Key> = findings
        .iter()
        .map(|f| {
            let object = &f["object"];
            (
                f["rule"].as_str().unwrap_or_default().to_string(),
                object["kind"].as_str().unwrap_or_default().to_string(),
                object["namespace"].as_str().unwrap_or_default().to_string(),
                object["name"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    keys.sort();
    keys
}

fn expected_keys(path: &Path) -> Vec<Key> {
    let text = fs::read_to_string(path).expect("expected.json lisible");
    let items: Vec<Value> = serde_json::from_str(&text).expect("expected.json valide");
    let mut keys: Vec<Key> = items
        .iter()
        .map(|item| {
            (
                item["rule"].as_str().unwrap_or_default().to_string(),
                item["kind"].as_str().unwrap_or_default().to_string(),
                item["namespace"].as_str().unwrap_or_default().to_string(),
                item["name"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    keys.sort();
    keys
}

/// Constats de la bibliothèque pour `check` seul, sur les objets de `manifests`.
fn library_findings(check: &str, manifests: &Path) -> Vec<Value> {
    let mut rhai = library(&lib_dir());
    rhai.set_dynamic("objs", &Value::Array(load_objects(manifests)));
    let script = format!(
        r#"import "render_lint" as render_lint;
           render_lint::all(objs, #{{}}, #{{only: ["{check}"]}})"#
    );
    let value = rhai.eval(&script).expect("render_lint::all doit s'évaluer");
    let json: Value = rhai::serde::from_dynamic(&value).expect("constats en JSON");
    json.as_array().cloned().unwrap_or_default()
}

fn ported_names() -> Vec<String> {
    let mut names: Vec<String> = read_catalogue(&catalogue_path())
        .checks
        .into_iter()
        .filter(|e| e.status == "ported")
        .map(|e| e.name)
        .collect();
    names.sort();
    names
}

// ===== Tests ordinaires : sans binaire externe =====

#[test]
fn a_ported_check_matches_its_recorded_verdict() {
    let mut mismatches = Vec::new();
    for name in ported_names() {
        let dir = fixtures_dir().join(&name);
        let got = verdict_keys(&library_findings(&name, &dir.join("manifests.yaml")));
        let want = expected_keys(&dir.join("expected.json"));
        if got != want {
            mismatches.push(format!("{name}: obtenu {got:?}, attendu {want:?}"));
        }
    }
    assert!(mismatches.is_empty(), "divergences :\n{}", mismatches.join("\n"));
}

#[test]
fn real_catalogue_is_consistent() {
    let problems = catalogue_problems(&catalogue_path(), &lib_dir(), &fixtures_dir());
    assert!(
        problems.is_empty(),
        "catalogue incohérent :\n{}",
        problems.join("\n")
    );

    let mut rhai = library(&lib_dir());
    let value = rhai
        .eval(r#"import "render_lint" as render_lint; render_lint::checks()"#)
        .expect("checks() doit s'évaluer");
    let listed: Vec<String> = rhai::serde::from_dynamic(&value).expect("tableau de noms");
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted, "checks() doit être trié");
    assert_eq!(
        listed,
        ported_names(),
        "checks() doit lister exactement les entrées portées"
    );
}

#[test]
fn the_default_set_matches_the_catalogue() {
    let mut rhai = library(&lib_dir());
    let value = rhai
        .eval(r#"import "render_lint" as render_lint; render_lint::default_checks()"#)
        .expect("default_checks() doit s'évaluer");
    let listed: Vec<String> = rhai::serde::from_dynamic(&value).expect("tableau de noms");
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted, "default_checks() doit être trié");
    let mut expected: Vec<String> = read_catalogue(&catalogue_path())
        .checks
        .into_iter()
        .filter(|e| e.status == "ported" && e.default)
        .map(|e| e.name)
        .collect();
    expected.sort();
    assert_eq!(
        listed, expected,
        "default_checks() doit lister exactement les entrées portées actives par défaut"
    );
}

#[test]
fn a_check_without_a_function_is_a_catalogue_error() {
    let tmp = tempfile::tempdir().expect("répertoire temporaire");
    let lib = tmp.path().join("lib");
    fs::create_dir_all(&lib).expect("lib");
    for theme in THEMES {
        fs::write(lib.join(format!("render_lint_{theme}.rhai")), "").expect("module");
    }
    let catalogue = tmp.path().join("catalogue.yaml");
    fs::write(
        &catalogue,
        "oracle_version: v0.8.3\nchecks:\n  - name: latest-tag\n    theme: pod\n    status: ported\n",
    )
    .expect("catalogue");
    let problems = catalogue_problems(&catalogue, &lib, tmp.path());
    assert!(
        problems
            .iter()
            .any(|p| p.contains("latest-tag") && p.contains("latest_tag")),
        "le problème doit nommer la vérification : {problems:?}"
    );
}

#[test]
fn an_unlisted_function_is_a_catalogue_error() {
    let tmp = tempfile::tempdir().expect("répertoire temporaire");
    let lib = tmp.path().join("lib");
    fs::create_dir_all(&lib).expect("lib");
    for theme in THEMES {
        fs::write(lib.join(format!("render_lint_{theme}.rhai")), "").expect("module");
    }
    fs::write(
        lib.join("render_lint_pod.rhai"),
        "fn orphan_check(obj, objs, ctx, params) { [] }\n",
    )
    .expect("module pod");
    let catalogue = tmp.path().join("catalogue.yaml");
    fs::write(&catalogue, "oracle_version: v0.8.3\nchecks: []\n").expect("catalogue");
    let problems = catalogue_problems(&catalogue, &lib, tmp.path());
    assert!(
        problems.iter().any(|p| p.contains("orphan_check")),
        "le problème doit nommer la fonction : {problems:?}"
    );
}

#[test]
fn a_deferred_check_carries_its_reason() {
    let tmp = tempfile::tempdir().expect("répertoire temporaire");
    let lib = tmp.path().join("lib");
    fs::create_dir_all(&lib).expect("lib");
    for theme in THEMES {
        fs::write(lib.join(format!("render_lint_{theme}.rhai")), "").expect("module");
    }
    let catalogue = tmp.path().join("catalogue.yaml");
    fs::write(
        &catalogue,
        "oracle_version: v0.8.3\nchecks:\n  - name: host-pid\n    theme: pod\n    status: deferred\n",
    )
    .expect("catalogue");
    let problems = catalogue_problems(&catalogue, &lib, tmp.path());
    assert!(
        problems
            .iter()
            .any(|p| p.contains("host-pid") && p.contains("reason")),
        "le problème doit nommer l'entrée : {problems:?}"
    );
}

/// Garde-fou : les tests qui interrogent l'outil de référence sont `#[ignore]`, pour que
/// `cargo test` ordinaire n'exige jamais le binaire. Vérifie que l'attribut précède bien
/// chacun de ces tests dans ce fichier.
#[test]
fn ordinary_test_run_needs_no_external_tool() {
    const TOOL_TESTS: [&str; 3] = [
        "the_oracle_inventory_drifts",
        "oracle_verdicts_match_the_tool_on_the_fixtures",
        "regenerate_expected_verdicts",
    ];
    let source = include_str!("lint_oracle.rs");
    let unguarded: Vec<&str> = TOOL_TESTS
        .into_iter()
        .filter(|name| {
            let Some(at) = source.find(&format!("fn {name}(")) else {
                return true;
            };
            let before = source.get(..at).unwrap_or_default();
            let attrs: String = before.lines().rev().take(3).collect::<Vec<_>>().join("\n");
            !attrs.contains("#[ignore")
        })
        .collect();
    assert!(
        unguarded.is_empty(),
        "tests d'oracle sans #[ignore] : {unguarded:?}"
    );
}

// ===== Tests d'oracle : avec KUBE_LINTER_BIN, marqués #[ignore] =====

fn kube_linter_bin() -> PathBuf {
    let path = std::env::var(ENV_BIN)
        .unwrap_or_else(|_| panic!("{ENV_BIN} doit désigner le binaire kube-linter v0.8.3 pour ce test"));
    PathBuf::from(path)
}

/// Lance l'outil et rend le JSON de sortie. L'outil sort en code non nul dès qu'il trouve
/// des constats : ce n'est pas une erreur.
fn tool_reports(bin: &Path, manifests: &Path, check: Option<&str>) -> Vec<Value> {
    let mut cmd = Command::new(bin);
    cmd.args(["lint"])
        .arg(manifests)
        .args(["--do-not-auto-add-defaults", "--format", "json"]);
    if let Some(check) = check {
        cmd.args(["--include", check]);
    }
    let output = cmd.output().expect("exécution de kube-linter");
    let json: Value = serde_json::from_slice(&output.stdout).expect("sortie JSON de kube-linter");
    json["Reports"].as_array().cloned().unwrap_or_default()
}

fn tool_keys(reports: &[Value]) -> Vec<Key> {
    let mut keys: Vec<Key> = reports
        .iter()
        .map(|r| {
            let k8s = &r["Object"]["K8sObject"];
            (
                r["Check"].as_str().unwrap_or_default().to_string(),
                k8s["GroupVersionKind"]["Kind"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                k8s["Namespace"].as_str().unwrap_or_default().to_string(),
                k8s["Name"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    keys.sort();
    keys
}

#[test]
#[ignore = "exige KUBE_LINTER_BIN (kube-linter v0.8.3)"]
fn the_oracle_inventory_drifts() {
    let bin = kube_linter_bin();
    let output = Command::new(&bin)
        .args(["checks", "list", "--format", "json"])
        .output()
        .expect("kube-linter checks list");
    let inventory: Vec<Value> =
        serde_json::from_slice(&output.stdout).expect("inventaire JSON de kube-linter");
    let tool: BTreeSet<String> = inventory
        .iter()
        .filter_map(|c| c["name"].as_str().map(str::to_string))
        .collect();
    let catalogue = read_catalogue(&catalogue_path());
    let listed: BTreeSet<String> = catalogue.checks.iter().map(|e| e.name.clone()).collect();
    let missing: Vec<&String> = tool.difference(&listed).collect();
    let extra: Vec<&String> = listed.difference(&tool).collect();
    assert!(
        missing.is_empty(),
        "vérifications de l'outil absentes du catalogue : {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "entrées du catalogue absentes de l'outil : {extra:?}"
    );
    assert_eq!(
        catalogue.oracle_version, "v0.8.3",
        "version de l'oracle épinglée dans le catalogue"
    );
}

#[test]
#[ignore = "exige KUBE_LINTER_BIN (kube-linter v0.8.3)"]
fn oracle_verdicts_match_the_tool_on_the_fixtures() {
    let bin = kube_linter_bin();
    let mut mismatches = Vec::new();
    for name in ported_names() {
        let dir = fixtures_dir().join(&name);
        let reports = tool_reports(&bin, &dir.join("manifests.yaml"), Some(&name));
        let got = tool_keys(&reports);
        let want = expected_keys(&dir.join("expected.json"));
        if got != want {
            mismatches.push(format!("{name}: outil {got:?}, expected.json {want:?}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "expected.json périmé :\n{}",
        mismatches.join("\n")
    );
}

/// Régénère `expected.json` depuis l'outil. Action explicite : `RENDER_LINT_REGENERATE=1`.
#[test]
#[ignore = "régénération explicite : RENDER_LINT_REGENERATE=1 et KUBE_LINTER_BIN"]
fn regenerate_expected_verdicts() {
    if std::env::var(ENV_REGENERATE).as_deref() != Ok("1") {
        return;
    }
    let bin = kube_linter_bin();
    for name in ported_names() {
        let dir = fixtures_dir().join(&name);
        let reports = tool_reports(&bin, &dir.join("manifests.yaml"), Some(&name));
        let items: Vec<Value> = tool_keys(&reports)
            .into_iter()
            .map(|(rule, kind, namespace, name)| {
                json!({
                    "rule": rule,
                    "kind": kind,
                    "namespace": if namespace.is_empty() { Value::Null } else { json!(namespace) },
                    "name": name,
                })
            })
            .collect();
        let text = serde_json::to_string_pretty(&items).expect("JSON");
        fs::write(dir.join("expected.json"), format!("{text}\n")).expect("écriture expected.json");
    }
}
