use clap::{Args, ValueEnum};
use client::testing::{FixStatus, FixStep, TestHandler};
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
    /// Apply the distribution's fixes to package.yaml (requires --lint-dir)
    #[arg(long = "fix", requires = "lint_dir")]
    fix: bool,
    /// Show the package.yaml that --fix would write, without writing it (requires --fix)
    #[arg(long = "dry-run", requires = "fix")]
    dry_run: bool,
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

/// Lit `<package_dir>/package.yaml`, applique les fix du lint et rend le texte de stderr. Le
/// fichier n'est réécrit que si un fix s'est appliqué, et jamais en `dry_run`.
fn apply_fixes(handler: &TestHandler, package_dir: &Path, dry_run: bool) -> Result<String> {
    let path = package_dir.join("package.yaml");
    let text = fs::read_to_string(&path).map_err(Error::Stdio)?;
    let doc: YamlDoc = text
        .parse()
        .map_err(|e| Error::YamlError(format!("{}: {e}", path.display())))?;
    let report = handler.run_fixes(&doc);
    if !report.changed {
        return Ok(fix_output(&report.steps, None));
    }
    let yaml = report.package.to_yaml_string().map_err(Error::YamlError)?;
    if dry_run {
        return Ok(fix_output(&report.steps, Some(&yaml)));
    }
    fs::write(&path, &yaml).map_err(Error::Stdio)?;
    Ok(fix_output(&report.steps, None))
}

pub fn run(args: &Parameters) -> Result<()> {
    if !args.package_dir.join("tests").is_dir() {
        return Err(Error::MissingTestDirectory(args.package_dir.clone()));
    }
    let mut handler = TestHandler::new(
        args.package_dir.clone(),
        args.script_dir.clone(),
        args.config_dir.clone(),
        args.template_dir.clone(),
        args.testset_dir
            .clone()
            .map(|testset_dir| Vec::from(&[testset_dir])),
    )?;
    if let Some(lint_dir) = &args.lint_dir {
        handler = handler.with_lint(lint_dir, args.schemas_dir.as_deref())?;
    }

    if args.start_all {
        handler.run_all_tests();
        handler.run_suite_lint();
        println!("{}", format_results(&handler.results, &args.format));
        if let Some(output) = args.junit_output_filename.clone() {
            fs::write(output, handler.results.to_junit()).map_err(Error::Stdio)?;
        }
    } else if let Some(test_name) = args.test_name.clone() {
        let created_objects: Arc<Mutex<Vec<Dynamic>>> = Arc::default();
        handler.run_test(&test_name, &created_objects);
        handler.run_suite_lint();
        println!("{}", format_results(&handler.results, &args.format));
        if let Some(output) = args.junit_output_filename.clone() {
            fs::write(output, handler.results.to_junit()).map_err(Error::Stdio)?;
        }
        if let Some(output) = args.template_output_filename.clone() {
            let objets = created_objects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let content = yaml_all_serialize_to_string(&objets)?;
            fs::write(output, content).map_err(Error::Stdio)?;
        }
    } else {
        for t in handler.list_tests() {
            println!("{t}");
        }
        return Ok(());
    }

    if args.fix {
        eprint!("{}", apply_fixes(&handler, &args.package_dir, args.dry_run)?);
    }

    if !handler.results.all_passed() {
        return Err(common::Error::TestFailure(
            handler.results.total_failed(),
            handler.results.total_asserts(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OutputFormat, Parameters, format_results, run};
    use clap::Parser;
    use client::testing::TestHandler;
    use super::{Parameters, fix_output, run};
    use clap::Parser;
    use client::testing::{FixStatus, FixStep};
    use common::Error;
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
}
