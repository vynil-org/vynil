use super::{
    lint::{
        context::{build_suite_context, build_test_context},
        runner::LintRunner,
        sections::Sections,
    },
    result::{Suppression, TestResultCollector},
    schema::registry::SchemaRegistry,
    vyniltest::{VynilTest, VynilTestSetRef, appslug},
    vyniltestset::{VynilAssert, VynilAssertResult, VynilAssertSelector, VynilTestSet, VynilTestSetMocks},
};
use common::{
    handlebarshandler::HandleBars,
    httpmock::HttpMockItem,
    rhaihandler::{Dynamic, Map, Script},
    vynilpackage::{VynilPackageSource, VynilPackageType, read_package_yaml},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const API_VERSION: &str = "vinyl.solidite.fr/v1beta1";

// ── Templating helpers (private) ────────────────────────────────────────────

/// Recursively templates all string keys and string values inside a `serde_json::Value`.
fn template_json(
    hbs: &mut HandleBars,
    ctx: &serde_json::Value,
    val: serde_json::Value,
) -> common::Result<serde_json::Value> {
    match val {
        serde_json::Value::String(s) => Ok(serde_json::Value::String(hbs.render(&s, ctx)?)),
        serde_json::Value::Array(arr) => Ok(serde_json::Value::Array(
            arr.into_iter()
                .map(|v| template_json(hbs, ctx, v))
                .collect::<common::Result<_>>()?,
        )),
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(hbs.render(&k, ctx)?, template_json(hbs, ctx, v)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        other => Ok(other),
    }
}

/// Templates all strings inside a `rhai::Dynamic` (via JSON round-trip).
fn template_dynamic(hbs: &mut HandleBars, ctx: &serde_json::Value, d: &Dynamic) -> common::Result<Dynamic> {
    let json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(d).map_err(common::Error::SerializationError)?)
            .map_err(common::Error::SerializationError)?;
    let templated = template_json(hbs, ctx, json)?;
    serde_json::from_str(&serde_json::to_string(&templated).map_err(common::Error::SerializationError)?)
        .map_err(common::Error::SerializationError)
}

/// Templates all string keys and values inside a `rhai::Map`.
fn template_rhai_map(hbs: &mut HandleBars, ctx: &serde_json::Value, map: &Map) -> common::Result<Map> {
    let d = template_dynamic(hbs, ctx, &Dynamic::from_map(map.clone()))?;
    d.try_cast::<Map>()
        .ok_or_else(|| common::Error::Other("expected map after templating".into()))
}

fn template_http_mock(
    hbs: &mut HandleBars,
    ctx: &serde_json::Value,
    item: &HttpMockItem,
) -> common::Result<HttpMockItem> {
    Ok(HttpMockItem {
        path: hbs.render(&item.path, ctx)?,
        method: item.method.clone(),
        return_obj: template_rhai_map(hbs, ctx, &item.return_obj)?,
    })
}

fn template_assert(
    hbs: &mut HandleBars,
    ctx: &serde_json::Value,
    a: &VynilAssert,
) -> common::Result<VynilAssert> {
    Ok(VynilAssert {
        name: a.name.clone(),
        description: a.description.clone(),
        selector: VynilAssertSelector {
            kind: a.selector.kind.as_ref().map(|s| hbs.render(s, ctx)).transpose()?,
            name: a.selector.name.as_ref().map(|s| hbs.render(s, ctx)).transpose()?,
            namespace: a
                .selector
                .namespace
                .as_ref()
                .map(|s| hbs.render(s, ctx))
                .transpose()?,
        },
        matcher: a.matcher.clone(),
        value: a.value.clone().map(|v| template_json(hbs, ctx, v)).transpose()?,
    })
}

/// Builds the `var` context for a testSet reference:
/// ref variable values take priority, then testSet defaults.
fn build_var_context(ts: &VynilTestSet, ts_ref: &VynilTestSetRef) -> serde_json::Value {
    let mut var = serde_json::Map::new();
    if let Some(variables) = &ts.variables {
        for (name, def) in variables {
            let value = ts_ref
                .variables
                .as_ref()
                .and_then(|v| v.get(name))
                .cloned()
                .or_else(|| def.default.clone())
                .unwrap_or(serde_json::Value::Null);
            var.insert(name.clone(), value);
        }
    }
    // Include ref variables not declared in the testSet definition
    if let Some(ref_vars) = &ts_ref.variables {
        for (name, value) in ref_vars {
            if !var.contains_key(name) {
                var.insert(name.clone(), value.clone());
            }
        }
    }
    serde_json::Value::Object(var)
}

/// Templates mocks/asserts from a source, appending to the collectors.
fn collect_templated(
    hbs: &mut HandleBars,
    ctx: &serde_json::Value,
    mocks: Option<&VynilTestSetMocks>,
    asserts: Option<&Vec<VynilAssert>>,
    k8s_out: &mut Vec<Dynamic>,
    http_out: &mut Vec<HttpMockItem>,
    asserts_out: &mut Vec<VynilAssert>,
) -> common::Result<()> {
    let mut context = ctx.clone();
    if let Some(obj) = context.as_object_mut() {
        obj.insert("context".into(), ctx.clone());
    }

    if let Some(m) = mocks {
        if let Some(k8s) = &m.kubernetes {
            for d in k8s {
                k8s_out.push(template_dynamic(hbs, &context, d)?);
            }
        }
        if let Some(http) = &m.http {
            for item in http {
                http_out.push(template_http_mock(hbs, &context, item)?);
            }
        }
    }
    if let Some(list) = asserts {
        for a in list {
            asserts_out.push(template_assert(hbs, &context, a)?);
        }
    }
    Ok(())
}

/// True when `k8s_mocks` already contains a mock of `instance_kind` named `instance.name`
/// in `instance.namespace`.
fn instance_already_mocked(
    k8s_mocks: &[Dynamic],
    instance_kind: &str,
    instance: &super::vyniltest::VynilTestInstance,
) -> bool {
    k8s_mocks.iter().any(|m| {
        let Ok(map) = m.as_map_ref() else { return false };
        let kind_ok = map
            .get("kind")
            .and_then(|v| v.clone().into_string().ok())
            .as_deref()
            == Some(instance_kind);
        if !kind_ok {
            return false;
        }
        let Some(meta) = map.get("metadata") else {
            return false;
        };
        let Ok(meta_map) = meta.as_map_ref() else {
            return false;
        };
        let name_ok = meta_map
            .get("name")
            .and_then(|v| v.clone().into_string().ok())
            .as_deref()
            == Some(instance.name.as_str());
        let ns_ok = meta_map
            .get("namespace")
            .and_then(|v| v.clone().into_string().ok())
            .as_deref()
            == Some(instance.namespace.as_str());
        name_ok && ns_ok
    })
}

/// Lit le `status` de l'instance mockée du test, tel que l'installation l'a persisté dans la
/// base de mocks (`fun_name` : `get_<usage>_instance`). L'échec de lecture est rendu à
/// l'appelant : `lint_sections` en fait une ligne `lint:sections`.
fn instance_status(rhai: &mut Script, fun_name: &str) -> Result<Dynamic, String> {
    rhai.eval(&format!("{fun_name}(args.namespace, args.instance).status"))
        .map_err(|e| e.to_string())
}

/// Voie lint de test : rend les lignes et les suppressions du script `lint` sur les objets
/// créés. Sans le hook `lint` (distribution qui ne définit que `lint_suite`), rien n'est lu ni
/// appelé : `read_status` n'est pas invoquée.
fn lint_test_lane(
    lint: &LintRunner,
    created: &[Dynamic],
    ctx: &serde_json::Value,
    read_status: impl FnOnce() -> Result<Dynamic, String>,
) -> (Vec<VynilAssertResult>, Vec<Suppression>) {
    if !lint.has_test_hook() {
        return (Vec::new(), Vec::new());
    }
    let (sections, unreadable) = lint_sections(created, read_status());
    let outcome = lint.run_test(ctx, &sections);
    let mut rows: Vec<VynilAssertResult> = unreadable.into_iter().collect();
    rows.extend(outcome.rows);
    (rows, outcome.suppressions)
}

/// Sections des objets créés d'après le `status` lu. Un statut illisible laisse toutes les
/// sections vides et rend en plus une ligne `lint:sections` en échec : une règle « pour chaque
/// objet de `objs.scalables` » ne passerait sinon vert sans rien avoir vérifié. Un statut lu
/// sans aucune section déclarée reste un cas légitime et ne produit aucune ligne.
fn lint_sections(
    created: &[Dynamic],
    status: Result<Dynamic, String>,
) -> (Sections, Option<VynilAssertResult>) {
    match status {
        Ok(status) => (Sections::from_status(created, &status), None),
        Err(error) => (
            Sections::from_status(created, &Dynamic::UNIT),
            Some(VynilAssertResult {
                name: "lint:sections".to_string(),
                description: Some("Lint sections".to_string()),
                passed: false,
                message: format!("status of the instance not readable, every object is unsectioned: {error}"),
            }),
        ),
    }
}

/// Builds the virtual instance mock (`Dynamic`) for the tested instance.
///
/// # Errors
///
/// Returns a [`common::Error`] when the instance object cannot round-trip through JSON.
fn virtual_instance_dynamic(
    package: &VynilPackageSource,
    instance: &super::vyniltest::VynilTestInstance,
) -> common::Result<Dynamic> {
    let instance_kind = match &package.metadata.usage {
        VynilPackageType::Service => "ServiceInstance",
        VynilPackageType::System => "SystemInstance",
        VynilPackageType::Tenant => "TenantInstance",
    };
    let mut spec = serde_json::Map::new();
    spec.insert(
        "category".to_string(),
        serde_json::Value::String(package.metadata.category.clone()),
    );
    spec.insert(
        "package".to_string(),
        serde_json::Value::String(package.metadata.name.clone()),
    );
    if let Some(opts) = &instance.options {
        spec.insert(
            "options".to_string(),
            serde_json::Value::Object(opts.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        );
    }
    let instance_obj = serde_json::json!({
        "apiVersion": "vynil.solidite.fr/v1",
        "kind": instance_kind,
        "metadata": {
            "name": instance.name,
            "namespace": instance.namespace,
        },
        "spec": serde_json::Value::Object(spec),
        "status": {},
    });
    let text = serde_json::to_string(&instance_obj).map_err(common::Error::SerializationError)?;
    serde_json::from_str(&text).map_err(common::Error::SerializationError)
}

// ── TestHandler ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct TestHandler {
    package_dir: PathBuf,
    script_dir: PathBuf,
    config_dir: PathBuf,
    template_dir: PathBuf,
    package: VynilPackageSource,
    pub results: TestResultCollector,
    tests: BTreeMap<String, VynilTest>,
    test_sets: BTreeMap<String, VynilTestSet>,
    /// Moteur de lint, présent seulement après `with_lint`.
    lint: Option<LintRunner>,
    /// Noms des tests exécutés par cette invocation (contexte de suite).
    executed: Vec<String>,
}

impl TestHandler {
    /// # Errors
    ///
    /// Returns a [`common::Error`] when `package.yaml` cannot be read or when scanning or
    /// parsing the test directories fails.
    pub fn new(
        package_dir: PathBuf,
        script_dir: PathBuf,
        config_dir: PathBuf,
        template_dir: PathBuf,
        testset_dirs: Option<Vec<PathBuf>>,
    ) -> common::Result<Self> {
        let package = read_package_yaml(&package_dir.join("package.yaml"))?;
        let mut handler = Self {
            package_dir,
            script_dir,
            config_dir,
            template_dir,
            package,
            results: TestResultCollector::new(),
            tests: BTreeMap::new(),
            test_sets: BTreeMap::new(),
            lint: None,
            executed: Vec::new(),
        };
        if let Some(ts_dirs) = testset_dirs {
            for dir in &ts_dirs {
                handler.load_testsets_from_dir(dir)?;
            }
        }
        handler.load_tests_from_dir(&handler.package_dir.join("tests"))?;
        Ok(handler)
    }

    /// Active la voie lint : registre de schémas (CRD `<package_dir>/crds` et répertoire de
    /// schémas donné), puis `LintRunner` sur `lint_dir`. Une erreur de construction remonte
    /// telle quelle.
    ///
    /// # Errors
    ///
    /// Returns a [`common::Error`] when the schema registry or the lint script cannot be built.
    pub fn with_lint(mut self, lint_dir: &Path, schemas_dir: Option<&Path>) -> common::Result<Self> {
        let crds = self.package_dir.join("crds");
        let registry = SchemaRegistry::new(Some(crds.as_path()), schemas_dir)?;
        self.lint = Some(LintRunner::new(lint_dir, &self.script_dir, registry)?);
        Ok(self)
    }

    /// Hook de suite du lint : une seule fois par invocation, après la suite. Ajoute un
    /// enregistrement `(suite)` seulement si la distribution définit `lint_suite`.
    pub fn run_suite_lint(&mut self) {
        let outcome = {
            let Some(lint) = self.lint.as_ref() else {
                return;
            };
            if !lint.has_suite_hook() {
                return;
            }
            let ctx = build_suite_context(&self.package, &self.tests, &self.test_sets, &self.executed);
            lint.run_suite(&ctx)
        };
        self.results
            .add("(suite)".to_string(), outcome.rows, std::time::Duration::ZERO);
        self.results.add_suppressions(outcome.suppressions);
    }

    /// Scans a directory for .yml/.yaml files (potentially multi-document)
    /// and loads all matching Test and `TestSet` objects.
    /// Objects with a different apiVersion or kind are silently ignored.
    /// Errors if a Test/TestSet name collides or if parsing fails.
    fn load_tests_from_dir(&mut self, dir: &Path) -> common::Result<()> {
        for (api_kind, json, file) in Self::scan_yaml_dir(dir)? {
            match api_kind.as_str() {
                "Test" => {
                    let test: VynilTest = serde_json::from_value(json)
                        .map_err(|e| common::Error::YamlError(format!("{file}: {e}")))?;
                    let name = test.metadata.name.clone();
                    if self.tests.contains_key(&name) {
                        return Err(common::Error::YamlError(format!(
                            "{file}: duplicate Test '{name}'"
                        )));
                    }
                    self.tests.insert(name, test);
                }
                "TestSet" => {
                    let ts: VynilTestSet = serde_json::from_value(json)
                        .map_err(|e| common::Error::YamlError(format!("{file}: {e}")))?;
                    let name = ts.metadata.name.clone();
                    if self.test_sets.contains_key(&name) {
                        return Err(common::Error::YamlError(format!(
                            "{file}: duplicate TestSet '{name}'"
                        )));
                    }
                    self.test_sets.insert(name, ts);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Scans a directory for .yml/.yaml files and loads only `TestSet` objects.
    /// Errors if a `TestSet` name collides or if parsing fails.
    fn load_testsets_from_dir(&mut self, dir: &Path) -> common::Result<()> {
        for (api_kind, json, file) in Self::scan_yaml_dir(dir)? {
            if api_kind != "TestSet" {
                continue;
            }
            let ts: VynilTestSet =
                serde_json::from_value(json).map_err(|e| common::Error::YamlError(format!("{file}: {e}")))?;
            let name = ts.metadata.name.clone();
            if self.test_sets.contains_key(&name) {
                return Err(common::Error::YamlError(format!(
                    "{file}: duplicate TestSet '{name}'"
                )));
            }
            self.test_sets.insert(name, ts);
        }
        Ok(())
    }

    #[must_use]
    pub fn get_test(&self, name: &str) -> Option<&VynilTest> {
        self.tests.get(name)
    }

    #[must_use]
    pub fn get_test_set(&self, name: &str) -> Option<&VynilTestSet> {
        self.test_sets.get(name)
    }

    #[must_use]
    pub fn list_tests(&self) -> Vec<String> {
        self.tests.keys().cloned().collect()
    }

    /// Returns a fully resolved `VynilTest`: handlebars templates in mocks and
    /// asserts are evaluated, and all referenced testSets are merged in.
    ///
    /// # Errors
    ///
    /// Returns a [`common::Error`] when the test or a referenced testSet is missing, when
    /// templating fails, or when the virtual instance mock cannot be serialized.
    pub fn get_templated_test(&self, name: &str) -> common::Result<VynilTest> {
        let test = self
            .tests
            .get(name)
            .ok_or_else(|| common::Error::Other(format!("Test '{name}' not found")))?;
        let package = &self.package;

        let mut hbs = HandleBars::new();

        // Build base context: package + instance (with appslug)
        let slug = appslug(&package.metadata.name, &test.instance.name);
        let mut inst = serde_json::to_value(&test.instance)?
            .as_object()
            .cloned()
            .unwrap_or_default();
        inst.insert("appslug".to_string(), serde_json::Value::String(slug));
        let base_ctx = serde_json::json!({
            "package": serde_json::to_value(package)?,
            "instance": serde_json::Value::Object(inst),
        });

        let mut k8s_mocks: Vec<Dynamic> = Vec::new();
        let mut http_mocks: Vec<HttpMockItem> = Vec::new();
        let mut asserts: Vec<VynilAssert> = Vec::new();

        // Template test's own mocks/asserts with base context
        collect_templated(
            &mut hbs,
            &base_ctx,
            test.mocks.as_ref(),
            test.asserts.as_ref(),
            &mut k8s_mocks,
            &mut http_mocks,
            &mut asserts,
        )?;

        // Merge each referenced testSet
        if let Some(refs) = &test.testSets {
            for ts_ref in refs {
                let ts = self
                    .test_sets
                    .get(&ts_ref.testSet)
                    .ok_or_else(|| common::Error::Other(format!("TestSet '{}' not found", ts_ref.testSet)))?;
                let mut ctx = base_ctx.clone();
                if let Some(obj) = ctx.as_object_mut() {
                    obj.insert("var".to_string(), build_var_context(ts, ts_ref));
                }

                collect_templated(
                    &mut hbs,
                    &ctx,
                    ts.mocks.as_ref(),
                    ts.asserts.as_ref(),
                    &mut k8s_mocks,
                    &mut http_mocks,
                    &mut asserts,
                )?;
            }
        }

        // Build a virtual instance mock for the instance being tested,
        // so Rhai scripts can find it via get_{service,system,tenant}_instance().
        let instance_kind = match &package.metadata.usage {
            VynilPackageType::Service => "ServiceInstance",
            VynilPackageType::System => "SystemInstance",
            VynilPackageType::Tenant => "TenantInstance",
        };
        if !instance_already_mocked(&k8s_mocks, instance_kind, &test.instance) {
            k8s_mocks.push(virtual_instance_dynamic(package, &test.instance)?);
        }

        Ok(VynilTest {
            apiVersion: test.apiVersion.clone(),
            kind: test.kind.clone(),
            metadata: test.metadata.clone(),
            instance: test.instance.clone(),
            testSets: None,
            mocks: if k8s_mocks.is_empty() && http_mocks.is_empty() {
                None
            } else {
                Some(VynilTestSetMocks {
                    kubernetes: if k8s_mocks.is_empty() {
                        None
                    } else {
                        Some(k8s_mocks)
                    },
                    http: if http_mocks.is_empty() {
                        None
                    } else {
                        Some(http_mocks)
                    },
                })
            },
            asserts: if asserts.is_empty() { None } else { Some(asserts) },
        })
    }

    /// Validates that every `TestSet` referenced in a Test's testSets exists.
    ///
    /// # Errors
    ///
    /// Returns a [`common::Error`] when a referenced testSet is not loaded.
    pub fn validate_refs(&self) -> common::Result<()> {
        for (test_name, test) in &self.tests {
            if let Some(refs) = &test.testSets {
                for ts_ref in refs {
                    if !self.test_sets.contains_key(&ts_ref.testSet) {
                        return Err(common::Error::Other(format!(
                            "Test '{test_name}' references unknown TestSet '{}'",
                            ts_ref.testSet
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn run_all_tests(&mut self) {
        let names = self.list_tests();
        for name in &names {
            // Shared vec where the mock k8s layer records created objects
            let created_objects: Arc<Mutex<Vec<Dynamic>>> = Arc::default();
            self.run_test(name, &created_objects);
        }
    }

    pub fn run_test(&mut self, name: &str, created_objects: &Arc<Mutex<Vec<Dynamic>>>) {
        let start = std::time::Instant::now();
        let mut suppressions: Vec<Suppression> = Vec::new();
        let result = self.run_test_inner(name, created_objects, &mut suppressions);
        let duration = start.elapsed();
        self.executed.push(name.to_string());
        self.results.add_suppressions(suppressions);

        match result {
            Ok(asserts) => {
                self.results.add(name.to_string(), asserts, duration);
            }
            Err(e) => {
                let fail = VynilAssertResult {
                    name: "execution".to_string(),
                    description: Some("Test execution".to_string()),
                    passed: false,
                    message: format!("{e}"),
                };
                self.results.add(name.to_string(), vec![fail], duration);
            }
        }
    }

    /// Exécute un test. Les suppressions du lint, s'il tourne, sont ajoutées à `suppressions`.
    fn run_test_inner(
        &self,
        name: &str,
        created_objects: &Arc<Mutex<Vec<Dynamic>>>,
        suppressions: &mut Vec<Suppression>,
    ) -> common::Result<Vec<VynilAssertResult>> {
        // Resolve test: merge testSets, template mocks/asserts
        let test = self.get_templated_test(name)?;

        // Extract mocks and asserts from resolved test
        let k8s_mocks = test
            .mocks
            .as_ref()
            .and_then(|m| m.kubernetes.clone())
            .unwrap_or_default();
        let http_mocks = test
            .mocks
            .as_ref()
            .and_then(|m| m.http.clone())
            .unwrap_or_default();

        // Build resolver path
        let type_dir = match self.package.metadata.usage {
            VynilPackageType::Tenant => "tenant",
            VynilPackageType::System => "system",
            VynilPackageType::Service => "service",
        };
        let resolver_path = vec![
            format!("{}/scripts", self.package_dir.display()),
            self.config_dir.display().to_string(),
            format!("{}/{type_dir}", self.script_dir.display()),
            format!("{}/lib", self.script_dir.display()),
        ];

        // Create mock rhai interpreter
        let mut rhai = Script::new_mock(resolver_path, http_mocks, k8s_mocks, created_objects.clone());

        // Execute value_script if present, otherwise empty map
        let mut asserts: Vec<VynilAssertResult> = Vec::new();
        let controller_values = if let Some(ref vs) = self.package.value_script {
            match rhai.eval(vs) {
                Ok(val) => {
                    asserts.push(VynilAssertResult {
                        name: "value_script".to_string(),
                        description: Some("Value script execution".to_string()),
                        passed: true,
                        message: "value script executed successfully".to_string(),
                    });
                    serde_json::to_string(&common::rhaihandler::to_dynamic(&val).unwrap_or_default())
                        .unwrap_or_else(|_| "{}".to_string())
                }
                Err(e) => {
                    asserts.push(VynilAssertResult {
                        name: "value_script".to_string(),
                        description: Some("Value script execution".to_string()),
                        passed: false,
                        message: format!("{e}"),
                    });
                    return Ok(asserts);
                }
            }
        } else {
            "{}".to_string()
        };

        // Build args for the rhai script
        let args = self.build_test_args(&test, &controller_values);
        rhai.set_dynamic("args", &args);

        // Build instance object for rhai (kept out of the runtime wiring on purpose)
        let _instance_json = self.build_test_instance(&test);
        let fun_name = match self.package.metadata.usage {
            VynilPackageType::Tenant => "get_tenant_instance",
            VynilPackageType::System => "get_system_instance",
            VynilPackageType::Service => "get_service_instance",
        };

        // Run the install script
        let _ = rhai.eval(
            format!(
                "import(\"context\") as ctx;\n\
            let instance = {fun_name}(args.namespace, args.instance);\n\
            let context = ctx::run(instance, args);\n\
            import(\"install\") as install;\n\
            install::run(instance, context);"
            )
            .as_str(),
        )?;

        // Validate asserts against created objects, then copy them for the lint lane
        let objects = created_objects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        asserts.extend(test.run_asserts(&objects));
        let created: Vec<Dynamic> = objects.to_vec();
        drop(objects);

        // Lint the created objects with the distribution script, when a lint lane is configured
        if let Some(lint) = &self.lint {
            let refs = self
                .tests
                .get(name)
                .and_then(|loaded| loaded.testSets.clone())
                .unwrap_or_default();
            let ctx = build_test_context(&self.package, &test, &refs);
            let (rows, lane_suppressions) =
                lint_test_lane(lint, &created, &ctx, || instance_status(&mut rhai, fun_name));
            asserts.extend(rows);
            suppressions.extend(lane_suppressions);
        }

        Ok(asserts)
    }

    /// Builds the rhai `args` object exposed to the install scripts during a test run.
    fn build_test_args(&self, test: &VynilTest, controller_values: &str) -> serde_json::Value {
        serde_json::json!({
            "namespace": test.instance.namespace,
            "instance": test.instance.name,
            "vynil_namespace": "vynil-system",
            "package_dir": self.package_dir.display().to_string(),
            "script_dir": self.script_dir.display().to_string(),
            "template_dir": self.template_dir.display().to_string(),
            "agent_image": common::DEFAULT_AGENT_IMAGE,
            "tag": "0.1.0",
            "config_dir": self.config_dir.display().to_string(),
            "controller_values": controller_values,
        })
    }

    /// Builds the instance JSON view handed to the rhai scripts during a test run.
    fn build_test_instance(&self, test: &VynilTest) -> serde_json::Value {
        let mut instance_json = serde_json::json!({
            "metadata": {
                "name": test.instance.name,
                "namespace": test.instance.namespace,
            },
            "spec": {
                "category": self.package.metadata.category,
                "package": self.package.metadata.name,
            },
            "status": {},
        });
        if let Some(opts) = &test.instance.options
            && let Some(spec) = instance_json
                .get_mut("spec")
                .and_then(serde_json::Value::as_object_mut)
        {
            spec.insert(
                "options".to_string(),
                serde_json::Value::Object(opts.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            );
        }
        instance_json
    }

    /// Reads all .yml/.yaml files in `dir`, parses multi-doc YAML,
    /// and yields (kind, `serde_json::Value`, filename) for each document
    /// matching our apiVersion.
    fn scan_yaml_dir(dir: &Path) -> common::Result<Vec<(String, serde_json::Value, String)>> {
        let mut results = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            match path.extension().and_then(|e| e.to_str()) {
                Some("yml" | "yaml") => {}
                _ => continue,
            }
            let display = path.display().to_string();
            let content = std::fs::read_to_string(&path)?;
            for de in serde_yaml::Deserializer::from_str(&content) {
                let json: serde_json::Value = serde::Deserialize::deserialize(de)
                    .map_err(|e| common::Error::YamlError(format!("{display}: {e}")))?;
                let api_version = json.get("apiVersion").and_then(|v| v.as_str()).map(String::from);
                let kind = json.get("kind").and_then(|v| v.as_str()).map(String::from);
                if api_version.as_deref() != Some(API_VERSION) {
                    continue;
                }
                if let Some(kind) = kind {
                    results.push((kind, json, display.clone()));
                }
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{cell::Cell, fs};

    const PACKAGE_YAML: &str = "apiVersion: vinyl.solidite.fr/v1beta1\nkind: Package\nmetadata:\n  name: demo\n  category: core\n  description: demo package\n  type: tenant\n  features: []\nrequirements: []\n";

    /// Installation qui crée un unique `ConfigMap` `app/web`.
    const INSTALL_ONE_CONFIGMAP: &str = r#"fn run(instance, context) {
    let h = k8s_resource("configmaps", "app");
    h.create(#{ apiVersion: "v1", kind: "ConfigMap", metadata: #{ name: "web", namespace: "app" } });
}
"#;

    /// Test sans asserts : l'installation et le lint sont les seules sources de lignes.
    fn plain_test(name: &str) -> String {
        format!(
            "apiVersion: vinyl.solidite.fr/v1beta1\nkind: Test\nmetadata:\n  name: {name}\ninstance:\n  name: demo\n  namespace: app\n"
        )
    }

    /// Test dont l'unique assert vérifie que `app/web` existe.
    fn test_with_web_assert() -> String {
        format!(
            "{}asserts:\n  - name: has-web\n    matcher: all\n    selector:\n      kind: ConfigMap\n      name: web\n      namespace: app\n",
            plain_test("default")
        )
    }

    /// Lint qui signale une erreur par objet reçu.
    const LINT_ERROR_PER_OBJECT: &str = r#"fn lint(objs, ctx) {
    let out = [];
    for obj in objs.all {
        out.push(#{
            rule: "no-web", severity: "error",
            object: #{ apiVersion: obj.apiVersion, kind: obj.kind, namespace: obj.metadata.namespace, name: obj.metadata.name },
            message: "web is forbidden"
        });
    }
    out
}
"#;

    struct Fixture {
        _root: tempfile::TempDir,
        package: PathBuf,
        scripts: PathBuf,
        lint: PathBuf,
    }

    /// Package de test : `package.yaml`, `scripts/` (context et install), `tests/`, et une
    /// distribution de lint optionnelle. Le répertoire de scripts de la distribution est vide.
    fn fixture(tests: &[(&str, String)], install: &str, lint: Option<&str>) -> Fixture {
        let root = tempfile::tempdir().expect("répertoire temporaire");
        let package = root.path().join("package");
        let scripts = package.join("scripts");
        let tests_dir = package.join("tests");
        let agent_scripts = root.path().join("agent-scripts");
        let lint_dir = root.path().join("distro");
        for dir in [&scripts, &tests_dir, &agent_scripts.join("lib"), &lint_dir] {
            fs::create_dir_all(dir).expect("création de répertoire");
        }
        fs::write(package.join("package.yaml"), PACKAGE_YAML).expect("package.yaml");
        fs::write(scripts.join("context.rhai"), "fn run(instance, args) { #{} }").expect("context.rhai");
        fs::write(scripts.join("install.rhai"), install).expect("install.rhai");
        for (file, body) in tests {
            fs::write(tests_dir.join(file), body).expect("fichier de test");
        }
        if let Some(src) = lint {
            fs::write(lint_dir.join("lint.rhai"), src).expect("lint.rhai");
        }
        Fixture {
            _root: root,
            package,
            scripts: agent_scripts,
            lint: lint_dir,
        }
    }

    fn handler(fx: &Fixture) -> TestHandler {
        TestHandler::new(
            fx.package.clone(),
            fx.scripts.clone(),
            fx.package.clone(),
            fx.scripts.clone(),
            None,
        )
        .expect("chargement du handler")
    }

    fn sink() -> Arc<Mutex<Vec<Dynamic>>> {
        Arc::default()
    }

    /// Document JSON du collecteur : `tests` (noms et asserts) et `suppressions`.
    fn report(h: &TestHandler) -> Value {
        serde_json::from_str(&h.results.to_json()).expect("JSON du collecteur")
    }

    fn assert_names(test: &Value) -> Vec<String> {
        test["asserts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn lint_rows_follow_asserts() {
        let fx = fixture(
            &[("default.yaml", test_with_web_assert())],
            INSTALL_ONE_CONFIGMAP,
            Some(LINT_ERROR_PER_OBJECT),
        );
        let mut h = handler(&fx).with_lint(&fx.lint, None).expect("lint configuré");
        h.run_test("default", &sink());
        let doc = report(&h);
        let test = &doc["tests"][0];
        assert_eq!(assert_names(test), ["has-web", "lint:no-web"]);
        assert_eq!(test["asserts"][0]["passed"], true);
        assert_eq!(test["asserts"][1]["passed"], false);
    }

    #[test]
    fn lint_skipped_when_install_fails() {
        let fx = fixture(
            &[("default.yaml", plain_test("default"))],
            "fn run(instance, context) { throw \"install broke\"; }\n",
            Some(LINT_ERROR_PER_OBJECT),
        );
        let mut h = handler(&fx).with_lint(&fx.lint, None).expect("lint configuré");
        h.run_test("default", &sink());
        let doc = report(&h);
        let asserts = &doc["tests"][0]["asserts"];
        assert_eq!(asserts.as_array().map(Vec::len), Some(1));
        assert_eq!(asserts[0]["name"], "execution");
        assert_eq!(asserts[0]["passed"], false);
    }

    #[test]
    fn lint_sees_own_objects_in_all_lane() {
        let fx = fixture(
            &[
                ("first.yaml", plain_test("first")),
                ("second.yaml", plain_test("second")),
            ],
            INSTALL_ONE_CONFIGMAP,
            Some(
                r#"fn lint(objs, ctx) {
    [#{
        rule: "count", severity: "warn",
        object: #{ apiVersion: "v1", kind: "Count", name: "all" },
        message: "objects=" + objs.all.len()
    }]
}
"#,
            ),
        );
        let mut h = handler(&fx).with_lint(&fx.lint, None).expect("lint configuré");
        h.run_all_tests();
        let doc = report(&h);
        for test in doc["tests"].as_array().into_iter().flatten() {
            let message = test["asserts"][0]["message"].as_str().unwrap_or_default();
            assert!(message.ends_with("objects=1"), "{message}");
        }
    }

    #[test]
    fn suppressed_finding_is_inventoried() {
        let fx = fixture(
            &[("default.yaml", plain_test("default"))],
            INSTALL_ONE_CONFIGMAP,
            Some(
                r#"fn lint(objs, ctx) {
    [#{
        rule: "no-web", severity: "error",
        object: #{ apiVersion: "v1", kind: "ConfigMap", namespace: "app", name: "web" },
        message: "web is forbidden",
        suppressed: true, reason: "legacy"
    }]
}
"#,
            ),
        );
        let mut h = handler(&fx).with_lint(&fx.lint, None).expect("lint configuré");
        h.run_test("default", &sink());
        assert_eq!(assert_names(&report(&h)["tests"][0]), Vec::<String>::new());
        let suppressions = h.results.suppressions();
        assert_eq!(suppressions.len(), 1);
        assert_eq!(suppressions[0].test, "default");
        assert_eq!(suppressions[0].rule, "no-web");
        assert_eq!(suppressions[0].reason.as_deref(), Some("legacy"));
    }

    #[test]
    fn no_with_lint_leaves_behaviour_unchanged() {
        let fx = fixture(
            &[("default.yaml", plain_test("default"))],
            INSTALL_ONE_CONFIGMAP,
            Some(LINT_ERROR_PER_OBJECT),
        );
        let mut h = handler(&fx);
        h.run_test("default", &sink());
        h.run_suite_lint();
        let doc = report(&h);
        assert_eq!(assert_names(&doc["tests"][0]), Vec::<String>::new());
        assert_eq!(doc["tests"].as_array().map(Vec::len), Some(1));
        assert!(doc.get("suppressions").is_none());
    }

    #[test]
    fn suite_hook_adds_one_suite_record() {
        let fx = fixture(
            &[("default.yaml", plain_test("default"))],
            INSTALL_ONE_CONFIGMAP,
            Some(
                r#"fn lint_suite(ctx) {
    [#{
        rule: "suite-size", severity: "error",
        object: #{ apiVersion: "v1", kind: "Suite", name: "all" },
        message: "suite checked"
    }]
}
"#,
            ),
        );
        let mut h = handler(&fx).with_lint(&fx.lint, None).expect("lint configuré");
        h.run_all_tests();
        h.run_suite_lint();
        let doc = report(&h);
        let suite = &doc["tests"][1];
        assert_eq!(suite["test_name"], "(suite)");
        assert_eq!(assert_names(suite), ["lint:suite-size"]);
        assert_eq!(suite["asserts"][0]["passed"], false);
    }

    #[test]
    fn suite_hook_without_lint_adds_nothing() {
        // Sans `with_lint`, `run_suite_lint` ne produit aucun enregistrement.
        let fx = fixture(&[], INSTALL_ONE_CONFIGMAP, None);
        let mut h = handler(&fx);
        h.run_suite_lint();
        assert_eq!(report(&h)["tests"].as_array().map(Vec::len), Some(0));
    }
    /// Installation qui crée un `Deployment`, un `ConfigMap` et un `Secret` ; seuls les deux
    /// premiers sont déclarés par `set_status_scalables` et `set_status_befores`.
    const INSTALL_SECTIONED: &str = r#"fn run(instance, context) {
    let deploys = k8s_resource("deployments", "app");
    deploys.create(#{ apiVersion: "apps/v1", kind: "Deployment", metadata: #{ name: "web", namespace: "app" } });
    let cms = k8s_resource("configmaps", "app");
    cms.create(#{ apiVersion: "v1", kind: "ConfigMap", metadata: #{ name: "cfg", namespace: "app" } });
    let secrets = k8s_resource("secrets", "app");
    secrets.create(#{ apiVersion: "v1", kind: "Secret", metadata: #{ name: "loose", namespace: "app" } });
    instance.set_status_scalables([#{ apiVersion: "apps/v1", kind: "Deployment", name: "web", namespace: "app" }]);
    instance.set_status_befores([#{ apiVersion: "v1", kind: "ConfigMap", name: "cfg", namespace: "app" }]);
}
"#;

    /// Lint qui constate chaque objet de `objs.scalables` puis chaque objet de `objs.unsectioned`.
    const LINT_SCALABLES_AND_UNSECTIONED: &str = r#"fn lint(objs, ctx) {
    let out = [];
    for obj in objs.scalables {
        out.push(#{
            rule: "scalable", severity: "error",
            object: #{ apiVersion: obj.apiVersion, kind: obj.kind, namespace: obj.metadata.namespace, name: obj.metadata.name },
            message: "scalable seen"
        });
    }
    for obj in objs.unsectioned {
        out.push(#{
            rule: "unsectioned", severity: "error",
            object: #{ apiVersion: obj.apiVersion, kind: obj.kind, namespace: obj.metadata.namespace, name: obj.metadata.name },
            message: "unsectioned seen"
        });
    }
    out
}
"#;

    #[test]
    fn lint_sections_follow_what_the_package_declared() {
        let fx = fixture(
            &[("default.yaml", plain_test("default"))],
            INSTALL_SECTIONED,
            Some(LINT_SCALABLES_AND_UNSECTIONED),
        );
        let mut h = handler(&fx).with_lint(&fx.lint, None).expect("lint configuré");
        h.run_test("default", &sink());
        let doc = report(&h);
        let test = &doc["tests"][0];
        assert_eq!(assert_names(test), ["lint:scalable", "lint:unsectioned"]);
        assert_eq!(test["asserts"][0]["description"], "Deployment app/web");
        assert_eq!(test["asserts"][1]["description"], "Secret app/loose");
    }

    #[test]
    fn an_unreadable_status_fails_a_lint_sections_row() {
        let created =
            vec![rhai::serde::to_dynamic(serde_json::json!({
            "apiVersion": "apps/v1", "kind": "Deployment", "metadata": {"name": "web", "namespace": "app"}
        }))
        .expect("objet convertible")];
        let (sections, row) = lint_sections(&created, Err("unknown function".to_string()));
        let row = row.expect("une ligne lint:sections");
        assert_eq!(row.name, "lint:sections");
        assert!(!row.passed);
        assert!(row.message.contains("unknown function"));
        assert!(sections.scalables.is_empty());
        assert_eq!(sections.unsectioned.len(), 1);
        let (_, none) = lint_sections(&created, Ok(Dynamic::from(rhai::Map::new())));
        assert!(none.is_none(), "un statut lu sans section n'est pas une erreur");
    }

    #[test]
    fn a_suite_only_distribution_reads_no_status() {
        let ctx = json!({"package": {"name": "demo"}, "test": {"name": "default"}});
        let fx = fixture(&[], INSTALL_ONE_CONFIGMAP, Some("fn lint_suite(ctx) { [] }\n"));
        let h = handler(&fx).with_lint(&fx.lint, None).expect("lint configuré");
        let lint = h.lint.as_ref().expect("voie lint");
        let read = Cell::new(false);
        let (rows, suppressions) = lint_test_lane(lint, &[], &ctx, || {
            read.set(true);
            Err("statut illisible".to_string())
        });
        assert!(!read.get(), "le statut ne doit pas être lu sans hook `lint`");
        assert!(rows.is_empty(), "{rows:?}");
        assert!(suppressions.is_empty());

        let fx = fixture(&[], INSTALL_ONE_CONFIGMAP, Some("fn lint(objs, ctx) { [] }\n"));
        let h = handler(&fx).with_lint(&fx.lint, None).expect("lint configuré");
        let lint = h.lint.as_ref().expect("voie lint");
        let read = Cell::new(false);
        let (rows, _) = lint_test_lane(lint, &[], &ctx, || {
            read.set(true);
            Err("statut illisible".to_string())
        });
        assert!(read.get(), "le statut doit être lu avec un hook `lint`");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].name, "lint:sections");
        assert!(!rows[0].passed);
    }
}
