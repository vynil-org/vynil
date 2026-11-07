//! Voir `native.sdd`.
//!
//! Table des types Kubernetes natifs : schéma racine tel que `schemars` le produit pour les
//! types de `k8s-openapi`, sans document `OpenAPI` embarqué.

use k8s_openapi::{
    api::{
        admissionregistration::v1 as admission_v1, apps::v1 as apps_v1, autoscaling::v2 as hpa_v2,
        batch::v1 as batch_v1, coordination::v1 as coordination_v1, core::v1 as core_v1,
        discovery::v1 as discovery_v1, networking::v1 as networking_v1, policy::v1 as policy_v1,
        rbac::v1 as rbac_v1, scheduling::v1 as scheduling_v1, storage::v1 as storage_v1,
    },
    apiextensions_apiserver::pkg::apis::apiextensions::v1 as apiext_v1,
    schemars::{JsonSchema, schema_for},
};
use serde_json::Value;

/// Version mineure de Kubernetes de la feature `k8s-openapi` `latest` du workspace (`v1_35`).
const NATIVE_K8S_VERSION: &str = "1.35";

/// Schéma racine autonome de `T`, sortie brute de `schemars` (JSON Schema 2020-12).
fn schema_of<T: JsonSchema>() -> Option<Value> {
    serde_json::to_value(schema_for!(T)).ok()
}

/// Pointeur de la définition `JSON` dans le schéma d'une CRD.
const JSON_DEF: &str = "/$defs/io.k8s.apiextensions-apiserver.pkg.apis.apiextensions.v1.JSON";

/// `schemars` décrit le type `JSON` des CRD — `default`, `example`, chaque élément de `enum`,
/// `const` — comme un objet, ce qui rejetait `default: fast`. La
/// définition ne porte alors aucune contrainte de type : tout JSON est valide, sans marqueur
/// à normaliser par ./dialect.sdd.
fn relax_json(mut schema: Value) -> Value {
    if let Some(json) = schema.pointer_mut(JSON_DEF).and_then(Value::as_object_mut) {
        json.remove("type");
    }
    schema
}

/// Schéma natif brut de `apiVersion`/`kind`, ou `None` pour un couple absent de la table.
#[must_use]
pub fn native_schema(api_version: &str, kind: &str) -> Option<Value> {
    let raw = match (api_version, kind) {
        ("v1", "Namespace") => schema_of::<core_v1::Namespace>(),
        ("v1", "ConfigMap") => schema_of::<core_v1::ConfigMap>(),
        ("v1", "Secret") => schema_of::<core_v1::Secret>(),
        ("v1", "Service") => schema_of::<core_v1::Service>(),
        ("v1", "ServiceAccount") => schema_of::<core_v1::ServiceAccount>(),
        ("v1", "PersistentVolumeClaim") => schema_of::<core_v1::PersistentVolumeClaim>(),
        ("v1", "PersistentVolume") => schema_of::<core_v1::PersistentVolume>(),
        ("v1", "Pod") => schema_of::<core_v1::Pod>(),
        ("v1", "Endpoints") => schema_of::<core_v1::Endpoints>(),
        ("v1", "ResourceQuota") => schema_of::<core_v1::ResourceQuota>(),
        ("v1", "LimitRange") => schema_of::<core_v1::LimitRange>(),
        ("apps/v1", "Deployment") => schema_of::<apps_v1::Deployment>(),
        ("apps/v1", "StatefulSet") => schema_of::<apps_v1::StatefulSet>(),
        ("apps/v1", "DaemonSet") => schema_of::<apps_v1::DaemonSet>(),
        ("apps/v1", "ReplicaSet") => schema_of::<apps_v1::ReplicaSet>(),
        ("batch/v1", "Job") => schema_of::<batch_v1::Job>(),
        ("batch/v1", "CronJob") => schema_of::<batch_v1::CronJob>(),
        ("networking.k8s.io/v1", "Ingress") => schema_of::<networking_v1::Ingress>(),
        ("networking.k8s.io/v1", "IngressClass") => schema_of::<networking_v1::IngressClass>(),
        ("networking.k8s.io/v1", "NetworkPolicy") => schema_of::<networking_v1::NetworkPolicy>(),
        ("rbac.authorization.k8s.io/v1", "Role") => schema_of::<rbac_v1::Role>(),
        ("rbac.authorization.k8s.io/v1", "ClusterRole") => schema_of::<rbac_v1::ClusterRole>(),
        ("rbac.authorization.k8s.io/v1", "RoleBinding") => schema_of::<rbac_v1::RoleBinding>(),
        ("rbac.authorization.k8s.io/v1", "ClusterRoleBinding") => schema_of::<rbac_v1::ClusterRoleBinding>(),
        ("autoscaling/v2", "HorizontalPodAutoscaler") => schema_of::<hpa_v2::HorizontalPodAutoscaler>(),
        ("policy/v1", "PodDisruptionBudget") => schema_of::<policy_v1::PodDisruptionBudget>(),
        ("storage.k8s.io/v1", "StorageClass") => schema_of::<storage_v1::StorageClass>(),
        ("scheduling.k8s.io/v1", "PriorityClass") => schema_of::<scheduling_v1::PriorityClass>(),
        ("apiextensions.k8s.io/v1", "CustomResourceDefinition") => {
            schema_of::<apiext_v1::CustomResourceDefinition>()
        }
        ("admissionregistration.k8s.io/v1", "ValidatingWebhookConfiguration") => {
            schema_of::<admission_v1::ValidatingWebhookConfiguration>()
        }
        ("admissionregistration.k8s.io/v1", "MutatingWebhookConfiguration") => {
            schema_of::<admission_v1::MutatingWebhookConfiguration>()
        }
        ("coordination.k8s.io/v1", "Lease") => schema_of::<coordination_v1::Lease>(),
        ("discovery.k8s.io/v1", "EndpointSlice") => schema_of::<discovery_v1::EndpointSlice>(),
        _ => None,
    }?;
    Some(relax_json(raw))
}

/// Version mineure de Kubernetes dont proviennent les schémas natifs, pour les messages.
#[must_use]
pub const fn native_k8s_version() -> &'static str {
    NATIVE_K8S_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::schema::dialect::{normalize, strictify};
    use serde_json::json;

    /// Couples `apiVersion`/`kind` de l'ensemble minimal de native.sdd.
    const MINIMUM: &[(&str, &str)] = &[
        ("v1", "Namespace"),
        ("v1", "ConfigMap"),
        ("v1", "Secret"),
        ("v1", "Service"),
        ("v1", "ServiceAccount"),
        ("v1", "PersistentVolumeClaim"),
        ("v1", "PersistentVolume"),
        ("v1", "Pod"),
        ("v1", "Endpoints"),
        ("v1", "ResourceQuota"),
        ("v1", "LimitRange"),
        ("apps/v1", "Deployment"),
        ("apps/v1", "StatefulSet"),
        ("apps/v1", "DaemonSet"),
        ("apps/v1", "ReplicaSet"),
        ("batch/v1", "Job"),
        ("batch/v1", "CronJob"),
        ("networking.k8s.io/v1", "Ingress"),
        ("networking.k8s.io/v1", "IngressClass"),
        ("networking.k8s.io/v1", "NetworkPolicy"),
        ("rbac.authorization.k8s.io/v1", "Role"),
        ("rbac.authorization.k8s.io/v1", "ClusterRole"),
        ("rbac.authorization.k8s.io/v1", "RoleBinding"),
        ("rbac.authorization.k8s.io/v1", "ClusterRoleBinding"),
        ("autoscaling/v2", "HorizontalPodAutoscaler"),
        ("policy/v1", "PodDisruptionBudget"),
        ("storage.k8s.io/v1", "StorageClass"),
        ("scheduling.k8s.io/v1", "PriorityClass"),
        ("apiextensions.k8s.io/v1", "CustomResourceDefinition"),
        (
            "admissionregistration.k8s.io/v1",
            "ValidatingWebhookConfiguration",
        ),
        ("admissionregistration.k8s.io/v1", "MutatingWebhookConfiguration"),
        ("coordination.k8s.io/v1", "Lease"),
        ("discovery.k8s.io/v1", "EndpointSlice"),
    ];

    fn deployment(cpu: &Value) -> Value {
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
                        "resources": {"limits": {"cpu": cpu}}
                    }]}
                }
            }
        })
    }

    /// Schéma natif passé par le même traitement de dialecte que le registre.
    fn compiled_deployment(strict: bool) -> jsonschema::Validator {
        let Some(mut schema) = native_schema("apps/v1", "Deployment") else {
            return jsonschema::validator_for(&json!(false)).expect("schéma trivial");
        };
        normalize(&mut schema);
        if strict {
            strictify(&mut schema);
        }
        jsonschema::validator_for(&schema).expect("schéma Deployment compilable")
    }

    /// Schéma natif de `CustomResourceDefinition` passé par le même traitement de dialecte
    /// que le registre.
    fn compiled_crd(strict: bool) -> jsonschema::Validator {
        let Some(mut schema) = native_schema("apiextensions.k8s.io/v1", "CustomResourceDefinition") else {
            return jsonschema::validator_for(&json!(false)).expect("schéma trivial");
        };
        normalize(&mut schema);
        if strict {
            strictify(&mut schema);
        }
        jsonschema::validator_for(&schema).expect("schéma CRD compilable")
    }

    /// CRD `widgets.example.com` au socle fixe, pour le `spec` donné.
    fn crd(spec: &Value) -> Value {
        json!({
            "apiVersion": "apiextensions.k8s.io/v1",
            "kind": "CustomResourceDefinition",
            "metadata": {"name": "widgets.example.com"},
            "spec": spec
        })
    }

    /// Les erreurs de `doc` contre `validator`, sous forme `chemin|message`.
    fn errors(validator: &jsonschema::Validator, doc: &Value) -> Vec<String> {
        validator
            .iter_errors(doc)
            .map(|e| format!("{}|{e}", e.instance_path))
            .collect()
    }

    /// CRD de fixture écrite pour ce dépôt, reprenant les motifs cnpg/cert-manager :
    /// `default`, `enum`, `items` imbriqués, `additionalProperties`,
    /// `x-kubernetes-preserve-unknown-fields`.
    fn realistic_crd() -> Value {
        crd(&json!({
            "group": "postgres.example.org",
            "names": {"kind": "Cluster", "plural": "clusters", "shortNames": ["cluster"]},
            "scope": "Namespaced",
            "versions": [{
                "name": "v1",
                "served": true,
                "storage": true,
                "schema": {"openAPIV3Schema": {
                    "type": "object",
                    "description": "Fixture de CRD de ce dépôt, motifs cnpg et cert-manager.",
                    "additionalProperties": false,
                    "properties": {
                        "spec": {"type": "object", "properties": {
                            "instances": {"type": "integer", "format": "int32", "minimum": 1,
        "default": 1},
                            "logLevel": {"type": "string", "default": "info",
                                "enum": ["debug", "info", "error"]},
                            "primaryUpdateMethod": {"type": "string", "default": "switchover",
                                "enum": ["switchover", "stop-start"]},
                            "storage": {"type": "object", "properties": {
                                "size": {"type": "string", "default": "1Gi"},
                                "resizeMode": {"type": "string", "default": "auto",
                                    "enum": ["online", "offline", "auto"]},
                                "parameters": {"type": "object",
                                    "additionalProperties": {"type": "string"}}
                            }},
                            "replicaClusters": {"type": "array", "items": {"type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "inherit": {"type": "array", "items": {
                                        "type": "string",
                                        "enum": ["metadata", "spec", "status"]
                                    }}
                                }}},
                            "resources": {"type": "object",
                                "x-kubernetes-preserve-unknown-fields": true},
                            "affinity": {"type": "object",
                                "x-kubernetes-preserve-unknown-fields": true}
                        }},
                        "status": {"type": "object", "x-kubernetes-preserve-unknown-fields": true}
                    }
                }}
            }]
        }))
    }

    #[test]
    fn every_minimum_resource_resolves() {
        for (api_version, kind) in MINIMUM {
            let schema = native_schema(api_version, kind)
                .unwrap_or_else(|| panic!("{api_version} {kind} absent de la table"));
            let properties = schema
                .pointer("/properties")
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("{api_version} {kind} sans properties racine"));
            for field in ["apiVersion", "kind", "metadata"] {
                assert!(
                    properties.contains_key(field),
                    "{api_version} {kind}: champ {field} absent de la racine"
                );
            }
        }
    }

    #[test]
    fn unknown_pair_is_not_an_error() {
        assert!(native_schema("cert-manager.io/v1", "Certificate").is_none());
    }

    #[test]
    fn quantity_accepts_integer_and_string() {
        for strict in [true, false] {
            let validator = compiled_deployment(strict);
            for accepted in [json!(1), json!("0.5"), json!("500m")] {
                assert!(
                    validator.is_valid(&deployment(&accepted)),
                    "strict={strict}: {accepted} doit être accepté"
                );
            }
            for rejected in [json!([1]), json!(true)] {
                assert!(
                    !validator.is_valid(&deployment(&rejected)),
                    "strict={strict}: {rejected} doit être rejeté"
                );
            }
        }
    }

    #[test]
    fn quantity_definition_carries_the_upstream_marker() {
        let schema = native_schema("apps/v1", "Deployment").expect("schéma Deployment");
        let quantity = schema
            .pointer("/$defs/io.k8s.apimachinery.pkg.api.resource.Quantity")
            .expect("définition Quantity introuvable : la clé `$defs` a changé");
        assert_eq!(
            quantity.get("x-kubernetes-int-or-string"),
            Some(&Value::Bool(true)),
            "k8s-openapi n'émet plus le marqueur int-or-string : {quantity}"
        );
    }

    #[test]
    fn json_definition_is_found_and_relaxed() {
        let schema = native_schema("apiextensions.k8s.io/v1", "CustomResourceDefinition")
            .expect("schéma CustomResourceDefinition");
        let json_def = schema
            .pointer(JSON_DEF)
            .expect("définition JSON introuvable : la clé `$defs` a changé");
        assert!(json_def.get("type").is_none(), "type résiduel : {json_def}");
    }

    #[test]
    fn a_crd_with_default_and_enum_values_is_valid() {
        // La CRD `widgets.example.com` de l'issue #305.
        let widget = crd(&json!({
            "group": "example.com",
            "names": {"kind": "Widget", "plural": "widgets"},
            "scope": "Namespaced",
            "versions": [{
                "name": "v1",
                "served": true,
                "storage": true,
                "schema": {"openAPIV3Schema": {
                    "type": "object",
                    "properties": {"spec": {"type": "object", "properties": {
                        "mode": {"type": "string", "default": "fast", "enum": ["fast", "slow"]},
                        "ports": {"type": "array", "items": {"type": "object", "properties": {
                            "port": {"type": "integer"},
                            "protocol": {"type": "string", "default": "TCP"}
                        }}}
                    }}}
                }}
            }]
        }));
        for strict in [true, false] {
            let found = errors(&compiled_crd(strict), &widget);
            assert!(found.is_empty(), "strict={strict}: {found:?}");
        }
    }

    #[test]
    fn default_and_enum_accept_any_json_value() {
        for default in [
            json!(7),
            json!(true),
            json!([1, "two"]),
            json!({"a": 1}),
            Value::Null,
        ] {
            let instance = crd(&json!({
                "group": "example.com",
                "names": {"kind": "Widget", "plural": "widgets"},
                "scope": "Namespaced",
                "versions": [{
                    "name": "v1",
                    "served": true,
                    "storage": true,
                    "schema": {"openAPIV3Schema": {
                        "type": "object",
                        "properties": {"spec": {"type": "object", "properties": {
                            "threshold": {"default": default}
                        }}}
                    }}
                }]
            }));
            for strict in [true, false] {
                let found = errors(&compiled_crd(strict), &instance);
                assert!(found.is_empty(), "strict={strict}: default={default} : {found:?}");
            }
        }
        let mixed = crd(&json!({
            "group": "example.com",
            "names": {"kind": "Widget", "plural": "widgets"},
            "scope": "Namespaced",
            "versions": [{
                "name": "v1",
                "served": true,
                "storage": true,
                "schema": {"openAPIV3Schema": {
                    "type": "object",
                    "properties": {"spec": {"type": "object", "properties": {
                        "levels": {"enum": ["low", 1, "high", 2]}
                    }}}
                }}
            }]
        }));
        for strict in [true, false] {
            let found = errors(&compiled_crd(strict), &mixed);
            assert!(found.is_empty(), "strict={strict}: enum mixte : {found:?}");
        }
    }

    #[test]
    fn a_real_world_crd_yields_no_finding() {
        let fixture = realistic_crd();
        for strict in [true, false] {
            let found = errors(&compiled_crd(strict), &fixture);
            assert!(found.is_empty(), "strict={strict}: {found:?}");
        }
    }

    #[test]
    fn a_malformed_crd_is_still_rejected() {
        let without_names = crd(&json!({
            "group": "example.com",
            "scope": "Namespaced",
            "versions": [{"name": "v1", "served": true, "storage": true}]
        }));
        let versions_not_array = crd(&json!({
            "group": "example.com",
            "names": {"kind": "Widget", "plural": "widgets"},
            "scope": "Namespaced",
            "versions": "v1"
        }));
        for strict in [true, false] {
            let validator = compiled_crd(strict);
            for (label, malformed) in [
                ("spec.names absent", &without_names),
                ("spec.versions non tableau", &versions_not_array),
            ] {
                assert!(
                    !errors(&validator, malformed).is_empty(),
                    "strict={strict}: la CRD à {label} doit rendre au moins une erreur"
                );
            }
        }
        // Sensibilité du gardien : un schéma sans aucune contrainte accepte les deux CRD
        // malformées, donc l'assertion ci-dessus rougit si toutes les contraintes
        // étaient supprimées par erreur.
        let permissive = jsonschema::validator_for(&json!({})).expect("schéma vide compilable");
        assert!(
            permissive.is_valid(&without_names) && permissive.is_valid(&versions_not_array),
            "le gardien doit être sensible à une suppression totale des contraintes"
        );
    }
}
