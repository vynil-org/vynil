use crate::{
    context,
    handlebarshandler::handlebars_rhai_register,
    httphandler::http_rhai_register,
    httpmock::{HttpMockItem, httpmock_rhai_register},
    instanceservice::service_rhai_register,
    instancesystem::system_rhai_register,
    instancetenant::tenant_rhai_register,
    jukebox::jukebox_rhai_register,
    jukebox_file::file_jukebox_rhai_register,
    k8sgeneric::k8sgeneric_rhai_register,
    k8smock::k8smock_rhai_register,
    k8sraw::k8sraw_rhai_register,
    k8sworkload::k8sworkload_rhai_register,
    s3handler::s3_rhai_register,
    vynilpackage::package_rhai_register,
    yamlhandler::yaml_ordered_rhai_register,
};
use vynil_core::oci_mock::oci_mock_rhai_register;

// ── Re-exports from core ──────────────────────────────────────────────────────

pub use vynil_core::engine::{
    AST, ASTNode, Array, Dynamic, Engine, Expr, FileModuleResolver, ImmutableString, Map, Module,
    ModuleResolversCollection, ParseError, Scope, Stmt, base64_decode, to_dynamic,
};

// ── Newtype wrapper around vynil_core::Script ─────────────────────────────────

/// Wrapper preserving the historical `common::Script` API over `vynil_core::engine::Script`.
#[derive(Debug)]
pub struct Script(pub vynil_core::engine::Script);

impl std::ops::Deref for Script {
    type Target = vynil_core::engine::Script;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for Script {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

fn vynil_owner_register(engine: &mut Engine) {
    engine.register_fn("vynil_owner", || -> Dynamic {
        context::get_owner()
            .and_then(|o| to_dynamic(&o).ok())
            .unwrap_or_else(|| Dynamic::from_map(Map::new()))
    });
}

impl Script {
    #[must_use]
    pub fn new_core(resolver_path: Vec<String>) -> Self {
        let mut script = Self(vynil_core::engine::Script::new_bare(resolver_path));
        vynil_owner_register(&mut script.engine);
        yaml_ordered_rhai_register(&mut script.engine);
        package_rhai_register(&mut script.engine);
        handlebars_rhai_register(&mut script.engine);
        script
    }

    #[must_use]
    pub fn new_file_scan(resolver_path: Vec<String>) -> Self {
        let mut script = Self::new_core(resolver_path);
        http_rhai_register(&mut script.engine);
        s3_rhai_register(&mut script.engine);
        file_jukebox_rhai_register(&mut script.engine);
        script
    }

    #[must_use]
    pub fn new(resolver_path: Vec<String>) -> Self {
        let mut script = Self::new_core(resolver_path);
        http_rhai_register(&mut script.engine);
        s3_rhai_register(&mut script.engine);
        service_rhai_register(&mut script.engine);
        system_rhai_register(&mut script.engine);
        tenant_rhai_register(&mut script.engine);
        jukebox_rhai_register(&mut script.engine);
        k8sgeneric_rhai_register(&mut script.engine);
        k8sraw_rhai_register(&mut script.engine);
        k8sworkload_rhai_register(&mut script.engine);
        script
    }

    pub fn new_mock(
        resolver_path: Vec<String>,
        http_mocks: Vec<HttpMockItem>,
        k8s_mocks: Vec<Dynamic>,
        created_objects: std::sync::Arc<std::sync::Mutex<Vec<Dynamic>>>,
    ) -> Self {
        let mut script = Self::new_core(resolver_path);
        oci_mock_rhai_register(&mut script.engine);
        httpmock_rhai_register(&mut script.engine, http_mocks);
        k8smock_rhai_register(&mut script.engine, k8s_mocks, created_objects);
        script
    }

    // ── Error-converting wrappers (vynil_core::Result → common::Result) ─────

    /// Runs the rhai script stored in `file`.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or the script fails to parse or run.
    pub fn run_file(&mut self, file: &std::path::PathBuf) -> crate::Result<Dynamic> {
        self.0.run_file(file).map_err(Into::into)
    }

    /// Evaluates a rhai script and returns its result.
    ///
    /// # Errors
    ///
    /// Returns an error when the script fails to parse or run.
    pub fn eval(&mut self, script: &str) -> crate::Result<Dynamic> {
        self.0.eval(script).map_err(Into::into)
    }

    /// Evaluates a rhai script expecting a boolean result.
    ///
    /// # Errors
    ///
    /// Returns an error when the script fails to parse, run, or return a boolean.
    pub fn eval_truth(&mut self, script: &str) -> crate::Result<bool> {
        self.0.eval_truth(script).map_err(Into::into)
    }

    /// Evaluates a rhai script expecting a string result.
    ///
    /// # Errors
    ///
    /// Returns an error when the script fails to parse, run, or return a string.
    pub fn eval_map_string(&mut self, script: &str) -> crate::Result<String> {
        self.0.eval_map_string(script).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_script() -> Script {
        Script::new(vec![])
    }

    // ── Régression : préservation de l'ordre des clés YAML ───────────────────
    #[test]
    fn test_yaml_decode_modify_single_key_preserves_order() {
        let mut s = make_script();

        let original = "z: first\na: second\nm: third\n";
        s.ctx
            .set_or_push("input_yaml", Dynamic::from(original.to_string()));

        let result = s
            .eval(
                r#"
            let doc = yaml_decode_ordered(input_yaml);
            doc["a"] = "MODIFIED";
            yaml_encode_ordered(doc)
        "#,
            )
            .unwrap();

        let encoded = result.to_string();

        let orig_lines: Vec<&str> = original.lines().filter(|l| !l.is_empty()).collect();
        let enc_lines: Vec<&str> = encoded.lines().filter(|l| !l.is_empty()).collect();

        assert_eq!(
            orig_lines.len(),
            enc_lines.len(),
            "Le nombre de lignes a changé — des clés ont été perdues ou ajoutées.\n\
             Original:\n{original}\nRésultat:\n{encoded}"
        );

        let changed: Vec<usize> = orig_lines
            .iter()
            .zip(enc_lines.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i)
            .collect();

        assert_eq!(
            changed.len(),
            1,
            "Attendu exactement 1 ligne modifiée (la clé 'a'), mais {} lignes diffèrent.\n\
             Cela indique que l'ordre des clés n'a PAS été préservé (bug serde_yaml/BTreeMap).\n\
             Original:\n{}\nRésultat:\n{}",
            changed.len(),
            original,
            encoded
        );

        assert!(
            enc_lines[changed[0]].contains("MODIFIED"),
            "La ligne modifiée ne contient pas 'MODIFIED' : {}",
            enc_lines[changed[0]]
        );
    }

    // ── Spec yamlhandler : « Removing a key keeps the order of the others » ──
    #[test]
    fn removing_a_key_keeps_the_order_of_the_others() {
        use std::str::FromStr;

        let mut s = make_script();
        s.ctx
            .set_or_push("input_yaml", Dynamic::from("z: 1\na: 2\nm: 3\n".to_string()));

        let result = s
            .eval(
                r#"
            let doc = yaml_decode_ordered(input_yaml);
            let first = doc.remove("a");
            let second = doc.remove("a");
            [first, second, yaml_encode_ordered(doc)]
        "#,
            )
            .unwrap();

        let results = result.into_array().expect("un tableau");
        assert!(
            results[0].clone().cast::<bool>(),
            "le premier remove doit rendre true"
        );
        assert!(
            !results[1].clone().cast::<bool>(),
            "le second remove doit rendre false"
        );
        assert_eq!(results[2].to_string(), "z: 1\nm: 3\n");

        let mut seq = crate::yamlhandler::YamlDoc::from_str("- x\n- y\n").unwrap();
        let before = seq.to_yaml_string().unwrap();
        assert!(
            !seq.remove("0".into()),
            "remove sur une racine séquence rend false"
        );
        assert_eq!(seq.to_yaml_string().unwrap(), before);

        let mut scalar = crate::yamlhandler::YamlDoc::from_str("hello\n").unwrap();
        let before = scalar.to_yaml_string().unwrap();
        assert!(
            !scalar.remove("hello".into()),
            "remove sur une racine scalaire rend false"
        );
        assert_eq!(scalar.to_yaml_string().unwrap(), before);
    }

    // ── Spec yamlhandler : « Read-only members resolve in scripts » ──────────
    #[test]
    fn read_only_members_resolve_in_scripts() {
        let mut s = make_script();
        s.ctx
            .set_or_push("input_yaml", Dynamic::from("a: 1\nb: 2\n".to_string()));

        let result = s
            .eval(
                r#"
            let doc = yaml_decode_ordered(input_yaml);
            [doc.contains("a"), doc.contains("z"), "a" in doc, doc.len(), doc.is_empty()]
        "#,
            )
            .unwrap();

        let results = result.into_array().expect("un tableau");
        assert!(
            results[0].clone().cast::<bool>(),
            "contains(\"a\") doit rendre true"
        );
        assert!(
            !results[1].clone().cast::<bool>(),
            "contains(\"z\") doit rendre false"
        );
        assert!(results[2].clone().cast::<bool>(), "\"a\" in doc doit rendre true");
        assert_eq!(results[3].clone().cast::<i64>(), 2, "len() doit rendre 2");
        assert!(!results[4].clone().cast::<bool>(), "is_empty() doit rendre false");

        s.ctx.set_or_push("input_yaml", Dynamic::from(String::new()));
        let result = s
            .eval(
                r#"
            let doc = yaml_decode_ordered(input_yaml);
            [doc.is_empty(), doc.len()]
        "#,
            )
            .unwrap();

        let results = result.into_array().expect("un tableau");
        assert!(
            results[0].clone().cast::<bool>(),
            "is_empty() sur un document vide doit rendre true"
        );
        assert_eq!(
            results[1].clone().cast::<i64>(),
            0,
            "len() sur un document vide doit rendre 0"
        );
    }
}
