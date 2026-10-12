//! Test de régression au niveau script : `service/install_crds` sur un répertoire de package
//! contenant un template `.yaml.hbs` dont le rendu produit un document vide entre deux `---`,
//! à côté d'une vraie CRD. La garde `yd::decode` (`agent/scripts/lib/yaml_docs.rhai`) est, sans elle,
//! `crd.kind` lève sur le `()` produit par `yaml_decode_multi` (vynil-core 0.8.0).

use common::rhaihandler::Script;
use rhai::Dynamic;
use std::{
    fs,
    sync::{Arc, Mutex},
};

const CRD_TEMPLATE: &str = "\
---
{{#if enabled}}
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: widgets.example.vynil.test
{{/if}}
---
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: gadgets.example.vynil.test
spec:
  group: example.vynil.test
  scope: Namespaced
  names:
    plural: gadgets
    singular: gadget
    kind: Gadget
  versions: []
";

fn make_service_script(k8s_mocks: Vec<Dynamic>) -> (Script, Arc<Mutex<Vec<Dynamic>>>) {
    let base = env!("CARGO_MANIFEST_DIR");
    let created = Arc::new(Mutex::new(vec![]));
    let script = Script::new_mock(
        vec![format!("{base}/scripts/service"), format!("{base}/scripts/lib")],
        vec![],
        k8s_mocks,
        created.clone(),
    );
    (script, created)
}

#[test]
fn install_crds_ignores_empty_rendered_document_and_applies_real_crd() {
    let pkg = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let crds_dir = pkg.path().join("crds");
    fs::create_dir_all(&crds_dir).unwrap();
    fs::write(crds_dir.join("crds.yaml.hbs"), CRD_TEMPLATE).unwrap();

    let instance_val = serde_json::json!({
        "apiVersion": "vynil.solidite.fr/v1",
        "kind": "ServiceInstance",
        "metadata": { "name": "test-app", "namespace": "default" },
        "spec": { "category": "test", "package": "test-pkg", "options": {} },
        "status": {}
    });
    let context_val = serde_json::json!({
        "package_dir": pkg.path().to_str().unwrap(),
        "config_dir": config.path().to_str().unwrap(),
        "cluster": { "crds": [] }
    });

    let instance_mock: Dynamic =
        serde_json::from_str(&serde_json::to_string(&instance_val).unwrap()).unwrap();
    let (mut rhai, created) = make_service_script(vec![instance_mock]);
    rhai.set_dynamic("context", &context_val);

    let result = rhai.eval(
        r#"
        import "install_crds" as crds;
        crds::run(get_service_instance("default", "test-app"), context);
        get_service_instance("default", "test-app").status.crds
    "#,
    );
    let applied = result.expect("install_crds::run ne doit pas lever sur un document vide");

    // La seule CRD réelle est appliquée, le document vide est ignoré.
    let applied: Vec<String> = applied
        .into_array()
        .expect("status.crds doit être un tableau")
        .into_iter()
        .map(|d| d.into_string().expect("nom de CRD en chaîne"))
        .collect();
    assert_eq!(applied, vec!["gadgets.example.vynil.test".to_string()]);

    let names: Vec<String> = created
        .lock()
        .unwrap()
        .iter()
        .filter_map(|obj| {
            let map = obj.as_map_ref().ok()?;
            if map.get("kind")?.clone().into_string().ok()? != "CustomResourceDefinition" {
                return None;
            }
            let meta = map.get("metadata")?.as_map_ref().ok()?;
            meta.get("name")?.clone().into_string().ok()
        })
        .collect();
    assert_eq!(names, vec!["gadgets.example.vynil.test".to_string()]);
}
