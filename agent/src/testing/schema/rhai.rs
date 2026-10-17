//! Voir `rhai.sdd`.

use crate::testing::schema::{
    native::native_k8s_version,
    registry::{SchemaError, SchemaRegistry},
};
use common::rhaihandler::Engine;
use rhai::{Array, Dynamic, EvalAltResult, Map, Position, serde::from_dynamic};
use serde_json::Value;

/// Erreur d'exécution Rhai portant `message`.
fn runtime_error(message: impl Into<String>) -> EvalAltResult {
    EvalAltResult::ErrorRuntime(Dynamic::from(message.into()), Position::NONE)
}

/// Convertit un objet Rhai en `Value` JSON ; la conversion est une copie.
fn to_json(obj: &Dynamic) -> Result<Value, Box<EvalAltResult>> {
    from_dynamic::<Value>(obj)
}

/// `apiVersion` et `kind` d'un objet, s'ils sont chaînes.
fn identity(value: &Value) -> Option<(&str, &str)> {
    Some((value.get("apiVersion")?.as_str()?, value.get("kind")?.as_str()?))
}

/// Carte Rhai `#{ path, message }` pour une erreur de validation.
fn error_map(error: SchemaError) -> Dynamic {
    let mut map = Map::new();
    map.insert("path".into(), Dynamic::from(error.path));
    map.insert("message".into(), Dynamic::from(error.message));
    Dynamic::from(map)
}

/// Valide un objet Rhai et rend le tableau de cartes d'erreurs.
fn validate_to_array(
    registry: &SchemaRegistry,
    obj: &Dynamic,
    strict: bool,
) -> Result<Array, Box<EvalAltResult>> {
    let value = to_json(obj)?;
    let errors = registry
        .validate(&value, strict)
        .map_err(|e| Box::new(runtime_error(e.to_string())))?;
    Ok(errors.into_iter().map(error_map).collect())
}

/// Lit l'option `strict` (défaut `true`) ; les autres clés sont ignorées.
fn strict_option(opts: &Map) -> Result<bool, Box<EvalAltResult>> {
    opts.get("strict").map_or(Ok(true), |value| {
        value.as_bool().map_err(|_| {
            Box::new(runtime_error(
                "schema_validate: the strict option must be a boolean",
            ))
        })
    })
}

/// Enregistre `schema_known`, `schema_validate` (1 ou 2 arguments) et
/// `schema_native_version` dans `engine`. Le registre est capturé par les fermetures.
pub fn schema_rhai_register(engine: &mut Engine, registry: SchemaRegistry) {
    let known_registry = registry.clone();
    engine.register_fn(
        "schema_known",
        move |obj: Dynamic| -> Result<bool, Box<EvalAltResult>> {
            let value = to_json(&obj)?;
            Ok(identity(&value).is_some_and(|(api, kind)| known_registry.known(api, kind)))
        },
    );

    let validate_registry = registry.clone();
    engine.register_fn(
        "schema_validate",
        move |obj: Dynamic| -> Result<Array, Box<EvalAltResult>> {
            validate_to_array(&validate_registry, &obj, true)
        },
    );

    let validate_opts_registry = registry;
    engine.register_fn(
        "schema_validate",
        move |obj: Dynamic, opts: Map| -> Result<Array, Box<EvalAltResult>> {
            let strict = strict_option(&opts)?;
            validate_to_array(&validate_opts_registry, &obj, strict)
        },
    );

    engine.register_fn("schema_native_version", || -> String {
        native_k8s_version().to_owned()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::{Array, Map, Scope, serde::to_dynamic};
    use serde_json::{Value, json};

    fn engine_with_native_registry() -> Engine {
        let registry = SchemaRegistry::new(None, None).expect("registre natif");
        let mut engine = Engine::new();
        schema_rhai_register(&mut engine, registry);
        engine
    }

    /// Deployment valide sauf un champ inventé sur son premier conteneur.
    fn deployment_with_invented_field() -> Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {"name": "web"},
            "spec": {
                "selector": {"matchLabels": {"app": "web"}},
                "template": {
                    "metadata": {"labels": {"app": "web"}},
                    "spec": {"containers": [{
                        "name": "web",
                        "image": "nginx",
                        "inventedField": 1
                    }]}
                }
            }
        })
    }

    fn scope_with(obj: &Value) -> Scope<'static> {
        let mut scope = Scope::new();
        let dynamic = to_dynamic(obj).expect("conversion Rhai");
        scope.push("obj", dynamic);
        scope
    }

    #[test]
    fn validate_returns_path_and_message_maps() {
        let engine = engine_with_native_registry();
        let mut scope = scope_with(&deployment_with_invented_field());
        let result: Array = engine
            .eval_with_scope(&mut scope, "schema_validate(obj)")
            .expect("schema_validate doit s'évaluer");
        assert_eq!(result.len(), 1, "{result:?}");
        let first = result
            .first()
            .and_then(|d| d.clone().try_cast::<Map>())
            .expect("chaque erreur est une carte");
        let path = first
            .get("path")
            .and_then(|d| d.clone().into_string().ok())
            .unwrap_or_default();
        let message = first
            .get("message")
            .and_then(|d| d.clone().into_string().ok())
            .unwrap_or_default();
        assert!(path.ends_with("/containers/0"), "chemin inattendu : {path}");
        assert_ne!(message, "");
    }

    #[test]
    fn non_strict_mode_accepts_unknown_fields() {
        let engine = engine_with_native_registry();
        let mut scope = scope_with(&deployment_with_invented_field());
        let result: Array = engine
            .eval_with_scope(&mut scope, "schema_validate(obj, #{strict: false})")
            .expect("schema_validate avec options doit s'évaluer");
        assert!(result.is_empty(), "{result:?}");
    }

    #[test]
    fn unknown_kind_is_known_false() {
        let engine = engine_with_native_registry();
        let certificate = json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "metadata": {"name": "x"}
        });
        let mut scope = scope_with(&certificate);
        let known: bool = engine
            .eval_with_scope(&mut scope, "schema_known(obj)")
            .expect("schema_known doit s'évaluer");
        assert!(!known);
    }
}
