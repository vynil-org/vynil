use clap::{Args, ValueEnum};
use client::testing::{FixStatus, FixStep, HintOutcome, HintStep, TestHandler};
use common::{
    Error, Result,
    rhaihandler::Dynamic,
    yamlhandler::{YamlDoc, yaml_all_serialize_to_string},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(ValueEnum, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[value(name = "text")]
    Text,
    #[value(name = "json")]
    Json,
}

#[derive(clap::ValueEnum, Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageType {
    /// Tenant package type
    Tenant,
    /// Service package type
    Service,
    #[default]
    /// System package type
    System,
}

/// Options de correction, aplaties dans [`Parameters`] : la ligne de commande garde ses noms et
/// ses prérequis. Le regroupement tient `Parameters` sous le seuil de `struct_excessive_bools`.
#[derive(Args, Debug, Serialize, Deserialize)]
struct FixArgs {
    /// Apply the distribution's fixes to package.yaml (requires --lint-dir)
    #[arg(long = "fix", requires = "lint_dir")]
    fix: bool,
    /// Show the package.yaml that --fix would write, without writing it (requires --fix)
    #[arg(long = "dry-run", requires = "fix")]
    dry_run: bool,
    /// Maximum number of runs of the suite, each one followed by its fixes (requires --fix)
    #[arg(
        long = "fix-rounds",
        default_value_t = 1,
        value_parser = clap::value_parser!(u32).range(1..),
        requires = "fix"
    )]
    fix_rounds: u32,
}

#[derive(Args, Debug, Serialize, Deserialize)]
pub struct Parameters {
    /// Package directory
    #[arg(
        short = 'p',
        long = "package-dir",
        env = "PACKAGE_DIRECTORY",
        value_name = "PACKAGE_DIRECTORY",
        default_value = "/tmp/package"
    )]
    package_dir: PathBuf,
    /// Agent script directory
    #[arg(
        short = 's',
        long = "script-dir",
        env = "SCRIPT_DIRECTORY",
        value_name = "SCRIPT_DIRECTORY",
        default_value = "./agent/scripts"
    )]
    script_dir: PathBuf,
    /// Agent template directory
    #[arg(
        short = 't',
        long = "template-dir",
        env = "TEMPLATE_DIRECTORY",
        value_name = "TEMPLATE_DIRECTORY",
        default_value = "./agent/templates"
    )]
    template_dir: PathBuf,
    /// Configuration directory
    #[arg(
        short = 'c',
        long = "config-dir",
        env = "CONFIG_DIR",
        value_name = "CONFIG_DIR",
        default_value = "."
    )]
    config_dir: PathBuf,
    /// testset additional directory (if any)
    #[arg(
        long = "testsets-dir",
        env = "TESTSETS_DIRECTORY",
        value_name = "TESTSETS_DIRECTORY"
    )]
    testset_dir: Option<PathBuf>,
    /// test-name
    #[arg(long = "test-name", env = "TEST_NAME", value_name = "TEST_NAME")]
    test_name: Option<String>,
    /// Start all tests
    #[arg(long = "all")]
    start_all: bool,
    /// Output format
    #[arg(long = "format", default_value = "text")]
    format: OutputFormat,
    /// junit output filename
    #[arg(
        long = "junit-output-filename",
        env = "JUNIT_OUTPUT_FILENAME",
        value_name = "JUNIT_OUTPUT_FILENAME"
    )]
    junit_output_filename: Option<PathBuf>,
    /// Template output filename (only available for a single test)
    #[arg(
        long = "template-output-filename",
        env = "TEMPLATE_OUTPUT_FILENAME",
        value_name = "TEMPLATE_OUTPUT_FILENAME"
    )]
    template_output_filename: Option<PathBuf>,
    /// Lint directory of a distribution (must contain lint.rhai)
    #[arg(long = "lint-dir", env = "LINT_DIRECTORY", value_name = "LINT_DIRECTORY")]
    lint_dir: Option<PathBuf>,
    /// Schemas directory of a distribution (requires --lint-dir)
    #[arg(
        long = "schemas-dir",
        env = "SCHEMAS_DIRECTORY",
        value_name = "SCHEMAS_DIRECTORY",
        requires = "lint_dir"
    )]
    schemas_dir: Option<PathBuf>,
    /// Fix options (--fix, --dry-run, --fix-rounds)
    #[command(flatten)]
    #[serde(flatten)]
    fixing: FixArgs,
    /// Write a hint for each rule that still has findings (requires --lint-dir)
    #[arg(long = "verbose", requires = "lint_dir")]
    verbose: bool,
}

fn format_results(results: &client::testing::TestResultCollector, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Json => results.to_json(),
        OutputFormat::Text => results.to_text(),
    }
}

/// Une ligne par étape (`fix {règle}: {statut}`, suivie de ` — {message}` si non vide), puis le
/// document `dry_run` si on en a un. Rend le texte destiné à stderr.
fn fix_output(steps: &[FixStep], document: Option<&str>) -> String {
    let mut lines: Vec<String> = steps
        .iter()
        .map(|step| {
            let status = match step.status {
                FixStatus::Applied => "applied",
                FixStatus::NotApplicable => "not applicable",
                FixStatus::Failed => "failed",
            };
            if step.message.is_empty() {
                format!("fix {}: {status}", step.rule)
            } else {
                format!("fix {}: {status} — {}", step.rule, step.message)
            }
        })
        .collect();
    if let Some(doc) = document {
        lines.push(doc.trim_end().to_string());
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Une ligne par règle ayant un hint : `hint {règle}:` puis le texte, chaque ligne indentée de
/// deux espaces ; `hint {règle}: failed — {message}` pour un échec ; rien pour une règle sans hint.
fn hint_output(steps: &[HintStep]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for step in steps {
        match &step.outcome {
            HintOutcome::Text(text) => {
                lines.push(format!("hint {}:", step.rule));
                lines.extend(text.trim_end().lines().map(|line| format!("  {line}")));
            }
            HintOutcome::Failed(message) => {
                lines.push(format!("hint {}: failed — {message}", step.rule));
            }
            HintOutcome::None => {}
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Un tour de correction : les étapes de `run_fixes`, le document qu'elles laissent, et le YAML
/// à écrire, présent seulement si un fix a changé le document.
struct FixRound {
    steps: Vec<FixStep>,
    document: YamlDoc,
    yaml: Option<String>,
    /// Texte de `package.yaml` avant ce tour, pour le restaurer si le tour suivant n'a pas progressé.
    original: String,
}

/// Une exécution de la suite : son moteur neuf et le collecteur des objets créés du test unique.
struct Round {
    handler: TestHandler,
    created_objects: Arc<Mutex<Vec<Dynamic>>>,
}

/// Texte de `package.yaml` et son document.
fn read_package_text(package_dir: &Path) -> Result<(String, YamlDoc)> {
    let path = package_dir.join("package.yaml");
    let text = fs::read_to_string(&path).map_err(Error::Stdio)?;
    let doc = text
        .parse::<YamlDoc>()
        .map_err(|e| Error::YamlError(format!("{}: {e}", path.display())))?;
    Ok((text, doc))
}

fn read_package(package_dir: &Path) -> Result<YamlDoc> {
    read_package_text(package_dir).map(|(_, doc)| doc)
}

/// Vrai si le premier contenu du texte (hors commentaires et lignes vides) est le marqueur `---`.
fn has_document_marker(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .is_some_and(|line| line == "---" || line.starts_with("--- "))
}

/// `yaml` précédé du marqueur `---` quand `original` en avait un : la sérialisation n'en émet pas.
fn keep_document_marker(original: &str, yaml: String) -> String {
    if has_document_marker(original) && !yaml.starts_with("---") {
        format!("---\n{yaml}")
    } else {
        yaml
    }
}

/// Écrit `content` dans `path` sans jamais laisser un fichier tronqué : fichier temporaire du
/// même répertoire, puis `rename`.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    // Un lien symbolique est suivi : c'est sa cible qu'on remplace, le lien reste en place.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let tmp = target.with_extension("yaml.tmp");
    let write = || -> std::io::Result<()> {
        fs::write(&tmp, content)?;
        if let Ok(meta) = fs::metadata(&target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &target)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::Stdio(e)
    })
}

/// Moteur neuf : relit `package.yaml` et réinstalle le lint quand il est demandé.
fn build_handler(args: &Parameters) -> Result<TestHandler> {
    let handler = TestHandler::new(
        args.package_dir.clone(),
        args.script_dir.clone(),
        args.config_dir.clone(),
        args.template_dir.clone(),
        args.testset_dir
            .clone()
            .map(|testset_dir| Vec::from(&[testset_dir])),
    )?;
    match &args.lint_dir {
        Some(lint_dir) => handler.with_lint(lint_dir, args.schemas_dir.as_deref()),
        None => Ok(handler),
    }
}

/// Exécute la suite (couloir all ou mono-test) avec un moteur neuf, puis le hook de suite.
fn run_round(args: &Parameters) -> Result<Round> {
    let mut handler = build_handler(args)?;
    let created_objects: Arc<Mutex<Vec<Dynamic>>> = Arc::default();
    if args.start_all {
        handler.run_all_tests();
    } else if let Some(test_name) = &args.test_name {
        handler.run_test(test_name, &created_objects);
    }
    handler.run_suite_lint();
    Ok(Round {
        handler,
        created_objects,
    })
}

/// Lit `package.yaml`, applique les fix du lint et prépare le YAML à écrire si un fix a changé
/// le document. N'écrit rien.
fn fix_round(handler: &TestHandler, package_dir: &Path) -> Result<FixRound> {
    let (original, doc) = read_package_text(package_dir)?;
    let report = handler.run_fixes(&doc);
    let yaml = if report.changed {
        let yaml = report.package.to_yaml_string().map_err(Error::YamlError)?;
        Some(keep_document_marker(&original, yaml))
    } else {
        None
    };
    Ok(FixRound {
        steps: report.steps,
        document: report.package,
        yaml,
        original,
    })
}

/// Écrit `package.yaml` si un fix a changé le document, sauf en `dry_run`.
fn write_fixes(args: &Parameters, fixes: &FixRound) -> Result<()> {
    if let (Some(yaml), false) = (&fixes.yaml, args.fixing.dry_run) {
        write_atomic(&args.package_dir.join("package.yaml"), yaml)?;
    }
    Ok(())
}

/// Texte de stderr des étapes ; le document n'y figure qu'en `dry_run`, quand il serait écrit.
fn fix_stderr(fixes: &FixRound, dry_run: bool) -> String {
    let shown = if dry_run { fixes.yaml.as_deref() } else { None };
    fix_output(&fixes.steps, shown)
}

/// Règles dont l'étape de fix est appliquée : elles ne reçoivent pas de hint.
fn applied_rules(fixes: &FixRound) -> Vec<String> {
    fixes
        .steps
        .iter()
        .filter(|step| matches!(step.status, FixStatus::Applied))
        .map(|step| step.rule.clone())
        .collect()
}

/// Dernière exécution : résumé, `JUnit` et dump de template (une seule fois), fix, hints, puis le
/// verdict de cette exécution. `fixes` vaut `None` sans `--fix` ou après un arrêt sans progrès.
fn report_last_round(args: &Parameters, last: &Round, fixes: Option<&FixRound>) -> Result<()> {
    let results = &last.handler.results;
    println!("{}", format_results(results, &args.format));
    if let Some(output) = args.junit_output_filename.clone() {
        fs::write(output, results.to_junit()).map_err(Error::Stdio)?;
    }
    if let Some(output) = args.template_output_filename.clone().filter(|_| !args.start_all) {
        let objets = last
            .created_objects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let content = yaml_all_serialize_to_string(&objets)?;
        fs::write(output, content).map_err(Error::Stdio)?;
    }
    if let Some(fixes) = fixes {
        write_fixes(args, fixes)?;
        eprint!("{}", fix_stderr(fixes, args.fixing.dry_run));
    }
    if args.verbose {
        let document = match fixes {
            Some(fixes) => fixes.document.clone(),
            None => read_package(&args.package_dir)?,
        };
        let skip = fixes.map(applied_rules).unwrap_or_default();
        eprint!("{}", hint_output(&last.handler.run_hints(&document, &skip)));
    }
    if !results.all_passed() {
        return Err(Error::TestFailure(
            results.total_failed(),
            results.total_asserts(),
        ));
    }
    Ok(())
}

pub fn run(args: &Parameters) -> Result<()> {
    if !args.package_dir.join("tests").is_dir() {
        return Err(Error::MissingTestDirectory(args.package_dir.clone()));
    }
    if !args.start_all && args.test_name.is_none() {
        for t in build_handler(args)?.list_tests() {
            println!("{t}");
        }
        return Ok(());
    }

    let mut round: u32 = 1;
    let mut previous: Option<(Round, FixRound)> = None;
    loop {
        let current = run_round(args)?;
        let findings = current.handler.finding_count();
        if args.fixing.fix {
            eprintln!("round {round}: {findings} finding(s)");
        }
        if let Some((kept, written)) = previous
            .take()
            .filter(|(kept, _)| findings >= kept.handler.finding_count())
        {
            // Le fix écrit au tour précédent n'a rien amélioré : on rend le fichier tel qu'il
            // était avant lui, et on rapporte le verdict de ce tour-là.
            write_atomic(&args.package_dir.join("package.yaml"), &written.original)?;
            eprintln!(
                "stop: no progress, package.yaml restored to its state before round {}",
                round.saturating_sub(1)
            );
            return report_last_round(args, &kept, None);
        }
        if !args.fixing.fix {
            return report_last_round(args, &current, None);
        }
        let fixes = fix_round(&current.handler, &args.package_dir)?;
        if fixes.yaml.is_none() || args.fixing.dry_run || round >= args.fixing.fix_rounds {
            return report_last_round(args, &current, Some(&fixes));
        }
        write_fixes(args, &fixes)?;
        eprint!("{}", fix_stderr(&fixes, false));
        previous = Some((current, fixes));
        round = round.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{OutputFormat, Parameters, fix_output, format_results, hint_output, run};
    use clap::Parser;
    use client::testing::{FixStatus, FixStep, HintOutcome, HintStep, TestHandler};
    use common::{Error, yamlhandler::YamlDoc};
    use std::{fs, path::PathBuf};

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Parameters,
    }

    const PACKAGE_YAML: &str = "apiVersion: vinyl.solidite.fr/v1beta1\nkind: Package\nmetadata:\n  name: demo\n  category: core\n  description: demo package\n  type: tenant\n  features: []\nrequirements: []\n";

    const INSTALL_ONE_CONFIGMAP: &str = r#"fn run(instance, context) {
    let h = k8s_resource("configmaps", "app");
    h.create(#{ apiVersion: "v1", kind: "ConfigMap", metadata: #{ name: "web", namespace: "app" } });
}
"#;

    const LINT_ERROR: &str = r#"fn lint(objs, ctx) {
    [#{
        rule: "no-web", severity: "error",
        object: #{ apiVersion: "v1", kind: "ConfigMap", namespace: "app", name: "web" },
        message: "web is forbidden"
    }]
}
"#;

    /// Package avec un test sans asserts, et une distribution de lint optionnelle.
    struct Tree {
        _root: tempfile::TempDir,
        package: PathBuf,
        scripts: PathBuf,
        lint: PathBuf,
    }

    fn tree(lint: Option<&str>) -> Tree {
        tree_with(lint, INSTALL_ONE_CONFIGMAP, PACKAGE_YAML)
    }

    /// Package avec l'installation et le `package.yaml` donnés.
    fn tree_with(lint: Option<&str>, install: &str, package_yaml: &str) -> Tree {
        let root = tempfile::tempdir().expect("répertoire temporaire");
        let package = root.path().join("package");
        let scripts = root.path().join("agent-scripts");
        let lint_dir = root.path().join("distro");
        fs::create_dir_all(package.join("scripts")).expect("scripts");
        fs::create_dir_all(package.join("tests")).expect("tests");
        fs::create_dir_all(scripts.join("lib")).expect("lib");
        fs::create_dir_all(&lint_dir).expect("distro");
        fs::write(package.join("package.yaml"), package_yaml).expect("package.yaml");
        fs::write(
            package.join("scripts/context.rhai"),
            "fn run(instance, args) { #{} }",
        )
        .expect("context");
        fs::write(package.join("scripts/install.rhai"), install).expect("install");
        fs::write(
            package.join("tests/default.yaml"),
            "apiVersion: vinyl.solidite.fr/v1beta1\nkind: Test\nmetadata:\n  name: default\ninstance:\n  name: demo\n  namespace: app\n",
        )
        .expect("test");
        if let Some(src) = lint {
            fs::write(lint_dir.join("lint.rhai"), src).expect("lint.rhai");
        }
        Tree {
            _root: root,
            package,
            scripts,
            lint: lint_dir,
        }
    }

    /// Arguments CLI communs ; `extra` ajoute les options du cas testé.
    fn params(tree: &Tree, extra: &[&str]) -> Parameters {
        let mut argv = vec![
            "test".to_string(),
            "-p".to_string(),
            tree.package.display().to_string(),
            "-s".to_string(),
            tree.scripts.display().to_string(),
            "-c".to_string(),
            tree.package.display().to_string(),
        ];
        argv.extend(extra.iter().map(ToString::to_string));
        Cli::try_parse_from(argv).expect("arguments valides").args
    }

    #[test]
    fn schemas_dir_requires_lint_dir() {
        let argv = ["test", "-p", "/pkg", "--schemas-dir", "/schemas"];
        assert!(Cli::try_parse_from(argv).is_err());
    }

    #[test]
    fn missing_lint_script_dies_before_output() {
        let t = tree(None);
        let args = params(&t, &["--all", "--lint-dir", &t.lint.display().to_string()]);
        let err = run(&args).expect_err("sans lint.rhai, le lint échoue avant tout affichage");
        assert!(!matches!(err, Error::TestFailure(..)), "{err}");
        assert!(err.to_string().contains("lint.rhai"), "{err}");
    }

    #[test]
    fn lint_dir_turns_findings_into_failed_asserts() {
        let t = tree(Some(LINT_ERROR));
        let args = params(&t, &["--all", "--lint-dir", &t.lint.display().to_string()]);
        let err = run(&args).expect_err("un constat error fait échouer la suite");
        assert!(matches!(err, Error::TestFailure(1, 1)), "{err}");
    }

    /// Sorties texte, JSON et `JUnit` d'un handler sans lint, après `--all`.
    fn outputs_without_lint(t: &Tree) -> (String, String, String) {
        let args = params(t, &["--all"]);
        let mut handler = TestHandler::new(
            args.package_dir.clone(),
            args.script_dir.clone(),
            args.config_dir.clone(),
            args.template_dir,
            None,
        )
        .expect("chargement du handler");
        handler.run_all_tests();
        handler.run_suite_lint();
        (
            format_results(&handler.results, &OutputFormat::Text),
            format_results(&handler.results, &OutputFormat::Json),
            handler.results.to_junit(),
        )
    }

    /// Remplace par `<variable>` durées, horodatage et nom d'hôte, qui varient d'une exécution
    /// à l'autre ; le reste de la sortie est comparé octet pour octet.
    fn neutralise(out: &str) -> String {
        out.split_inclusive('\n')
            .map(|piece| {
                piece.strip_suffix('\n').map_or_else(
                    || neutralise_line(piece),
                    |line| format!("{}\n", neutralise_line(line)),
                )
            })
            .collect()
    }

    fn neutralise_line(line: &str) -> String {
        if let Some(rest) = line.strip_prefix("Test: ") {
            let name = rest.split(" (").next().unwrap_or(rest);
            return format!("Test: {name} (<variable>)");
        }
        let mut line = line.to_string();
        if let Some(at) = line.find("\"duration\": ") {
            line.truncate(at + "\"duration\": ".len());
            line.push_str("<variable>");
        }
        for attr in ["hostname", "timestamp", "time"] {
            let marker = format!("{attr}=\"");
            if let Some(start) = line.find(&marker) {
                let value = start + marker.len();
                if let Some(len) = line[value..].find('"') {
                    line.replace_range(value..value + len, "<variable>");
                }
            }
        }
        line
    }

    #[test]
    fn no_lint_dir_leaves_behaviour_unchanged() {
        let t = tree(Some(LINT_ERROR));
        let (text, json, junit) = outputs_without_lint(&t);
        assert_eq!(
            neutralise(&text),
            "Test: default (<variable>)\n\nResults: 0 passed, 0 failed, 0 total\n"
        );
        assert_eq!(
            neutralise(&json),
            r#"{
  "tests": [
    {
      "test_name": "default",
      "asserts": [],
      "duration": <variable>
    }
  ],
  "summary": {
    "total_tests": 1,
    "total_asserts": 0,
    "passed": 0,
    "failed": 0
  }
}"#
        );
        assert_eq!(
            neutralise(&junit),
            r#"<?xml version="1.0" encoding="utf-8"?><testsuites><testsuite id="0" name="default" package="testsuite/default" tests="0" errors="0" failures="0" hostname="<variable>" timestamp="<variable>" time="<variable>"/></testsuites>"#
        );
    }

    /// Installation qui crée un `Deployment` `app/web` dont le conteneur a une limite CPU.
    const INSTALL_DEPLOYMENT_WITH_CPU: &str = r#"fn run(instance, context) {
    let h = k8s_resource("deployments", "app");
    h.create(#{
        apiVersion: "apps/v1", kind: "Deployment",
        metadata: #{ name: "web", namespace: "app" },
        spec: #{ template: #{ spec: #{ containers: [#{ name: "app", resources: #{ limits: #{ cpu: "500m" } } }] } } }
    });
}
"#;

    /// `package.yaml` aux clés non triées : `zeta` précède `alpha`, et `limits.memory` existe.
    const PACKAGE_WITH_CPU_LIMITS: &str = "apiVersion: vinyl.solidite.fr/v1beta1\nkind: Package\nmetadata:\n  name: demo\n  category: core\n  description: demo package\n  type: tenant\n  features: []\nrequirements: []\nresources:\n  zeta:\n    limits:\n      memory: 256Mi\n      cpu: 500m\n  alpha:\n    limits:\n      cpu: 100m\n";

    /// `package.yaml` commenté dont aucune limite CPU n'est à retirer.
    const PACKAGE_COMMENTED_NO_CPU: &str = "# commentaire de tête, perdu par un aller-retour YamlDoc\napiVersion: vinyl.solidite.fr/v1beta1\nkind: Package\nmetadata:\n  name: demo  # nom du paquet\n  category: core\n  description: demo package\n  type: tenant\n  features: []\nrequirements: []\nresources:\n  app:\n    limits:\n      memory: 128Mi\n";

    /// Distribution : `lint` signale chaque `Deployment` dont un conteneur a une limite CPU ;
    /// `fix` retire toutes les `resources.*.limits.cpu`, et le bloc `limits` s'il devient vide.
    const NO_CPU_LIMITS_DISTRO: &str = r#"fn lint(objs, ctx) {
    let out = [];
    for obj in objs.all {
        if obj.kind == "Deployment" {
            for c in obj.spec.template.spec.containers {
                if "resources" in c && "limits" in c.resources && "cpu" in c.resources.limits {
                    out.push(#{
                        rule: "no-cpu-limits", severity: "error",
                        object: #{ apiVersion: obj.apiVersion, kind: obj.kind, namespace: obj.metadata.namespace, name: obj.metadata.name },
                        message: "cpu limit on " + c.name
                    });
                }
            }
        }
    }
    out
}

fn fix(rule, findings, pkg) {
    let resources = pkg["resources"];
    if type_of(resources) != "map" { return #{ status: "not_applicable" }; }
    let changed = false;
    for name in resources.keys() {
        let component = resources[name];
        let limits = component["limits"];
        if type_of(limits) == "map" && limits.keys().contains("cpu") {
            limits.remove("cpu");
            if limits.keys().len() == 0 {
                component.remove("limits");
            } else {
                component["limits"] = limits;
            }
            resources[name] = component;
            changed = true;
        }
    }
    if !changed { return #{ status: "not_applicable" }; }
    pkg["resources"] = resources;
    #{ status: "applied", pkg: pkg }
}
"#;

    /// Distribution dont le constat vit dans `package.yaml` : `lint` signale `no-cpu-limits` tant
    /// que `resources` garde une limite CPU (`ctx.package`) ; `fix` la retire comme
    /// `NO_CPU_LIMITS_DISTRO`.
    const CPU_FROM_PACKAGE_DISTRO: &str = r#"fn lint(objs, ctx) {
    let out = [];
    let resources = ctx["package"]["resources"];
    for name in resources.keys() {
        let limits = resources[name]["limits"];
        if type_of(limits) == "map" && type_of(limits["cpu"]) == "string" {
            out.push(#{
                rule: "no-cpu-limits", severity: "error",
                object: #{ apiVersion: "v1", kind: "ConfigMap", namespace: "app", name: "web" },
                message: "cpu limit in " + name
            });
        }
    }
    out
}

fn fix(rule, findings, pkg) {
    let resources = pkg["resources"];
    if type_of(resources) != "map" { return #{ status: "not_applicable" }; }
    let changed = false;
    for name in resources.keys() {
        let component = resources[name];
        let limits = component["limits"];
        if type_of(limits) == "map" && limits.keys().contains("cpu") {
            limits.remove("cpu");
            if limits.keys().len() == 0 {
                component.remove("limits");
            } else {
                component["limits"] = limits;
            }
            resources[name] = component;
            changed = true;
        }
    }
    if !changed { return #{ status: "not_applicable" }; }
    pkg["resources"] = resources;
    #{ status: "applied", pkg: pkg }
}
"#;

    /// Distribution dont le constat ne disparaît jamais : `fix` ajoute une marque à `marks` à
    /// chaque appel, et `lint` signale toujours la même règle.
    const ALWAYS_FINDING_DISTRO: &str = r#"fn lint(objs, ctx) {
    [#{
        rule: "always", severity: "error",
        object: #{ apiVersion: "v1", kind: "ConfigMap", namespace: "app", name: "web" },
        message: "toujours là"
    }]
}

fn fix(rule, findings, pkg) {
    let marks = pkg["marks"];
    if type_of(marks) != "array" { marks = []; }
    marks.push("marque-tour");
    pkg["marks"] = marks;
    #{ status: "applied", pkg: pkg }
}
"#;

    /// Distribution dont `hint` répond pour la seule règle `no-web`.
    const HINT_DISTRO: &str = r#"fn lint(objs, ctx) {
    [#{
        rule: "no-web", severity: "error",
        object: #{ apiVersion: "v1", kind: "ConfigMap", namespace: "app", name: "web" },
        message: "web is forbidden"
    }]
}

fn hint(rule, findings, pkg) {
    "retirer la ConfigMap web\nou la renommer"
}
"#;

    fn read(path: &std::path::Path) -> String {
        fs::read_to_string(path).expect("lecture de package.yaml")
    }

    #[test]
    fn fix_rewrites_package_yaml() {
        let t = tree_with(
            Some(NO_CPU_LIMITS_DISTRO),
            INSTALL_DEPLOYMENT_WITH_CPU,
            PACKAGE_WITH_CPU_LIMITS,
        );
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
        ]);
        let err = run(&args).expect_err("le constat valait un échec avant le fix");
        assert!(matches!(err, Error::TestFailure(1, 1)), "{err}");
        let text = read(&t.package.join("package.yaml"));
        assert!(!text.contains("cpu"), "{text}");
        assert!(text.contains("memory: 256Mi"), "{text}");
        assert!(
            text.find("zeta") < text.find("alpha"),
            "l'ordre des clés est conservé : {text}"
        );
        assert!(text.find("apiVersion") < text.find("resources"), "{text}");
    }

    #[test]
    fn dry_run_leaves_package_yaml_untouched() {
        let t = tree_with(
            Some(NO_CPU_LIMITS_DISTRO),
            INSTALL_DEPLOYMENT_WITH_CPU,
            PACKAGE_WITH_CPU_LIMITS,
        );
        let before = read(&t.package.join("package.yaml"));
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
            "--dry-run",
        ]);
        let err = run(&args).expect_err("le verdict ne change pas avec --dry-run");
        assert!(matches!(err, Error::TestFailure(1, 1)), "{err}");
        assert_eq!(read(&t.package.join("package.yaml")), before);
    }

    #[test]
    fn nothing_to_fix_leaves_the_file_byte_identical() {
        let t = tree_with(
            Some(NO_CPU_LIMITS_DISTRO),
            INSTALL_DEPLOYMENT_WITH_CPU,
            PACKAGE_COMMENTED_NO_CPU,
        );
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
        ]);
        let err = run(&args).expect_err("le constat reste un échec");
        assert!(matches!(err, Error::TestFailure(1, 1)), "{err}");
        assert_eq!(read(&t.package.join("package.yaml")), PACKAGE_COMMENTED_NO_CPU);
    }

    #[test]
    fn fix_and_dry_run_need_their_prerequisites() {
        assert!(Cli::try_parse_from(["test", "-p", "/pkg", "--fix"]).is_err());
        assert!(Cli::try_parse_from(["test", "-p", "/pkg", "--dry-run", "--lint-dir", "/d"]).is_err());
        assert!(Cli::try_parse_from(["test", "-p", "/pkg", "--fix", "--lint-dir", "/d"]).is_ok());
        assert!(
            Cli::try_parse_from(["test", "-p", "/pkg", "--fix", "--dry-run", "--lint-dir", "/d"]).is_ok()
        );
    }

    #[test]
    fn no_fix_flag_never_writes_package_yaml() {
        let t = tree_with(
            Some(NO_CPU_LIMITS_DISTRO),
            INSTALL_DEPLOYMENT_WITH_CPU,
            PACKAGE_WITH_CPU_LIMITS,
        );
        let before = read(&t.package.join("package.yaml"));
        let args = params(&t, &["--all", "--lint-dir", &t.lint.display().to_string()]);
        let err = run(&args).expect_err("le constat échoue sans fix");
        assert!(matches!(err, Error::TestFailure(1, 1)), "{err}");
        assert_eq!(read(&t.package.join("package.yaml")), before);
    }

    #[test]
    fn fix_output_has_one_line_per_step_then_the_document() {
        let steps = [
            FixStep {
                rule: "no-cpu-limits".to_string(),
                status: FixStatus::Applied,
                message: String::new(),
            },
            FixStep {
                rule: "other".to_string(),
                status: FixStatus::NotApplicable,
                message: "rien à retirer".to_string(),
            },
            FixStep {
                rule: "broken".to_string(),
                status: FixStatus::Failed,
                message: "boom".to_string(),
            },
        ];
        let text = fix_output(&steps, None);
        assert_eq!(
            text,
            "fix no-cpu-limits: applied\nfix other: not applicable — rien à retirer\nfix broken: failed — boom\n"
        );
        let with_doc = fix_output(&steps[..1], Some("a: 1\n"));
        assert_eq!(with_doc, "fix no-cpu-limits: applied\na: 1\n");
    }

    #[test]
    fn fix_rounds_rerun_until_the_findings_are_gone() {
        let t = tree_with(
            Some(CPU_FROM_PACKAGE_DISTRO),
            INSTALL_ONE_CONFIGMAP,
            PACKAGE_WITH_CPU_LIMITS,
        );
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
            "--fix-rounds",
            "3",
        ]);
        run(&args).expect("la dernière exécution n'a aucun constat");
        let text = read(&t.package.join("package.yaml"));
        assert!(!text.contains("cpu"), "{text}");
        assert!(text.contains("memory: 256Mi"), "{text}");
    }

    #[test]
    fn fix_rounds_stop_when_there_is_no_progress() {
        let t = tree_with(Some(ALWAYS_FINDING_DISTRO), INSTALL_ONE_CONFIGMAP, PACKAGE_YAML);
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
            "--fix-rounds",
            "5",
        ]);
        let err = run(&args).expect_err("le constat ne disparaît jamais");
        assert!(matches!(err, Error::TestFailure(1, 1)), "{err}");
        let text = read(&t.package.join("package.yaml"));
        assert_eq!(
            text, PACKAGE_YAML,
            "le fix du tour sans progrès est retiré : {text}"
        );
    }

    #[test]
    fn a_fix_returning_the_same_document_leaves_the_file_alone() {
        const SAME_DOCUMENT_DISTRO: &str = r#"fn lint(objs, ctx) {
    [#{
        rule: "always", severity: "error",
        object: #{ apiVersion: "v1", kind: "ConfigMap", namespace: "app", name: "web" },
        message: "toujours là"
    }]
}

fn fix(rule, findings, pkg) { #{ status: "applied", pkg: pkg } }
"#;
        let t = tree_with(
            Some(SAME_DOCUMENT_DISTRO),
            INSTALL_ONE_CONFIGMAP,
            PACKAGE_COMMENTED_NO_CPU,
        );
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
            "--fix-rounds",
            "3",
        ]);
        run(&args).expect_err("le constat reste");
        let text = read(&t.package.join("package.yaml"));
        assert_eq!(text, PACKAGE_COMMENTED_NO_CPU, "commentaires conservés : {text}");
    }

    #[cfg(unix)]
    #[test]
    fn a_rewrite_keeps_the_permissions_of_package_yaml() {
        use std::os::unix::fs::PermissionsExt;
        let t = tree_with(
            Some(NO_CPU_LIMITS_DISTRO),
            INSTALL_DEPLOYMENT_WITH_CPU,
            PACKAGE_WITH_CPU_LIMITS,
        );
        let file = t.package.join("package.yaml");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).expect("chmod");
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
        ]);
        run(&args).expect_err("une seule exécution");
        assert!(!read(&file).contains("cpu"), "le fichier a bien été réécrit");
        let mode = fs::metadata(&file).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_rewrite_follows_a_symlinked_package_yaml() {
        let t = tree_with(
            Some(NO_CPU_LIMITS_DISTRO),
            INSTALL_DEPLOYMENT_WITH_CPU,
            PACKAGE_WITH_CPU_LIMITS,
        );
        let link = t.package.join("package.yaml");
        let real = t.package.join("real-package.yaml");
        fs::rename(&link, &real).expect("déplacement");
        std::os::unix::fs::symlink("real-package.yaml", &link).expect("lien");
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
        ]);
        run(&args).expect_err("une seule exécution");
        let meta = fs::symlink_metadata(&link).expect("stat du lien");
        assert!(meta.file_type().is_symlink(), "package.yaml reste un lien");
        assert!(!read(&real).contains("cpu"), "la cible porte le document réécrit");
    }

    #[test]
    fn the_leading_document_marker_survives_a_rewrite() {
        let original = format!("---\n# en-tête\n{PACKAGE_WITH_CPU_LIMITS}");
        let t = tree_with(Some(CPU_FROM_PACKAGE_DISTRO), INSTALL_ONE_CONFIGMAP, &original);
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
        ]);
        run(&args).expect_err("une seule exécution");
        let text = read(&t.package.join("package.yaml"));
        assert!(text.starts_with("---\n"), "{text}");
        assert!(!text.contains("cpu"), "{text}");
        assert_eq!(text.matches("---").count(), 1, "{text}");
    }

    #[test]
    fn the_default_is_a_single_pass() {
        let t = tree_with(
            Some(CPU_FROM_PACKAGE_DISTRO),
            INSTALL_ONE_CONFIGMAP,
            PACKAGE_WITH_CPU_LIMITS,
        );
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--fix",
        ]);
        let err = run(&args).expect_err("une seule exécution : le verdict reste celui d'avant le fix");
        assert!(matches!(err, Error::TestFailure(2, 2)), "{err}");
        let text = read(&t.package.join("package.yaml"));
        assert!(!text.contains("cpu"), "{text}");
    }

    #[test]
    fn hint_output_has_one_block_per_rule() {
        let steps = [
            HintStep {
                rule: "no-cpu-limits".to_string(),
                outcome: HintOutcome::Text("retirer la limite\nou la rendre plus haute\n".to_string()),
            },
            HintStep {
                rule: "quiet".to_string(),
                outcome: HintOutcome::None,
            },
            HintStep {
                rule: "broken".to_string(),
                outcome: HintOutcome::Failed("boom".to_string()),
            },
        ];
        assert_eq!(
            hint_output(&steps),
            "hint no-cpu-limits:\n  retirer la limite\n  ou la rendre plus haute\nhint broken: failed — boom\n"
        );
        assert_eq!(hint_output(&[]), "");
    }

    #[test]
    fn hints_are_written_for_what_is_left() {
        let t = tree(Some(HINT_DISTRO));
        let args = params(&t, &["--all", "--lint-dir", &t.lint.display().to_string()]);
        let mut handler = TestHandler::new(
            args.package_dir.clone(),
            args.script_dir.clone(),
            args.config_dir.clone(),
            args.template_dir,
            None,
        )
        .expect("moteur")
        .with_lint(&t.lint, None)
        .expect("lint");
        handler.run_all_tests();
        handler.run_suite_lint();
        let doc: YamlDoc = PACKAGE_YAML.parse().expect("package.yaml");
        assert_eq!(
            hint_output(&handler.run_hints(&doc, &[])),
            "hint no-web:\n  retirer la ConfigMap web\n  ou la renommer\n"
        );
        assert_eq!(hint_output(&handler.run_hints(&doc, &["no-web".to_string()])), "");
    }

    #[test]
    fn verbose_does_not_change_the_verdict() {
        let t = tree(Some(HINT_DISTRO));
        let args = params(&t, &[
            "--all",
            "--lint-dir",
            &t.lint.display().to_string(),
            "--verbose",
        ]);
        let err = run(&args).expect_err("le constat échoue ; le hint ne change pas le verdict");
        assert!(matches!(err, Error::TestFailure(1, 1)), "{err}");
    }

    #[test]
    fn fix_rounds_and_verbose_need_their_prerequisites() {
        assert!(Cli::try_parse_from(["test", "-p", "/pkg", "--fix-rounds", "2"]).is_err());
        assert!(Cli::try_parse_from(["test", "-p", "/pkg", "--verbose"]).is_err());
        assert!(
            Cli::try_parse_from([
                "test",
                "-p",
                "/pkg",
                "--fix",
                "--lint-dir",
                "/d",
                "--fix-rounds",
                "0"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "test",
                "-p",
                "/pkg",
                "--fix",
                "--lint-dir",
                "/d",
                "--fix-rounds",
                "2"
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["test", "-p", "/pkg", "--lint-dir", "/d", "--verbose"]).is_ok());
    }
}
