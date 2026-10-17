//! Voir `dialect.sdd`.
//!
//! Couche pure : normalisation de dialecte Kubernetes vers JSON Schema 2020-12, mode strict,
//! sémantique `null` de l'apiserver. Aucune E/S, aucune horloge, aucun état.

use serde_json::{Map, Value, json};

/// Positions dont la valeur est un objet de schémas indexés par nom.
const MAP_POSITIONS: &[&str] = &[
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
];

/// Positions dont la valeur est un schéma (ou, par tolérance, un tableau de schémas).
const SCHEMA_POSITIONS: &[&str] = &[
    "items",
    "additionalProperties",
    "additionalItems",
    "contains",
    "propertyNames",
    "not",
    "if",
    "then",
    "else",
    "unevaluatedItems",
    "unevaluatedProperties",
];

/// Positions dont la valeur est un tableau de schémas.
const ARRAY_POSITIONS: &[&str] = &["allOf", "anyOf", "oneOf", "prefixItems"];

/// Visite chaque schéma objet de `schema` (racine incluse) par position de schéma.
/// Les valeurs de `default`, `enum`, `const`, `required`, `example` ne sont jamais parcourues.
fn visit_schemas(schema: &mut Value, visit: &mut dyn FnMut(&mut Map<String, Value>)) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };
    visit(obj);
    for key in MAP_POSITIONS {
        if let Some(Value::Object(children)) = obj.get_mut(*key) {
            for child in children.values_mut() {
                visit_schemas(child, visit);
            }
        }
    }
    for key in SCHEMA_POSITIONS.iter().chain(ARRAY_POSITIONS) {
        match obj.get_mut(*key) {
            Some(Value::Array(items)) => {
                for item in items.iter_mut() {
                    visit_schemas(item, visit);
                }
            }
            Some(child @ Value::Object(_)) => visit_schemas(child, visit),
            _ => {}
        }
    }
}

/// Convertit les extensions Kubernetes en JSON Schema, en place, sur un schéma complet.
pub fn normalize(schema: &mut Value) {
    visit_schemas(schema, &mut normalize_one);
}

fn normalize_one(obj: &mut Map<String, Value>) {
    if obj.remove("nullable") == Some(Value::Bool(true)) {
        make_nullable(obj);
    }
    if obj.remove("x-kubernetes-int-or-string") == Some(Value::Bool(true)) {
        obj.remove("type");
        obj.insert(
            "anyOf".to_owned(),
            json!([{ "type": "integer" }, { "type": "string" }]),
        );
    }
}

/// `type: T` devient `type: [T, "null"]` ; sans `type`, rien à faire.
fn make_nullable(obj: &mut Map<String, Value>) {
    let Some(kind) = obj.get_mut("type") else {
        return;
    };
    if let Some(name) = kind.as_str() {
        *kind = json!([name, "null"]);
    } else if let Some(types) = kind.as_array_mut()
        && !types.iter().any(|t| t.as_str() == Some("null"))
    {
        types.push(json!("null"));
    }
}

/// Pose `additionalProperties: false` sur chaque schéma déclarant `properties`, sauf s'il
/// porte déjà `additionalProperties` ou `x-kubernetes-preserve-unknown-fields: true`.
pub fn strictify(schema: &mut Value) {
    visit_schemas(schema, &mut |obj| {
        let declares_properties = obj.get("properties").is_some_and(Value::is_object);
        let preserves = obj.get("x-kubernetes-preserve-unknown-fields") == Some(&Value::Bool(true));
        if declares_properties && !preserves && !obj.contains_key("additionalProperties") {
            obj.insert("additionalProperties".to_owned(), Value::Bool(false));
        }
    });
}

/// Retire les membres `null` d'un document instance, à toute profondeur, en place. Les
/// éléments `null` d'un tableau sont conservés.
pub fn strip_nulls(doc: &mut Value) {
    match doc {
        Value::Object(members) => {
            members.retain(|_, value| !value.is_null());
            for value in members.values_mut() {
                strip_nulls(value);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                strip_nulls(item);
            }
        }
        _ => {}
    }
}

/// Complète la racine d'un `openAPIV3Schema` de CRD avec `apiVersion`, `kind` et `metadata`
/// lorsqu'ils sont absents, sans écraser une définition existante.
#[must_use]
pub fn crd_root(mut openapi_schema: Value) -> Value {
    if let Some(obj) = openapi_schema.as_object_mut() {
        let properties = obj.entry("properties").or_insert_with(|| json!({}));
        if let Some(properties) = properties.as_object_mut() {
            properties
                .entry("apiVersion")
                .or_insert_with(|| json!({ "type": "string" }));
            properties
                .entry("kind")
                .or_insert_with(|| json!({ "type": "string" }));
            properties
                .entry("metadata")
                .or_insert_with(|| json!({ "type": "object" }));
        }
    }
    openapi_schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nullable_becomes_a_type_union() {
        let mut schema = json!({"type": "string", "nullable": true});
        normalize(&mut schema);
        assert_eq!(schema, json!({"type": ["string", "null"]}));
    }

    #[test]
    fn int_or_string_accepts_exactly_an_integer_or_a_string() {
        for raw in [
            json!({"x-kubernetes-int-or-string": true}),
            json!({"x-kubernetes-int-or-string": true, "type": "string"}),
        ] {
            let mut schema = raw;
            normalize(&mut schema);
            assert!(jsonschema::is_valid(&schema, &json!(8080)));
            assert!(jsonschema::is_valid(&schema, &json!("http")));
            assert!(!jsonschema::is_valid(&schema, &json!([1])));
            assert!(!jsonschema::is_valid(&schema, &json!(true)));
        }
    }

    #[test]
    fn strict_mode_rejects_an_invented_field() {
        let mut schema = json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}
        });
        strictify(&mut schema);
        let validator = jsonschema::validator_for(&schema).expect("schéma compilable");
        let errors: Vec<String> = validator
            .iter_errors(&json!({"a": 1, "c": 2}))
            .map(|e| format!("{}|{e}", e.instance_path))
            .collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        let [only] = errors.as_slice() else {
            return;
        };
        let (path, message) = only.split_once('|').unwrap_or(("?", ""));
        assert_eq!(path, "");
        assert!(message.to_lowercase().contains("additional properties"));
        assert!(message.contains("'c'"));
    }

    #[test]
    fn preserve_unknown_fields_stays_open() {
        let mut schema = json!({
            "type": "object",
            "x-kubernetes-preserve-unknown-fields": true,
            "properties": {"a": {"type": "integer"}}
        });
        strictify(&mut schema);
        assert!(schema.get("additionalProperties").is_none());
    }

    #[test]
    fn a_field_named_properties_is_data() {
        let mut schema = json!({
            "type": "object",
            "properties": {"properties": {"type": "string"}}
        });
        strictify(&mut schema);
        // Le schéma racine déclare `properties` : il est fermé.
        assert_eq!(schema.get("additionalProperties"), Some(&json!(false)));
        // Le conteneur `properties` n'est pas un schéma : aucune clé ne lui est ajoutée.
        let container = schema.get("properties").cloned().unwrap_or(Value::Null);
        assert!(container.get("additionalProperties").is_none());
        // La propriété nommée `properties` reste un schéma de chaîne, inchangé.
        assert_eq!(container.get("properties"), Some(&json!({"type": "string"})));
        // Une instance qui porte un champ `properties` est valide.
        assert!(jsonschema::is_valid(&schema, &json!({"properties": "x"})));
    }

    #[test]
    fn null_on_an_optional_field_means_absent() {
        let mut doc = json!({
            "metadata": {"name": "x", "creationTimestamp": null},
            "spec": {"args": [null]}
        });
        strip_nulls(&mut doc);
        assert_eq!(doc, json!({"metadata": {"name": "x"}, "spec": {"args": [null]}}));
    }

    #[test]
    fn a_crd_root_gains_the_implicit_fields() {
        let openapi = json!({
            "type": "object",
            "properties": {
                "spec": {"type": "object"},
                "status": {"type": "object"}
            }
        });
        let mut schema = crd_root(openapi);
        normalize(&mut schema);
        strictify(&mut schema);
        let doc = json!({
            "apiVersion": "example.org/v1",
            "kind": "Foo",
            "metadata": {"name": "x"},
            "spec": {},
            "status": {}
        });
        assert!(jsonschema::is_valid(&schema, &doc));
    }
}
