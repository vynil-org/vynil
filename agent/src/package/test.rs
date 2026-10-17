use clap::{Args, ValueEnum};
use client::testing::TestHandler;
use common::{Error, Result, rhaihandler::Dynamic, yamlhandler::yaml_all_serialize_to_string};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
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
}

fn format_results(results: &client::testing::TestResultCollector, format: &OutputFormat) -> String {
    match format {
        OutputFormat::Json => results.to_json(),
        OutputFormat::Text => results.to_text(),
    }
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
        let root = tempfile::tempdir().expect("répertoire temporaire");
        let package = root.path().join("package");
        let scripts = root.path().join("agent-scripts");
        let lint_dir = root.path().join("distro");
        fs::create_dir_all(package.join("scripts")).expect("scripts");
        fs::create_dir_all(package.join("tests")).expect("tests");
        fs::create_dir_all(scripts.join("lib")).expect("lib");
        fs::create_dir_all(&lint_dir).expect("distro");
        fs::write(package.join("package.yaml"), PACKAGE_YAML).expect("package.yaml");
        fs::write(
            package.join("scripts/context.rhai"),
            "fn run(instance, args) { #{} }",
        )
        .expect("context");
        fs::write(package.join("scripts/install.rhai"), INSTALL_ONE_CONFIGMAP).expect("install");
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
}
