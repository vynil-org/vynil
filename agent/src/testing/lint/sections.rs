//! Découpage des objets d'un test par section du paquet (spec : `sections.sdd`).

use common::rhaihandler::{Array, Dynamic, Map};

/// Une liste d'objets par section du paquet, plus la liste complète et les objets sans section.
#[derive(Clone, Debug, Default)]
pub struct Sections {
    pub all: Vec<Dynamic>,
    pub crds: Vec<Dynamic>,
    pub systems: Vec<Dynamic>,
    pub befores: Vec<Dynamic>,
    pub vitals: Vec<Dynamic>,
    pub scalables: Vec<Dynamic>,
    pub others: Vec<Dynamic>,
    pub posts: Vec<Dynamic>,
    pub unsectioned: Vec<Dynamic>,
}

/// Clés de section du paquet, dans l'ordre de `to_dynamic`, sans `all` ni `unsectioned`.
const SECTION_KEYS: [&str; 7] = [
    "crds",
    "systems",
    "befores",
    "vitals",
    "scalables",
    "others",
    "posts",
];

impl Sections {
    /// Classe `created` selon les sections déclarées dans `status`.
    #[must_use]
    pub fn from_status(created: &[Dynamic], status: &Dynamic) -> Self {
        let status = status.clone().try_cast::<Map>().unwrap_or_default();
        let mut out = Self {
            all: created.to_vec(),
            ..Self::default()
        };
        let mut designated = vec![false; created.len()];
        for key in SECTION_KEYS {
            let entries = status
                .get(key)
                .and_then(|v| v.clone().try_cast::<Array>())
                .unwrap_or_default();
            let section = out.section_mut(key);
            for (flag, obj) in designated.iter_mut().zip(created) {
                if entries.iter().any(|e| designates(key, e, obj)) {
                    section.push(obj.clone());
                    *flag = true;
                }
            }
        }
        out.unsectioned = created
            .iter()
            .zip(designated)
            .filter(|(_, d)| !d)
            .map(|(obj, _)| obj.clone())
            .collect();
        out
    }

    /// Carte Rhai aux neuf clés, chacune un tableau d'objets.
    pub fn to_dynamic(&self) -> Dynamic {
        let mut map = Map::new();
        let fields: [(&str, &Vec<Dynamic>); 9] = [
            ("all", &self.all),
            ("crds", &self.crds),
            ("systems", &self.systems),
            ("befores", &self.befores),
            ("vitals", &self.vitals),
            ("scalables", &self.scalables),
            ("others", &self.others),
            ("posts", &self.posts),
            ("unsectioned", &self.unsectioned),
        ];
        for (key, objs) in fields {
            map.insert(key.into(), Dynamic::from_array(objs.clone()));
        }
        Dynamic::from_map(map)
    }

    fn section_mut(&mut self, key: &str) -> &mut Vec<Dynamic> {
        match key {
            "crds" => &mut self.crds,
            "systems" => &mut self.systems,
            "befores" => &mut self.befores,
            "vitals" => &mut self.vitals,
            "scalables" => &mut self.scalables,
            "others" => &mut self.others,
            _ => &mut self.posts,
        }
    }
}

/// Vrai quand une entrée de `status` désigne l'objet. Pour `crds`, l'entrée est un nom de
/// `CustomResourceDefinition` ; ailleurs, `kind`, `name` et `namespace` coïncident (absent des
/// deux côtés ou égal) et `apiVersion` n'est comparée que si l'entrée en porte une.
fn designates(key: &str, entry: &Dynamic, obj: &Dynamic) -> bool {
    let Some(obj_map) = obj.clone().try_cast::<Map>() else {
        return false;
    };
    let kind = str_field(&obj_map, "kind");
    let meta = obj_map
        .get("metadata")
        .and_then(|m| m.clone().try_cast::<Map>())
        .unwrap_or_default();
    let name = str_field(&meta, "name");
    if key == "crds" {
        let wanted = entry.clone().into_string().ok();
        return kind.as_deref() == Some("CustomResourceDefinition") && wanted.is_some() && wanted == name;
    }
    let Some(entry_map) = entry.clone().try_cast::<Map>() else {
        return false;
    };
    let api_version = str_field(&entry_map, "apiVersion").or_else(|| str_field(&entry_map, "api_version"));
    let api_matches = api_version.is_none() || api_version == str_field(&obj_map, "apiVersion");
    api_matches
        && str_field(&entry_map, "kind") == kind
        && str_field(&entry_map, "name") == name
        && str_field(&entry_map, "namespace") == str_field(&meta, "namespace")
}

fn str_field(map: &Map, key: &str) -> Option<String> {
    map.get(key).and_then(|v| v.clone().into_string().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::rhaihandler::to_dynamic;
    use serde_json::{Value, json};

    fn obj(api_version: &str, kind: &str, name: &str, namespace: Option<&str>) -> Dynamic {
        let mut metadata = json!({ "name": name });
        if let Some(ns) = namespace {
            metadata["namespace"] = Value::String(ns.to_string());
        }
        to_dynamic(json!({ "apiVersion": api_version, "kind": kind, "metadata": metadata }))
            .expect("objet Rhai")
    }

    fn deployment(name: &str) -> Dynamic {
        obj("apps/v1", "Deployment", name, Some("app"))
    }

    fn configmap(name: &str, namespace: &str) -> Dynamic {
        obj("v1", "ConfigMap", name, Some(namespace))
    }

    fn crd(name: &str) -> Dynamic {
        obj("apiextensions.k8s.io/v1", "CustomResourceDefinition", name, None)
    }

    /// Entrée de `status` : carte `{ apiVersion, kind, name, namespace? }`.
    fn entry(api_version: &str, kind: &str, name: &str, namespace: Option<&str>) -> Value {
        let mut e = json!({ "apiVersion": api_version, "kind": kind, "name": name });
        if let Some(ns) = namespace {
            e["namespace"] = Value::String(ns.to_string());
        }
        e
    }

    fn status(value: &Value) -> Dynamic {
        to_dynamic(value.clone()).expect("status Rhai")
    }

    fn names(list: &[Dynamic]) -> Vec<String> {
        list.iter()
            .map(|d| {
                let map = d.clone().try_cast::<Map>().expect("objet carte");
                let meta = map
                    .get("metadata")
                    .and_then(|m| m.clone().try_cast::<Map>())
                    .expect("metadata");
                meta.get("name")
                    .and_then(|n| n.clone().into_string().ok())
                    .expect("name")
            })
            .collect()
    }

    fn namespace_of(d: &Dynamic) -> Option<String> {
        let map = d.clone().try_cast::<Map>()?;
        let meta = map.get("metadata")?.clone().try_cast::<Map>()?;
        meta.get("namespace")?.clone().into_string().ok()
    }

    fn all_sections_empty_except_all_and_unsectioned(s: &Sections) -> bool {
        s.crds.is_empty()
            && s.systems.is_empty()
            && s.befores.is_empty()
            && s.vitals.is_empty()
            && s.scalables.is_empty()
            && s.others.is_empty()
            && s.posts.is_empty()
    }

    #[test]
    fn objects_are_classified_by_what_the_package_declared() {
        let created = vec![deployment("web"), configmap("cfg", "app")];
        let st = status(&json!({
            "scalables": [entry("apps/v1", "Deployment", "web", Some("app"))],
            "befores": [entry("v1", "ConfigMap", "cfg", Some("app"))],
        }));
        let s = Sections::from_status(&created, &st);
        assert_eq!(names(&s.all), ["web", "cfg"]);
        assert_eq!(names(&s.scalables), ["web"]);
        assert_eq!(names(&s.befores), ["cfg"]);
        assert!(s.others.is_empty());
        assert!(s.unsectioned.is_empty());
    }

    #[test]
    fn kind_alone_never_classifies() {
        let created = vec![deployment("web")];
        let s = Sections::from_status(&created, &status(&json!({})));
        assert!(all_sections_empty_except_all_and_unsectioned(&s));
        assert_eq!(names(&s.all), ["web"]);
        assert_eq!(names(&s.unsectioned), ["web"]);
    }

    #[test]
    fn crds_are_matched_by_name() {
        let created = vec![crd("widgets.example.test")];
        let st = status(&json!({ "crds": ["widgets.example.test"] }));
        let s = Sections::from_status(&created, &st);
        assert_eq!(names(&s.crds), ["widgets.example.test"]);
        assert!(s.unsectioned.is_empty());
    }

    #[test]
    fn namespace_participates_in_identity() {
        let created = vec![configmap("cfg", "a"), configmap("cfg", "b")];
        let st = status(&json!({
            "others": [entry("v1", "ConfigMap", "cfg", Some("b"))],
        }));
        let s = Sections::from_status(&created, &st);
        assert_eq!(s.others.len(), 1);
        assert_eq!(namespace_of(&s.others[0]).as_deref(), Some("b"));
        assert_eq!(s.unsectioned.len(), 1);
        assert_eq!(namespace_of(&s.unsectioned[0]).as_deref(), Some("a"));
    }

    #[test]
    fn a_malformed_status_is_harmless() {
        let created = vec![deployment("web")];
        let as_text = Dynamic::from("pas une carte".to_string());
        let s = Sections::from_status(&created, &as_text);
        assert!(all_sections_empty_except_all_and_unsectioned(&s));
        assert_eq!(names(&s.all), ["web"]);
        assert_eq!(names(&s.unsectioned), ["web"]);

        let befores_int = status(&json!({ "befores": 3 }));
        let s = Sections::from_status(&created, &befores_int);
        assert!(all_sections_empty_except_all_and_unsectioned(&s));
        assert_eq!(names(&s.all), ["web"]);
        assert_eq!(names(&s.unsectioned), ["web"]);
    }

    #[test]
    fn every_key_is_always_present() {
        let map = Sections::default()
            .to_dynamic()
            .try_cast::<Map>()
            .expect("carte Rhai");
        let keys = [
            "all",
            "crds",
            "systems",
            "befores",
            "vitals",
            "scalables",
            "others",
            "posts",
            "unsectioned",
        ];
        assert_eq!(map.len(), keys.len());
        for key in keys {
            let value = map
                .get(key)
                .and_then(|v| v.clone().try_cast::<Array>())
                .unwrap_or_else(|| panic!("clé `{key}` absente ou non tableau"));
            assert!(value.is_empty(), "clé `{key}` non vide");
        }
    }
}
