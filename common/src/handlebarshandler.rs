use crate::RhaiRes;
use handlebars::handlebars_helper;
pub use handlebars::{
    Path as HbsPath, PathSeg,
    template::{Parameter, Template, TemplateElement},
};
pub use serde_json::Value;
use tracing::warn;

/// Helpers Handlebars spécifiques à vynil (en plus de `vynil_core::hbs::CORE_HBS_HELPERS`).
pub const VYNIL_HBS_HELPERS: &[&str] = &[
    "selector_from_ctx",
    "labels_from_ctx",
    "ctx_have_crd",
    "have_system_service",
    "have_tenant_service",
    "image_from_ctx",
    "resources_from_ctx",
    "render_template",
    "render_file",
];

/// Vrai si `name` est un helper natif (core générique OU contextuel vynil).
#[must_use]
pub fn is_native_hbs_helper(name: &str) -> bool {
    vynil_core::hbs::CORE_HBS_HELPERS.contains(&name) || VYNIL_HBS_HELPERS.contains(&name)
}

// ── Contextual helpers (vynil-specific, stay in common) ────────────────────────

handlebars_helper!(selector: |ctx: Value, {comp:str=""}| {
    let mut sel = ctx
        .get("instance")
        .and_then(|v| v.get("selector"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !comp.is_empty() {
        sel.insert("app.kubernetes.io/component".into(), Value::from(comp));
    }
    sel
});
handlebars_helper!(labels: |ctx: Value, {comp:str=""}| {
    let mut sel = ctx
        .get("instance")
        .and_then(|v| v.get("labels"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !comp.is_empty() {
        sel.insert("app.kubernetes.io/component".into(), Value::from(comp));
    }
    sel
});
handlebars_helper!(have_crd: |ctx: Value, name: String| {
    ctx.get("cluster")
        .and_then(|v| v.get("crds"))
        .and_then(Value::as_array)
        .is_some_and(|crds| crds.iter().any(|crd| *crd == name))
});
handlebars_helper!(have_system_service: |ctx: Value, name: String| {
    ctx.get("cluster")
        .and_then(|v| v.get("services"))
        .and_then(Value::as_array)
        .is_some_and(|services| {
            services
                .iter()
                .any(|s| s.get("key") == Some(&Value::String(name.clone())))
        })
});
handlebars_helper!(have_tenant_service: |ctx: Value, name: String| {
    ctx.get("tenant")
        .and_then(|v| v.get("services"))
        .and_then(Value::as_array)
        .is_some_and(|services| {
            services
                .iter()
                .any(|s| s.get("key") == Some(&Value::String(name.clone())))
        })
});

handlebars_helper!(render_template: |template: String, data: Value| {
    let mut hbs = HandleBars::new();
    hbs.render(&template, &data).unwrap_or_else(|e| {
        warn!("handlebars::render_template failed with: {:?}",e);
        String::new()
    })
});

// Variante de `render_template` pour le moteur restreint : le gabarit imbriqué est lui aussi
// rendu par un moteur restreint, sinon il rouvrirait les helpers qui touchent à l'hôte.
handlebars_helper!(render_template_restricted: |template: String, data: Value| {
    let mut hbs = HandleBars::new_restricted();
    hbs.render(&template, &data).unwrap_or_else(|e| {
        warn!("handlebars::render_template failed with: {:?}",e);
        String::new()
    })
});

handlebars_helper!(render_file: |file: String, data: Value| {
    let mut hbs = HandleBars::new();
    match std::fs::read_to_string(file.clone()) {
        Ok(template) => {
            hbs.render_named(&file, &template, &data).unwrap_or_else(|e| {
                warn!("handlebars::render_file failed to render with: {:?}",e);
                String::new()
            })
        },
        Err(e) => {
            warn!("handlebars::render_file failed to read {file} with: {:?}", e);
            String::new()
        }
    }
});

// ── Newtype wrapper around vynil_core::HandleBars ──────────────────────────────

#[derive(Clone, Debug)]
pub struct HandleBars<'a>(pub vynil_core::hbs::HandleBars<'a>);

impl<'a> std::ops::Deref for HandleBars<'a> {
    type Target = vynil_core::hbs::HandleBars<'a>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for HandleBars<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl HandleBars<'_> {
    /// Moteur complet : tous les helpers de vynil, `render_file` compris.
    #[must_use]
    pub fn new() -> HandleBars<'static> {
        Self::with_base(vynil_core::hbs::HandleBars::new(), true)
    }

    /// Moteur de @new privé des helpers qui touchent à l'hôte (voir la spec de ce module) :
    /// les mêmes helpers de vynil, sauf `render_file`, et sans `jsonnet` (son `importstr`
    /// lit n'importe quel fichier). `render_template` imbriqué reste restreint.
    #[must_use]
    pub fn new_restricted() -> HandleBars<'static> {
        let mut inner = vynil_core::hbs::HandleBars::new_restricted();
        inner.engine_mut().unregister_helper("jsonnet");
        Self::with_base(inner, false)
    }

    /// Ajoute les helpers de vynil à `inner`. `host_file_render` vrai : `render_template` et
    /// `render_file` au moteur complet ; faux : `render_template` au moteur restreint seulement.
    fn with_base(
        mut inner: vynil_core::hbs::HandleBars<'static>,
        host_file_render: bool,
    ) -> HandleBars<'static> {
        let engine = inner.engine_mut();
        engine.register_helper("labels_from_ctx", Box::new(labels));
        engine.register_helper("ctx_have_crd", Box::new(have_crd));
        engine.register_helper("selector_from_ctx", Box::new(selector));
        engine.register_helper("have_system_service", Box::new(have_system_service));
        engine.register_helper("have_tenant_service", Box::new(have_tenant_service));
        if host_file_render {
            engine.register_helper("render_template", Box::new(render_template));
            engine.register_helper("render_file", Box::new(render_file));
        } else {
            engine.register_helper("render_template", Box::new(render_template_restricted));
        }
        let _ = engine.register_script_helper("image_from_ctx",
            "let root = params[0];let name = params[1];\n\
            let im = root[\"instance\"][\"images\"][name];\n\
            let tag = if (\"tag\" in im && im[\"tag\"] != ()) {im[\"tag\"]} else {root[\"instance\"][\"package\"][\"app_version\"]};
            `${im[\"registry\"]}/${im[\"repository\"]}:${tag}`");
        let _ = engine.register_script_helper(
            "resources_from_ctx",
            "let root = params[0]?[\"instance\"]?[\"resources\"]; let name = params[1];\n\
            let res = #{}; let req = #{}; let lim = #{};\n\
            let v = root?[name]?[\"requests\"]?[\"cpu\"]; if v != () { req[\"cpu\"] = v; }\n\
            let v = root?[name]?[\"requests\"]?[\"memory\"]; if v != () { req[\"memory\"] = v; }\n\
            let v = root?[name]?[\"limits\"]?[\"cpu\"]; if v != () { lim[\"cpu\"] = v; }\n\
            let v = root?[name]?[\"limits\"]?[\"memory\"]; if v != () { lim[\"memory\"] = v; }\n\
            if req.len() != 0 { res[\"requests\"] = req; }\n\
            if lim.len() != 0 { res[\"limits\"] = lim; }\n\
            res",
        );
        HandleBars(inner)
    }
}

pub fn handlebars_rhai_register(engine: &mut rhai::Engine) {
    engine
        .register_type_with_name::<HandleBars<'_>>("HandleBars")
        .register_fn("new_hbs", HandleBars::new)
        .register_fn(
            "register_template",
            |h: &mut HandleBars, name: String, template: String| -> RhaiRes<()> {
                h.rhai_register_template(name, template)
            },
        )
        .register_fn(
            "register_partial_dir",
            |h: &mut HandleBars, directory: String| -> RhaiRes<()> { h.rhai_register_partial_dir(directory) },
        )
        .register_fn(
            "register_helper_dir",
            |h: &mut HandleBars, directory: String| -> RhaiRes<()> { h.rhai_register_helper_dir(directory) },
        )
        .register_fn(
            "render_from",
            |h: &mut HandleBars, template: String, data: rhai::Map| -> RhaiRes<String> {
                h.rhai_render(template, data)
            },
        )
        .register_fn(
            "render_named",
            |h: &mut HandleBars, name: String, template: String, data: rhai::Map| -> RhaiRes<String> {
                h.rhai_render_named(name, template, data)
            },
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── rhai_render: conversion rhai::Map → contexte Handlebars ──────────────

    #[test]
    fn test_rhai_render_scalar_values() {
        let mut hbs = HandleBars::new();
        let mut map = rhai::Map::new();
        map.insert("num".into(), rhai::Dynamic::from(42_i64));
        map.insert("msg".into(), rhai::Dynamic::from("hello"));
        map.insert("flag".into(), rhai::Dynamic::from(true));
        assert_eq!(hbs.rhai_render("{{num}}".into(), map.clone()).unwrap(), "42");
        assert_eq!(hbs.rhai_render("{{msg}}".into(), map.clone()).unwrap(), "hello");
        assert_eq!(hbs.rhai_render("{{flag}}".into(), map).unwrap(), "true");
    }

    #[test]
    fn test_rhai_render_nested_map() {
        let mut hbs = HandleBars::new();
        let mut inner = rhai::Map::new();
        inner.insert("port".into(), rhai::Dynamic::from(8080_i64));
        inner.insert("host".into(), rhai::Dynamic::from("localhost"));
        let mut outer = rhai::Map::new();
        outer.insert("svc".into(), rhai::Dynamic::from_map(inner));
        let result = hbs
            .rhai_render("{{svc.host}}:{{svc.port}}".into(), outer)
            .unwrap();
        assert_eq!(result, "localhost:8080");
    }

    #[test]
    fn test_rhai_render_named_basic() {
        let mut hbs = HandleBars::new();
        let mut map = rhai::Map::new();
        map.insert("v".into(), rhai::Dynamic::from("ok"));
        let result = hbs
            .rhai_render_named("t".into(), "result: {{v}}".into(), map)
            .unwrap();
        assert_eq!(result, "result: ok");
    }

    // ── Built-in helpers ──────────────────────────────────────────────────────

    #[test]
    fn test_helper_gen_password_length_and_classes() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({});
        let p = hbs.render("{{gen_password 24}}", &data).unwrap();
        assert_eq!(p.chars().count(), 24);
        assert!(p.chars().any(|c| c.is_ascii_lowercase()));
        assert!(p.chars().any(|c| c.is_ascii_uppercase()));
        assert!(p.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_helper_gen_password_symbols_disabled() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({});
        let p = hbs.render("{{gen_password 32 symbols=0}}", &data).unwrap();
        assert_eq!(p.chars().count(), 32);
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_helper_gen_private_key_ed25519() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({});
        let result = hbs.render("{{gen_private_key \"ed25519\"}}", &data).unwrap();
        assert!(result.contains("-----BEGIN PRIVATE KEY-----"));
    }

    #[test]
    fn test_helper_gen_private_key_rsa_with_bits() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({});
        let result = hbs
            .render("{{gen_private_key \"rsa\" bits=2048}}", &data)
            .unwrap();
        assert!(result.contains("-----BEGIN PRIVATE KEY-----"));
        let key = openssl::pkey::PKey::private_key_from_pem(result.as_bytes()).unwrap();
        assert_eq!(key.bits(), 2048);
    }

    #[test]
    fn test_helper_base64_encode() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({"v": "hello"});
        let result = hbs.render("{{base64_encode v}}", &data).unwrap();
        assert_eq!(result, "aGVsbG8=");
    }

    #[test]
    fn test_helper_base64_decode() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({"v": "aGVsbG8="});
        let result = hbs.render("{{base64_decode v}}", &data).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_helper_base64_roundtrip() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({"v": "secret-password-123"});
        let encoded = hbs.render("{{base64_encode v}}", &data).unwrap();
        let decoded_data = serde_json::json!({"v": encoded});
        let decoded = hbs.render("{{base64_decode v}}", &decoded_data).unwrap();
        assert_eq!(decoded, "secret-password-123");
    }

    #[test]
    fn test_helper_url_encode() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({"v": "hello world&foo=bar"});
        let result = hbs.render("{{url_encode v}}", &data).unwrap();
        assert!(result.contains("%26") || result.contains("&amp;"));
        assert!(!result.contains(' '));
    }

    #[test]
    fn test_helper_to_decimal_octal() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({"v": "755"});
        let result = hbs.render("{{to_decimal v}}", &data).unwrap();
        assert_eq!(result, "493");
    }

    #[test]
    fn test_helper_to_decimal_zero() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({"v": "0"});
        let result = hbs.render("{{to_decimal v}}", &data).unwrap();
        assert_eq!(result, "0");
    }

    #[test]
    fn test_helper_concat() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({"a": "foo", "b": "bar"});
        let result = hbs.render("{{concat a b}}", &data).unwrap();
        assert_eq!(result, "foobar");
    }

    #[test]
    fn test_helper_header_basic() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({"u": "user", "p": "pass"});
        let result = hbs.render("{{header_basic u p}}", &data).unwrap();
        assert_eq!(result, "Basic dXNlcjpwYXNz");
    }

    // ── resources_from_ctx ────────────────────────────────────────────────────

    #[test]
    fn test_resources_from_ctx_full_config() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({
            "instance": {
                "resources": {
                    "app": {
                        "requests": {"cpu": "50m", "memory": "128Mi"},
                        "limits":   {"cpu": "500m", "memory": "512Mi"}
                    }
                }
            }
        });
        let result = hbs
            .render(r#"{{json_to_str (resources_from_ctx this "app")}}"#, &data)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["requests"]["cpu"], "50m");
        assert_eq!(parsed["requests"]["memory"], "128Mi");
        assert_eq!(parsed["limits"]["cpu"], "500m");
        assert_eq!(parsed["limits"]["memory"], "512Mi");
    }

    #[test]
    fn test_resources_from_ctx_requests_only_no_limits_key() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({
            "instance": {
                "resources": {
                    "app": {
                        "requests": {"cpu": "50m", "memory": "256Mi"}
                    }
                }
            }
        });
        let result = hbs
            .render(r#"{{json_to_str (resources_from_ctx this "app")}}"#, &data)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["requests"]["cpu"], "50m");
        assert_eq!(parsed["requests"]["memory"], "256Mi");
        assert!(
            parsed.get("limits").is_none(),
            "limits key must be absent when not configured, got: {result}"
        );
    }

    #[test]
    fn test_resources_from_ctx_scaler_does_not_leak() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({
            "instance": {
                "resources": {
                    "app": {
                        "requests": {"cpu": "50m"},
                        "scaler": {"max_replicas": 5, "average_utilization": 70}
                    }
                }
            }
        });
        let result = hbs
            .render(r#"{{json_to_str (resources_from_ctx this "app")}}"#, &data)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            parsed.get("scaler").is_none(),
            "vynil 'scaler' field must not appear in k8s resources output, got: {result}"
        );
    }

    #[test]
    fn test_resources_from_ctx_empty_context_returns_empty_object() {
        let mut hbs = HandleBars::new();
        let data = serde_json::json!({ "instance": {} });
        let result = hbs
            .render(r#"{{json_to_str (resources_from_ctx this "app")}}"#, &data)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            parsed.get("requests").is_none() && parsed.get("limits").is_none(),
            "empty context must produce empty resources object, got: {result}"
        );
    }

    // ── Régression : new_hbs() en rhai expose les helpers contextuels vynil ──
    //
    // Après l'extraction de vynil-core, le moteur Handlebars générique vit dans
    // vynil-core (sans helpers contextuels). Le binding rhai `new_hbs` doit
    // construire le `common::HandleBars` COMPLET, sinon tout rendu de package via
    // `new_hbs().render_*(…, context)` (install_crds, template_crds, schedule_backup…)
    // perd selector_from_ctx / labels_from_ctx / image_from_ctx / resources_from_ctx.
    #[test]
    fn test_new_hbs_rhai_exposes_contextual_helpers() {
        let mut s = crate::rhaihandler::Script::new(vec![]);
        let code = r#"
            let ctx = #{ instance: #{ selector: #{ "app": "demo" } } };
            let hbs = new_hbs();
            hbs.render_from("{{#each (selector_from_ctx this)}}{{@key}}={{this}}{{/each}}", ctx)
        "#;
        let out = s.eval(code).unwrap().to_string();
        assert_eq!(
            out, "app=demo",
            "new_hbs() en rhai ne voit pas selector_from_ctx — le binding doit produire common::HandleBars"
        );
    }

    // ── new_restricted : aucun helper qui touche à l'hôte ─────────────────────

    #[test]
    fn a_restricted_engine_has_no_host_helpers() {
        let mut hbs = HandleBars::new_restricted();
        let data = serde_json::json!({ "this": 1 });
        for template in [
            r#"{{render_file "x" this}}"#,
            r#"{{read_to_str "x"}}"#,
            r#"{{env_var "PATH"}}"#,
            r#"{{canonicalize "."}}"#,
            r#"{{#jsonnet}}importstr "/etc/hostname"{{/jsonnet}}"#,
        ] {
            let result = hbs.render(template, &data);
            assert!(
                matches!(result, Err(vynil_core::Error::HbsRenderError(_))),
                "{template} doit échouer en rendu, obtenu {result:?}"
            );
        }
        assert_eq!(hbs.render(r#"{{concat "a" "b"}}"#, &data).unwrap(), "ab");
        assert!(hbs.render(r#"{{render_template "x" this}}"#, &data).is_ok());
        assert!(hbs.render("{{labels_from_ctx this}}", &data).is_ok());
    }

    #[test]
    fn a_restricted_engine_does_not_reopen_host_helpers_through_render_template() {
        let mut hbs = HandleBars::new_restricted();
        let data = serde_json::json!({});
        let path = std::env::var("PATH").expect("PATH de test");
        assert!(!path.is_empty());
        let out = hbs
            .render(r#"{{render_template "{{env_var \"PATH\"}}" this}}"#, &data)
            .expect("le helper avale l'échec imbriqué");
        assert!(!out.contains(&path), "le PATH de l'hôte est rendu : {out}");
    }

    #[test]
    fn a_restricted_engine_cannot_import_host_files_through_jsonnet() {
        let file = tempfile::NamedTempFile::new().expect("fichier temporaire");
        let secret = "SECRET_HOTE_JSONNET_42";
        std::fs::write(file.path(), secret).expect("écriture");
        let template = format!(
            r#"{{{{#jsonnet}}}}importstr "{}"{{{{/jsonnet}}}}"#,
            file.path().display()
        );
        let mut hbs = HandleBars::new_restricted();
        let result = hbs.render(&template, &serde_json::json!({}));
        assert!(
            matches!(result, Err(vynil_core::Error::HbsRenderError(_))),
            "importstr doit échouer en rendu, obtenu {result:?}"
        );
        assert!(!format!("{result:?}").contains(secret));
    }
}
