use crate::{Error, Result, RhaiRes, jukebox::JukeBoxDef, rhai_err, vynilpackage::VynilPackage};
use rhai::{Dynamic, Engine};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FileScanSpec {
    pub source: Option<JukeBoxDef>,
    pub pull_secret: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CacheIndex {
    pub packages: Vec<CacheIndexEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheIndexEntry {
    pub category: String,
    pub name: String,
    pub file: String,
}

#[derive(Clone, Debug)]
pub struct FileJukeBox {
    spec: FileScanSpec,
    cache_dir: PathBuf,
}

impl FileJukeBox {
    #[must_use]
    pub const fn new(spec: FileScanSpec, cache_dir: PathBuf) -> Self {
        Self { spec, cache_dir }
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the section cannot be converted into a rhai `Dynamic`.
    pub fn get_spec(&mut self) -> RhaiRes<Dynamic> {
        let v = serde_json::json!({
            "source": self.spec.source,
            "maturity": "alpha",
            "pull_secret": self.spec.pull_secret,
            "schedule": "",
        });
        serde_json::from_value(v).map_err(|e| rhai_err(Error::SerializationError(e)))
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the section cannot be converted into a rhai `Dynamic`.
    pub fn get_status(&mut self) -> RhaiRes<Dynamic> {
        let packages = self.read_all_from_cache().map_err(rhai_err)?;
        let v = serde_json::json!({ "conditions": [], "packages": packages });
        serde_json::from_value(v).map_err(|e| rhai_err(Error::SerializationError(e)))
    }

    fn read_all_from_cache(&self) -> Result<Vec<VynilPackage>> {
        let index_path = self.cache_dir.join("index.yaml");
        if !index_path.exists() {
            return Ok(vec![]);
        }
        let content = std::fs::read_to_string(&index_path).map_err(Error::Stdio)?;
        let index: CacheIndex =
            serde_yaml::from_str(&content).map_err(|e| Error::YamlError(e.to_string()))?;
        let mut all = vec![];
        for entry in &index.packages {
            let path = self.cache_dir.join(&entry.file);
            if path.exists() {
                let pkg_content = std::fs::read_to_string(&path).map_err(Error::Stdio)?;
                let pkgs: Vec<VynilPackage> =
                    serde_yaml::from_str(&pkg_content).map_err(|e| Error::YamlError(e.to_string()))?;
                all.extend(pkgs);
            }
        }
        Ok(all)
    }

    fn write_to_cache(&self, packages: &[VynilPackage], filter: Option<&str>) -> Result<()> {
        let mut grouped: BTreeMap<(String, String), Vec<VynilPackage>> = BTreeMap::new();
        for pkg in packages {
            grouped
                .entry((pkg.metadata.category.clone(), pkg.metadata.name.clone()))
                .or_default()
                .push(pkg.clone());
        }

        let index_path = self.cache_dir.join("index.yaml");
        let mut index: CacheIndex = if index_path.exists() {
            let content = std::fs::read_to_string(&index_path).map_err(Error::Stdio)?;
            serde_yaml::from_str(&content).map_err(|e| Error::YamlError(e.to_string()))?
        } else {
            CacheIndex::default()
        };

        for ((category, name), pkgs) in &grouped {
            let filename = format!("{category}_{name}.yaml");
            let pkg_path = self.cache_dir.join(&filename);
            let yaml = serde_yaml::to_string(pkgs).map_err(|e| Error::YamlError(e.to_string()))?;
            std::fs::write(&pkg_path, yaml).map_err(Error::Stdio)?;

            if !index
                .packages
                .iter()
                .any(|e| e.category == *category && e.name == *name)
            {
                index.packages.push(CacheIndexEntry {
                    category: category.clone(),
                    name: name.clone(),
                    file: filename,
                });
            }
        }

        let index_yaml = serde_yaml::to_string(&index).map_err(|e| Error::YamlError(e.to_string()))?;
        std::fs::write(&index_path, index_yaml).map_err(Error::Stdio)?;

        let _ = filter;
        Ok(())
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the rhai argument cannot be converted or the underlying
    /// operation fails.
    pub fn rhai_set_status_updated(&mut self, list: Dynamic) -> RhaiRes<Self> {
        let v = serde_json::to_string(&list).map_err(|e| rhai_err(Error::SerializationError(e)))?;
        let packages: Vec<VynilPackage> =
            serde_json::from_str(&v).map_err(|e| rhai_err(Error::SerializationError(e)))?;
        self.write_to_cache(&packages, None).map_err(rhai_err)?;
        drop(list);
        Ok(self.clone())
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the rhai argument cannot be converted or the underlying
    /// operation fails.
    pub fn rhai_set_status_packages_merge(&mut self, filter: String, list: Dynamic) -> RhaiRes<Self> {
        let v = serde_json::to_string(&list).map_err(|e| rhai_err(Error::SerializationError(e)))?;
        let packages: Vec<VynilPackage> =
            serde_json::from_str(&v).map_err(|e| rhai_err(Error::SerializationError(e)))?;
        self.write_to_cache(&packages, Some(&filter)).map_err(rhai_err)?;
        drop(list);
        drop(filter);
        Ok(self.clone())
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the underlying operation fails.
    pub fn rhai_set_status_failed(&mut self, reason: String) -> RhaiRes<Self> {
        let msg = format!("SCAN-FILE-001: Scan failed: {reason}");
        drop(reason);
        Err(rhai_err(Error::Other(msg)))
    }
}

pub fn file_jukebox_rhai_register(engine: &mut Engine) {
    engine
        .register_type_with_name::<FileJukeBox>("FileJukeBox")
        .register_fn("set_status_updated", FileJukeBox::rhai_set_status_updated)
        .register_fn("set_status_failed", FileJukeBox::rhai_set_status_failed)
        .register_fn(
            "set_status_packages_merge",
            FileJukeBox::rhai_set_status_packages_merge,
        )
        .register_get("spec", FileJukeBox::get_spec)
        .register_get("status", FileJukeBox::get_status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vynilpackage::{VynilPackageMeta, VynilPackageType};
    use tempfile::TempDir;

    fn make_pkg(category: &str, name: &str) -> VynilPackage {
        VynilPackage {
            registry: "docker.io".to_string(),
            image: format!("{category}/{name}"),
            tag: "1.0.0".to_string(),
            metadata: VynilPackageMeta {
                name: name.to_string(),
                category: category.to_string(),
                description: "test".to_string(),
                app_version: None,
                usage: VynilPackageType::Service,
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: None,
            value_script: None,
        }
    }

    fn make_box(dir: &TempDir) -> FileJukeBox {
        FileJukeBox::new(FileScanSpec::default(), dir.path().to_path_buf())
    }

    #[test]
    fn write_to_cache_creates_files() {
        let dir = TempDir::new().unwrap();
        let fb = make_box(&dir);
        let pkgs = vec![make_pkg("database", "postgresql")];
        fb.write_to_cache(&pkgs, None).unwrap();

        assert!(dir.path().join("database_postgresql.yaml").exists());
        assert!(dir.path().join("index.yaml").exists());

        let index_content = std::fs::read_to_string(dir.path().join("index.yaml")).unwrap();
        let index: CacheIndex = serde_yaml::from_str(&index_content).unwrap();
        assert_eq!(index.packages.len(), 1);
        assert_eq!(index.packages[0].category, "database");
        assert_eq!(index.packages[0].name, "postgresql");
    }

    #[test]
    fn write_to_cache_no_duplicate_in_index() {
        let dir = TempDir::new().unwrap();
        let fb = make_box(&dir);
        let pkgs = vec![make_pkg("database", "postgresql")];
        fb.write_to_cache(&pkgs, None).unwrap();
        fb.write_to_cache(&pkgs, None).unwrap();

        let index_content = std::fs::read_to_string(dir.path().join("index.yaml")).unwrap();
        let index: CacheIndex = serde_yaml::from_str(&index_content).unwrap();
        assert_eq!(index.packages.len(), 1);
    }

    #[test]
    fn read_all_from_cache_returns_packages() {
        let dir = TempDir::new().unwrap();
        let fb = make_box(&dir);
        let pkgs = vec![
            make_pkg("database", "postgresql"),
            make_pkg("monitoring", "prometheus"),
        ];
        fb.write_to_cache(&pkgs, None).unwrap();

        let result = fb.read_all_from_cache().unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn read_all_from_cache_no_index_returns_empty() {
        let dir = TempDir::new().unwrap();
        let fb = make_box(&dir);
        let result = fb.read_all_from_cache().unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn get_status_returns_correct_packages() {
        let dir = TempDir::new().unwrap();
        let mut fb = make_box(&dir);
        let pkgs = vec![make_pkg("database", "postgresql")];
        fb.write_to_cache(&pkgs, None).unwrap();

        let status = fb.get_status().unwrap();
        let status_map = status.cast::<crate::rhaihandler::Map>();
        let packages = status_map["packages"].clone().cast::<rhai::Array>();
        assert_eq!(packages.len(), 1);
        let pkg_map = packages[0].clone().cast::<crate::rhaihandler::Map>();
        let meta = pkg_map["metadata"].clone().cast::<crate::rhaihandler::Map>();
        assert_eq!(meta["category"].clone().cast::<String>(), "database");
        assert_eq!(meta["name"].clone().cast::<String>(), "postgresql");
    }
}
