//! Contexte en lecture seule passé au script de lint (spec : `context.sdd`).

use crate::testing::{
    vyniltest::{VynilTest, VynilTestSetRef, appslug},
    vyniltestset::VynilTestSet,
};
use common::vynilpackage::VynilPackageSource;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

fn to_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Valeur sérialisée, ou une carte vide quand l'option est absente.
fn or_empty_map<T: Serialize>(value: Option<&T>) -> Value {
    value.map_or_else(|| json!({}), to_value)
}

fn package_section(package: &VynilPackageSource) -> Value {
    let meta = &package.metadata;
    let mut map = Map::new();
    map.insert("name".to_string(), json!(meta.name));
    map.insert("type".to_string(), to_value(&meta.usage));
    map.insert("category".to_string(), json!(meta.category));
    map.insert("description".to_string(), json!(meta.description));
    if let Some(app_version) = &meta.app_version {
        map.insert("app_version".to_string(), json!(app_version));
    }
    map.insert("features".to_string(), to_value(&meta.features));
    map.insert("options".to_string(), or_empty_map(package.options.as_ref()));
    map.insert("images".to_string(), or_empty_map(package.images.as_ref()));
    map.insert("resources".to_string(), or_empty_map(package.resources.as_ref()));
    Value::Object(map)
}

/// Contexte d'un test résolu : `package`, `instance` et `test`.
#[must_use]
pub fn build_test_context(package: &VynilPackageSource, test: &VynilTest, refs: &[VynilTestSetRef]) -> Value {
    let testsets: Vec<Value> = refs
        .iter()
        .map(|r| json!({"name": r.testSet, "variables": or_empty_map(r.variables.as_ref())}))
        .collect();
    let mut test_section = Map::new();
    test_section.insert("name".to_string(), json!(test.metadata.name));
    if let Some(description) = &test.metadata.description {
        test_section.insert("description".to_string(), json!(description));
    }
    test_section.insert("testsets".to_string(), Value::Array(testsets));
    test_section.insert(
        "definition".to_string(),
        json!({
            "apiVersion": test.apiVersion,
            "kind": test.kind,
            "metadata": to_value(&test.metadata),
            "instance": to_value(&test.instance),
            "mocks": to_value(&test.mocks),
            "asserts": test.asserts.as_ref().map_or_else(|| json!([]), to_value),
        }),
    );
    json!({
        "package": package_section(package),
        "instance": {
            "name": test.instance.name,
            "namespace": test.instance.namespace,
            "appslug": appslug(&package.metadata.name, &test.instance.name),
            "options": or_empty_map(test.instance.options.as_ref()),
        },
        "test": Value::Object(test_section),
    })
}

/// Contexte de suite : `package` et `suite` (tests et testSets chargés, noms exécutés triés).
#[must_use]
pub fn build_suite_context(
    package: &VynilPackageSource,
    tests: &BTreeMap<String, VynilTest>,
    testsets: &BTreeMap<String, VynilTestSet>,
    executed: &[String],
) -> Value {
    let mut executed = executed.to_vec();
    executed.sort();
    json!({
        "package": package_section(package),
        "suite": {
            "tests": to_value(tests),
            "testsets": to_value(testsets),
            "executed": executed,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{build_suite_context, build_test_context};
    use crate::testing::{vyniltest::VynilTest, vyniltestset::VynilTestSet};
    use common::vynilpackage::VynilPackageSource;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn package(usage: &str) -> VynilPackageSource {
        serde_json::from_value(json!({
            "apiVersion": "vinyl.solidite.fr/v1beta1",
            "kind": "Package",
            "metadata": {
                "name": "traefik",
                "category": "core",
                "description": "Ingress controller",
                "type": usage,
                "features": []
            },
            "requirements": [],
            "options": {"replicas": 2}
        }))
        .unwrap()
    }

    fn test_doc(name: &str, asserts: &[&str]) -> VynilTest {
        let asserts: Vec<serde_json::Value> = asserts
            .iter()
            .map(|a| json!({"name": a, "selector": {}, "matcher": "all"}))
            .collect();
        serde_json::from_value(json!({
            "apiVersion": "vinyl.solidite.fr/v1beta1",
            "kind": "Test",
            "metadata": {"name": name, "description": "a test"},
            "instance": {"name": "traefik", "namespace": "traefik", "options": {"replicas": 3}},
            "testSets": [{"testSet": "ts", "variables": {"host": "example.org"}}],
            "asserts": asserts
        }))
        .unwrap()
    }

    fn testset_doc(name: &str) -> VynilTestSet {
        serde_json::from_value(json!({
            "apiVersion": "vinyl.solidite.fr/v1beta1",
            "kind": "TestSet",
            "metadata": {"name": name}
        }))
        .unwrap()
    }

    #[test]
    fn package_and_instance_facts_are_exposed() {
        let pkg = package("system");
        let test = test_doc("default", &[]);
        let ctx = build_test_context(&pkg, &test, &[]);
        assert_eq!(ctx["package"]["type"], "system");
        assert_eq!(ctx["package"]["name"], "traefik");
        assert_eq!(ctx["instance"]["appslug"], "traefik");
        assert_eq!(ctx["test"]["name"], "default");
    }

    #[test]
    fn the_test_definition_is_the_resolved_one() {
        let pkg = package("tenant");
        let test = test_doc("default", &["own-a", "ts-a", "ts-b"]);
        let refs = test.testSets.clone().unwrap_or_default();
        let ctx = build_test_context(&pkg, &test, &refs);
        let names: Vec<&str> = ctx["test"]["definition"]["asserts"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(names, ["own-a", "ts-a", "ts-b"]);
        assert_eq!(ctx["test"]["testsets"][0]["name"], "ts");
        assert_eq!(ctx["test"]["testsets"][0]["variables"]["host"], "example.org");
    }

    #[test]
    fn the_suite_context_lists_every_loaded_test() {
        let pkg = package("tenant");
        let mut tests = BTreeMap::new();
        tests.insert("t1".to_string(), test_doc("t1", &[]));
        tests.insert("t2".to_string(), test_doc("t2", &[]));
        let mut testsets = BTreeMap::new();
        testsets.insert("ts".to_string(), testset_doc("ts"));
        let ctx = build_suite_context(&pkg, &tests, &testsets, &["t1".to_string()]);
        assert_eq!(
            ctx["suite"]["tests"].as_object().map(serde_json::Map::len),
            Some(2)
        );
        assert_eq!(ctx["suite"]["executed"], json!(["t1"]));
        assert_eq!(
            ctx["suite"]["testsets"].as_object().map(serde_json::Map::len),
            Some(1)
        );
    }

    #[test]
    fn context_is_deterministic() {
        let pkg = package("service");
        let test = test_doc("default", &["a"]);
        let refs = test.testSets.clone().unwrap_or_default();
        assert_eq!(
            build_test_context(&pkg, &test, &refs),
            build_test_context(&pkg, &test, &refs)
        );
    }
}
