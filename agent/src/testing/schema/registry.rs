//! Voir `registry.sdd`.

use crate::testing::schema::{
    dialect::{crd_root, normalize, strictify, strip_nulls},
    native::{native_k8s_version, native_schema},
};
use common::Error;
use jsonschema::{Draft, Validator};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

/// Clé d'un schéma : `apiVersion` tel qu'écrit dans un manifeste, puis `kind`.
type Key = (String, String);

/// Erreur de validation : chemin d'instance pointé et message du validateur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    /// Chemin d'instance pointé séparé par `/`, vide pour la racine.
    pub path: String,
    /// Message du validateur.
    pub message: String,
}

/// Schéma source non encore normalisé.
#[derive(Debug, Clone)]
struct Source {
    /// Schéma brut, avant le pipeline de dialecte.
    schema: Value,
    /// Fichier d'origine, pour les messages d'erreur.
    origin: String,
    /// Vrai pour un `openAPIV3Schema` de CRD (racine à compléter par `crd_root`).
    crd: bool,
}

#[derive(Debug)]
struct Inner {
    /// Schémas des CRD du package : prioritaires.
    package: BTreeMap<Key, Source>,
    /// Schémas de la distribution : après le package, avant la table native.
    distribution: BTreeMap<Key, Source>,
    /// Validateurs compilés par (`apiVersion`, `kind`, strict).
    compiled: Mutex<HashMap<(Key, bool), Arc<Validator>>>,
}

/// Registre des schémas, `Clone` bon marché (état partagé derrière `Arc`).
#[derive(Clone)]
pub struct SchemaRegistry {
    inner: Arc<Inner>,
}

impl fmt::Debug for SchemaRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchemaRegistry")
            .field("package", &self.inner.package.len())
            .field("distribution", &self.inner.distribution.len())
            .finish_non_exhaustive()
    }
}

impl SchemaRegistry {
    /// Charge les sources de schémas.
    ///
    /// `package_crds` est lu sans récursion, `schemas_dir` récursivement. Un chemin absent
    /// n'apporte aucune source.
    ///
    /// # Errors
    /// Répertoire illisible, fichier malformé, ou fichier de schéma mal nommé : le
    /// chargement échoue entièrement, nommant le fichier fautif.
    pub fn new(package_crds: Option<&Path>, schemas_dir: Option<&Path>) -> common::Result<Self> {
        let mut package = BTreeMap::new();
        if let Some(dir) = package_crds.filter(|d| d.is_dir()) {
            let mut files = Vec::new();
            collect_files(dir, false, &mut files)?;
            files.sort();
            for file in &files {
                if is_yaml(file) {
                    load_crd_file(file, &mut package)?;
                }
            }
        }

        let mut distribution = BTreeMap::new();
        if let Some(root) = schemas_dir.filter(|d| d.is_dir()) {
            let mut files = Vec::new();
            collect_files(root, true, &mut files)?;
            files.sort();
            for file in &files {
                if is_json(file) {
                    load_json_schema(root, file, &mut distribution)?;
                } else if is_yaml(file) {
                    load_crd_file(file, &mut distribution)?;
                }
            }
        }

        Ok(Self {
            inner: Arc::new(Inner {
                package,
                distribution,
                compiled: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// Vrai si un schéma existe pour `apiVersion`/`kind` dans une des trois sources.
    #[must_use]
    pub fn known(&self, api_version: &str, kind: &str) -> bool {
        let key = (api_version.to_owned(), kind.to_owned());
        self.inner.package.contains_key(&key)
            || self.inner.distribution.contains_key(&key)
            || native_schema(api_version, kind).is_some()
    }

    /// Valide `obj` contre le schéma de son `apiVersion`/`kind`.
    ///
    /// Un objet dont le schéma est inconnu est conforme. `strict` choisit le mode strict de
    /// ./dialect.sdd. L'objet de l'appelant n'est pas modifié.
    ///
    /// # Errors
    /// Schéma qui ne compile pas, nommant le couple et sa source.
    pub fn validate(&self, obj: &Value, strict: bool) -> common::Result<Vec<SchemaError>> {
        let (Some(api_version), Some(kind)) = (
            obj.get("apiVersion").and_then(Value::as_str),
            obj.get("kind").and_then(Value::as_str),
        ) else {
            return Ok(vec![SchemaError {
                path: String::new(),
                message: "les champs apiVersion et kind sont requis (chaînes)".to_owned(),
            }]);
        };
        if !self.known(api_version, kind) {
            return Ok(Vec::new());
        }
        let validator = self.validator(api_version, kind, strict)?;
        let mut doc = obj.clone();
        strip_nulls(&mut doc);
        Ok(validator
            .iter_errors(&doc)
            .map(|error| SchemaError {
                path: error.instance_path.to_string(),
                message: error.to_string(),
            })
            .collect())
    }

    /// Validateur compilé pour le couple et le mode, compilé au premier besoin.
    fn validator(&self, api_version: &str, kind: &str, strict: bool) -> common::Result<Arc<Validator>> {
        let cache_key = ((api_version.to_owned(), kind.to_owned()), strict);
        let cached = self
            .inner
            .compiled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&cache_key)
            .map(Arc::clone);
        if let Some(validator) = cached {
            return Ok(validator);
        }
        // Compilation hors verrou : deux appels concurrents compilent au pire deux fois le
        // même validateur, sans effet de bord.
        let validator = Arc::new(self.compile(api_version, kind, strict)?);
        self.inner
            .compiled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(cache_key, Arc::clone(&validator));
        Ok(validator)
    }

    fn compile(&self, api_version: &str, kind: &str, strict: bool) -> common::Result<Validator> {
        let key = (api_version.to_owned(), kind.to_owned());
        let (raw, crd, origin) = if let Some(source) = self.inner.package.get(&key) {
            (source.schema.clone(), source.crd, source.origin.clone())
        } else if let Some(source) = self.inner.distribution.get(&key) {
            (source.schema.clone(), source.crd, source.origin.clone())
        } else if let Some(schema) = native_schema(api_version, kind) {
            let origin = format!("table native, Kubernetes {}", native_k8s_version());
            (schema, false, origin)
        } else {
            return Err(Error::Other(format!("no schema for {api_version} {kind}")));
        };
        let schema = prepare(raw, crd, strict);
        jsonschema::options()
            .with_draft(Draft::Draft202012)
            .build(&schema)
            .map_err(|e| Error::Other(format!("invalid schema for {api_version} {kind} ({origin}): {e}")))
    }

    /// Nombre de validateurs compilés, pour vérifier le cache.
    #[cfg(test)]
    #[must_use]
    pub fn compiled_count(&self) -> usize {
        self.inner
            .compiled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

/// Pipeline de dialecte : `crd_root` (CRD seulement), `normalize`, puis `strictify` en mode
/// strict. Le mode strict ne saute jamais la normalisation.
fn prepare(raw: Value, crd: bool, strict: bool) -> Value {
    let mut schema = if crd { crd_root(raw) } else { raw };
    normalize(&mut schema);
    if strict {
        strictify(&mut schema);
    }
    schema
}

fn is_yaml(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("yml" | "yaml"))
}

fn is_json(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("json")
}

/// Collecte les fichiers réguliers de `dir`, récursivement si demandé. Les liens symboliques
/// ne sont pas suivis.
fn collect_files(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> common::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            if recursive {
                collect_files(&path, true, out)?;
            }
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Charge chaque document `CustomResourceDefinition` d'un fichier YAML multi-documents.
/// Les autres documents sont ignorés ; le premier couple rencontré l'emporte.
fn load_crd_file(path: &Path, into: &mut BTreeMap<Key, Source>) -> common::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let origin = path.display().to_string();
    for document in serde_yaml::Deserializer::from_str(&text) {
        let doc = Value::deserialize(document).map_err(|e| Error::YamlError(format!("{origin} : {e}")))?;
        if doc.get("kind").and_then(Value::as_str) != Some("CustomResourceDefinition") {
            continue;
        }
        let group = doc.pointer("/spec/group").and_then(Value::as_str);
        let kind = doc.pointer("/spec/names/kind").and_then(Value::as_str);
        let (Some(group), Some(kind)) = (group, kind) else {
            return Err(Error::Other(format!(
                "{origin} : CRD sans spec.group ou spec.names.kind"
            )));
        };
        let versions = doc
            .pointer("/spec/versions")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for version in versions {
            let name = version.get("name").and_then(Value::as_str);
            let schema = version.pointer("/schema/openAPIV3Schema");
            let (Some(name), Some(schema)) = (name, schema) else {
                continue;
            };
            into.entry((format!("{group}/{name}"), kind.to_owned()))
                .or_insert_with(|| Source {
                    schema: schema.clone(),
                    origin: origin.clone(),
                    crd: true,
                });
        }
    }
    Ok(())
}

/// Charge un schéma JSON rangé `<groupe>/<Kind>_<version>.json` (`core` pour le groupe vide).
fn load_json_schema(root: &Path, path: &Path, into: &mut BTreeMap<Key, Source>) -> common::Result<()> {
    let origin = path.display().to_string();
    let naming_error = || Error::Other(format!("{origin}: expected name <group>/<Kind>_<version>.json"));
    let group = match path.parent() {
        Some(parent) if parent != root => parent.file_name().and_then(|n| n.to_str()),
        _ => None,
    }
    .ok_or_else(naming_error)?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(naming_error)?;
    let (kind, version) = stem.rsplit_once('_').ok_or_else(naming_error)?;
    let api_version = if group == "core" {
        version.to_owned()
    } else {
        format!("{group}/{version}")
    };
    let text = std::fs::read_to_string(path)?;
    let schema: Value =
        serde_json::from_str(&text).map_err(|e| Error::YamlError(format!("{origin} : {e}")))?;
    into.entry((api_version, kind.to_owned()))
        .or_insert_with(|| Source {
            schema,
            origin,
            crd: false,
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::fs;
    use tempfile::TempDir;

    /// CRD `example.org/v1` `Foo` : `spec.size` doit être un entier.
    const FOO_CRD: &str = r"
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: foos.example.org
spec:
  group: example.org
  names:
    kind: Foo
    plural: foos
  scope: Namespaced
  versions:
    - name: v1
      served: true
      storage: true
      schema:
        openAPIV3Schema:
          type: object
          properties:
            spec:
              type: object
              properties:
                size:
                  type: integer
";

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("création du répertoire de fixture");
        }
        fs::write(path, content).expect("écriture de fixture");
    }

    fn deployment(metadata: &Value) -> Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": metadata,
            "spec": {
                "selector": {"matchLabels": {"app": "web"}},
                "template": {
                    "metadata": {"labels": {"app": "web"}},
                    "spec": {"containers": [{"name": "web", "image": "nginx"}]}
                }
            }
        })
    }

    #[test]
    fn package_crd_wins_over_the_distribution_directory() {
        let dir = TempDir::new().expect("tempdir");
        let crds = dir.path().join("crds");
        let schemas = dir.path().join("schemas");
        write(&crds.join("foo.yaml"), FOO_CRD);
        // Distribution : exige `spec.size` en chaîne, contredit la CRD du package.
        write(
            &schemas.join("example.org/Foo_v1.json"),
            r#"{"type":"object","properties":{"spec":{"type":"object",
               "properties":{"size":{"type":"string"}}}}}"#,
        );
        let registry = SchemaRegistry::new(Some(&crds), Some(&schemas)).expect("chargement");
        let obj = json!({
            "apiVersion": "example.org/v1",
            "kind": "Foo",
            "metadata": {"name": "x"},
            "spec": {"size": 1}
        });
        let errors = registry.validate(&obj, false).expect("validation");
        assert!(
            errors.is_empty(),
            "la CRD du package doit l'emporter : {errors:?}"
        );
    }

    #[test]
    fn distribution_directory_hardens_a_native_type() {
        let dir = TempDir::new().expect("tempdir");
        let schemas = dir.path().join("schemas");
        write(
            &schemas.join("apps/Deployment_v1.json"),
            r#"{"type":"object","required":["metadata"],"properties":{"metadata":
               {"type":"object","required":["labels"]}}}"#,
        );
        let registry = SchemaRegistry::new(None, Some(&schemas)).expect("chargement");
        let obj = deployment(&json!({"name": "web"}));
        let errors = registry.validate(&obj, false).expect("validation");
        assert_eq!(errors.len(), 1, "{errors:?}");
        let message = errors.first().map(|e| e.message.clone()).unwrap_or_default();
        assert!(message.contains("labels"), "{message}");
    }

    #[test]
    fn broken_schema_file_fails_loading() {
        let dir = TempDir::new().expect("tempdir");
        let schemas = dir.path().join("schemas");
        write(&schemas.join("apps/Deployment_v1.json"), "{ pas du json");
        let Err(error) = SchemaRegistry::new(None, Some(&schemas)) else {
            panic!("un fichier de schéma cassé doit faire échouer le chargement");
        };
        assert!(
            error.to_string().contains("Deployment_v1.json"),
            "l'erreur doit nommer le fichier : {error}"
        );
    }

    #[test]
    fn unknown_kind_is_a_fact() {
        let registry = SchemaRegistry::new(None, None).expect("chargement");
        let obj = json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "metadata": {"name": "x"},
            "spec": {"anything": true}
        });
        assert!(!registry.known("cert-manager.io/v1", "Certificate"));
        let errors = registry.validate(&obj, true).expect("validation");
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn the_caller_s_object_is_never_modified() {
        let registry = SchemaRegistry::new(None, None).expect("chargement");
        let obj = deployment(&json!({"name": "web", "creationTimestamp": null}));
        let errors = registry.validate(&obj, true).expect("validation");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(obj.pointer("/metadata/creationTimestamp"), Some(&Value::Null));
    }

    #[test]
    fn validators_are_compiled_once() {
        let registry = SchemaRegistry::new(None, None).expect("chargement");
        let obj = deployment(&json!({"name": "web"}));
        registry.validate(&obj, true).expect("premier appel");
        registry.validate(&obj, true).expect("second appel");
        assert_eq!(registry.compiled_count(), 1);
    }
}
