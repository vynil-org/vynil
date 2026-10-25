use crate::RhaiRes;
use rhai::{Dynamic, Engine, Map, serde::to_dynamic};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, PoisonError};

// Re-export for backward compatibility (agent tests import from common::k8smock)
pub use vynil_core::oci_mock::oci_mock_rhai_register;

// ── K8sInstance mock (ServiceInstance, SystemInstance, TenantInstance) ────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct K8sInstanceMockObj {
    pub obj: Dynamic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct K8sInstanceMock {
    pub obj: Dynamic,
    pub mocks: Arc<Mutex<Vec<Dynamic>>>,
}

impl K8sInstanceMock {
    fn get_sub(&self, key: &str) -> RhaiRes<Dynamic> {
        let map = self
            .obj
            .as_map_ref()
            .map_err(|e| -> Box<rhai::EvalAltResult> { format!("mock object is not a map: {e}").into() })?;
        Ok(map.get(key).cloned().unwrap_or(Dynamic::UNIT))
    }

    fn set_status_field(&mut self, key: &str, val: Dynamic) {
        let mut top: Map = match self.obj.as_map_ref() {
            Ok(map) => map.clone(),
            Err(_) => return,
        };
        let status = top
            .entry("status".into())
            .or_insert_with(|| Dynamic::from_map(Map::new()));
        let status_map: Option<Map> = status.as_map_ref().ok().map(|m| m.clone());
        if let Some(mut status_map) = status_map {
            status_map.insert(key.into(), val);
            *status = Dynamic::from_map(status_map);
        }
        self.obj = Dynamic::from_map(top);
        self.persist();
    }

    fn persist(&self) {
        let Ok(map) = self.obj.as_map_ref() else {
            return;
        };
        let Some(kind) = map.get("kind").and_then(|k| k.clone().into_string().ok()) else {
            return;
        };
        let meta: Option<Map> = map
            .get("metadata")
            .and_then(|m| m.as_map_ref().ok().map(|g| g.clone()));
        let Some(meta) = meta else {
            return;
        };
        let Some(name) = meta.get("name").and_then(|n| n.clone().into_string().ok()) else {
            return;
        };
        let Some(ns) = meta.get("namespace").and_then(|n| n.clone().into_string().ok()) else {
            return;
        };
        let mut mocks = self.mocks.lock().unwrap_or_else(PoisonError::into_inner);
        for entry in mocks.iter_mut() {
            let entry_map: Option<Map> = entry.as_map_ref().ok().map(|g| g.clone());
            let Some(entry_map) = entry_map else {
                continue;
            };
            let entry_kind = entry_map.get("kind").and_then(|k| k.clone().into_string().ok());
            let entry_meta: Option<Map> = entry_map
                .get("metadata")
                .and_then(|m| m.as_map_ref().ok().map(|g| g.clone()));
            let entry_name = entry_meta
                .as_ref()
                .and_then(|m| m.get("name"))
                .and_then(|n| n.clone().into_string().ok());
            let entry_ns = entry_meta
                .as_ref()
                .and_then(|m| m.get("namespace"))
                .and_then(|n| n.clone().into_string().ok());
            if entry_kind.as_deref() == Some(kind.as_str())
                && entry_name.as_deref() == Some(name.as_str())
                && entry_ns.as_deref() == Some(ns.as_str())
            {
                *entry = self.obj.clone();
                return;
            }
        }
    }

    // ── Getters ─────────────────────────────────────────────────────────

    /// Returns the `metadata` section of the mocked instance.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_metadata(&mut self) -> RhaiRes<Dynamic> {
        self.get_sub("metadata")
    }

    /// Returns the `spec` section of the mocked instance.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_spec(&mut self) -> RhaiRes<Dynamic> {
        self.get_sub("spec")
    }

    /// Returns the `status` section of the mocked instance.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_status(&mut self) -> RhaiRes<Dynamic> {
        self.get_sub("status")
    }

    pub const fn get_options_digest(&mut self) -> String {
        String::new()
    }

    /// Returns the mocked tfstate stored in `status.tfstate`, or an empty string.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_tfstate(&mut self) -> RhaiRes<String> {
        let status = self.get_sub("status")?;
        if let Ok(m) = status.as_map_ref()
            && let Some(v) = m.get("tfstate")
            && let Ok(s) = v.clone().into_string()
        {
            return Ok(s);
        }
        Ok(String::new())
    }

    /// Returns the mocked rhaistate stored in `status.rhaistate`, or an empty string.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_rhaistate(&mut self) -> RhaiRes<String> {
        let status = self.get_sub("status")?;
        if let Ok(m) = status.as_map_ref()
            && let Some(v) = m.get("rhaistate")
            && let Ok(s) = v.clone().into_string()
        {
            return Ok(s);
        }
        Ok(String::new())
    }

    // ── Common status setters ───────────────────────────────────────────

    /// Stores `tag` in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_ready(&mut self, tag: String) -> RhaiRes<Self> {
        self.set_status_field("tag", Dynamic::from(tag));
        Ok(self.clone())
    }

    /// Records the agent start in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_agent_started(&mut self) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Records a missing `JukeBox` in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_missing_box(&mut self, _jukebox: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Records a missing package in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_missing_package(&mut self, _cat: String, _pkg: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Records a missing requirement in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_missing_requirement(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Records a missing init version in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_missing_init_version(&mut self, _version: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Stores `tfstate` in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_tfstate(&mut self, tfstate: String) -> RhaiRes<Self> {
        self.set_status_field("tfstate", Dynamic::from(tfstate));
        Ok(self.clone())
    }

    /// Records a tofu failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_tofu_failed(&mut self, _tfstate: String, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Stores `rhaistate` in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_rhaistate(&mut self, rhaistate: String) -> RhaiRes<Self> {
        self.set_status_field("rhaistate", Dynamic::from(rhaistate));
        Ok(self.clone())
    }

    /// Records a rhai failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_rhai_failed(&mut self, _rhaistate: String, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    // ── Children status setters ─────────────────────────────────────────

    /// Stores the applied CRDs in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_crds(&mut self, list: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("crds", list);
        Ok(self.clone())
    }

    /// Records a CRD failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_crd_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Stores the applied befores in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_befores(&mut self, list: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("befores", list);
        Ok(self.clone())
    }

    /// Records a before failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_before_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Stores the applied vitals in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_vitals(&mut self, list: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("vitals", list);
        Ok(self.clone())
    }

    /// Records a vital failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_vital_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Stores the applied scalables in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_scalables(&mut self, list: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("scalables", list);
        Ok(self.clone())
    }

    /// Records a scalable failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_scalable_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Stores the applied others in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_others(&mut self, list: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("others", list);
        Ok(self.clone())
    }

    /// Records an other failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_other_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Stores the applied posts in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_posts(&mut self, list: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("posts", list);
        Ok(self.clone())
    }

    /// Records a post failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_post_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Stores the applied systems in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_systems(&mut self, list: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("systems", list);
        Ok(self.clone())
    }

    /// Records a system failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_system_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Records an init failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_init_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    /// Records a schedule backup failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_schedule_backup_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }

    // ── Services ────────────────────────────────────────────────────────

    /// Stores the published services in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_services(&mut self, services: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("services", services);
        Ok(self.clone())
    }

    pub fn get_services_string(&mut self) -> String {
        if let Ok(status) = self.get_sub("status")
            && let Ok(status_map) = status.as_map_ref()
            && let Some(services) = status_map.get("services")
            && let Ok(arr) = services.clone().into_array()
        {
            let mut keys: Vec<String> = arr
                .iter()
                .filter_map(|s| {
                    let m = s.as_map_ref().ok()?;
                    let k = m.get("key")?;
                    k.clone().into_string().ok()
                })
                .collect();
            keys.sort();
            return keys.join(",");
        }
        String::new()
    }

    // ── Tenant-specific ─────────────────────────────────────────────────

    /// Returns the tenant label of the mocked namespace, falling back to the
    /// instance namespace when no label is set.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_tenant_name(&mut self) -> RhaiRes<String> {
        let Some(own) = self.own_namespace() else {
            return Ok(String::new());
        };
        Ok(tenant_of(&self.namespace_mocks(), &own).unwrap_or(own))
    }

    /// Returns the namespaces of the mocked tenant: every mocked `Namespace` sharing the
    /// instance namespace's tenant label, or only the instance namespace when it has none.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_tenant_namespaces(&mut self) -> RhaiRes<Dynamic> {
        let Some(own) = self.own_namespace() else {
            return Ok(Dynamic::from_array(vec![Dynamic::from(String::new())]));
        };
        let entries = self.namespace_mocks();
        let Some(tenant) = tenant_of(&entries, &own) else {
            return Ok(Dynamic::from_array(vec![Dynamic::from(own)]));
        };
        let mut names: Vec<String> = Vec::new();
        for (name, label) in entries {
            if label.as_deref() == Some(tenant.as_str()) && !names.contains(&name) {
                names.push(name);
            }
        }
        Ok(Dynamic::from_array(
            names.into_iter().map(Dynamic::from).collect(),
        ))
    }

    /// The instance's own `metadata.namespace`, when readable as a string.
    fn own_namespace(&self) -> Option<String> {
        let meta = self.get_sub("metadata").ok()?;
        let map = meta.as_map_ref().ok()?;
        map.get("namespace")?.clone().into_string().ok()
    }

    /// `(metadata.name, tenant label display text)` of every mocked `Namespace`, in base order.
    /// The label key is read from `TENANT_LABEL` on each call.
    fn namespace_mocks(&self) -> Vec<(String, Option<String>)> {
        let label_key =
            std::env::var("TENANT_LABEL").unwrap_or_else(|_| "vynil.solidite.fr/tenant".to_string());
        lock_clone(&self.mocks)
            .iter()
            .filter_map(|m| {
                let map: Map = m.as_map_ref().ok()?.clone();
                let is_namespace = map
                    .get("kind")
                    .and_then(|k| k.clone().into_string().ok())
                    .is_some_and(|k| k == "Namespace");
                if !is_namespace {
                    return None;
                }
                let meta: Map = map.get("metadata")?.as_map_ref().ok()?.clone();
                let name = meta.get("name")?.clone().into_string().ok()?;
                let tenant = meta
                    .get("labels")
                    .and_then(|l| l.as_map_ref().ok().map(|g| g.clone()))
                    .and_then(|labels| labels.get(label_key.as_str()).map(ToString::to_string));
                Some((name, tenant))
            })
            .collect()
    }

    /// Returns the service names of the mocked tenant (always empty in the mock).
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn get_tenant_services_names(&mut self) -> RhaiRes<Dynamic> {
        Ok(Dynamic::from_array(vec![]))
    }
}

/// Tenant label of the first mocked `Namespace` named `own` that carries one.
fn tenant_of(entries: &[(String, Option<String>)], own: &str) -> Option<String> {
    entries
        .iter()
        .find_map(|(name, tenant)| if name == own { tenant.clone() } else { None })
}

fn lock_clone(mocks: &Arc<Mutex<Vec<Dynamic>>>) -> Vec<Dynamic> {
    let guard = mocks.lock().unwrap_or_else(PoisonError::into_inner);
    guard.clone()
}

fn matches_kind_and_namespace(item: &Dynamic, kind: &str, namespace: &str) -> bool {
    let map: Option<Map> = item.as_map_ref().ok().map(|g| g.clone());
    let Some(map) = map else {
        return false;
    };
    let kind_match = map
        .get("kind")
        .and_then(|k| k.clone().into_string().ok())
        .is_some_and(|k| k == kind);
    if !kind_match {
        return false;
    }
    let meta: Option<Map> = map
        .get("metadata")
        .and_then(|v| v.as_map_ref().ok().map(|g| g.clone()));
    let Some(meta) = meta else {
        return false;
    };
    meta.get("namespace")
        .and_then(|n| n.clone().into_string().ok())
        .is_some_and(|n| n == namespace)
}

fn find_instance_mock(
    mocks: &Arc<Mutex<Vec<Dynamic>>>,
    kind: &str,
    namespace: &str,
    name: &str,
) -> RhaiRes<K8sInstanceMock> {
    for m in lock_clone(mocks) {
        if !matches_kind_and_namespace(&m, kind, namespace) {
            continue;
        }
        let map: Option<Map> = m.as_map_ref().ok().map(|g| g.clone());
        let Some(map) = map else {
            continue;
        };
        let name_match = map
            .get("metadata")
            .and_then(|v| v.as_map_ref().ok().map(|g| g.clone()))
            .and_then(|meta| meta.get("name").and_then(|n| n.clone().into_string().ok()))
            .is_some_and(|n| n == name);
        if name_match {
            return Ok(K8sInstanceMock {
                obj: m,
                mocks: Arc::clone(mocks),
            });
        }
    }
    Err(format!("Failed to find {kind} {name} in namespace {namespace} in the Mock database").into())
}

fn list_instance_mocks(mocks: &Arc<Mutex<Vec<Dynamic>>>, kind: &str, namespace: &str) -> RhaiRes<Dynamic> {
    let items: Vec<K8sInstanceMockObj> = lock_clone(mocks)
        .iter()
        .filter(|m| matches_kind_and_namespace(m, kind, namespace))
        .map(|m| K8sInstanceMockObj { obj: m.clone() })
        .collect();
    to_dynamic(serde_json::json!({"items": items}))
}

// ── JukeBox mock ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct K8sJukeBoxMock {
    pub obj: Dynamic,
}

impl K8sJukeBoxMock {
    fn get_sub(&self, key: &str) -> RhaiRes<Dynamic> {
        let map = self
            .obj
            .as_map_ref()
            .map_err(|e| -> Box<rhai::EvalAltResult> { format!("mock object is not a map: {e}").into() })?;
        Ok(map.get(key).cloned().unwrap_or(Dynamic::UNIT))
    }

    fn set_status_field(&mut self, key: &str, val: Dynamic) {
        let mut top: Map = match self.obj.as_map_ref() {
            Ok(map) => map.clone(),
            Err(_) => return,
        };
        let status = top
            .entry("status".into())
            .or_insert_with(|| Dynamic::from_map(Map::new()));
        let status_map: Option<Map> = status.as_map_ref().ok().map(|m| m.clone());
        if let Some(mut status_map) = status_map {
            status_map.insert(key.into(), val);
            *status = Dynamic::from_map(status_map);
        }
        self.obj = Dynamic::from_map(top);
    }

    /// Returns the `metadata` section of the mocked `JukeBox`.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_metadata(&mut self) -> RhaiRes<Dynamic> {
        self.get_sub("metadata")
    }

    /// Returns the `spec` section of the mocked `JukeBox`.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_spec(&mut self) -> RhaiRes<Dynamic> {
        self.get_sub("spec")
    }

    /// Returns the `status` section of the mocked `JukeBox`.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the mocked object is not a map.
    pub fn get_status(&mut self) -> RhaiRes<Dynamic> {
        self.get_sub("status")
    }

    /// Stores the scanned packages in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_updated(&mut self, packages: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("packages", packages);
        Ok(self.clone())
    }

    /// Stores the merged packages in the mocked `status`.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_packages_merge(&mut self, _filter: String, packages: Dynamic) -> RhaiRes<Self> {
        self.set_status_field("packages", packages);
        Ok(self.clone())
    }

    /// Records a scan failure in the mock.
    ///
    /// # Errors
    ///
    /// Never fails; the mock always succeeds.
    pub fn set_status_failed(&mut self, _reason: String) -> RhaiRes<Self> {
        Ok(self.clone())
    }
}

fn find_jukebox_mock(mocks: &Arc<Mutex<Vec<Dynamic>>>, name: &str) -> RhaiRes<K8sJukeBoxMock> {
    for m in lock_clone(mocks) {
        let map: Option<Map> = m.as_map_ref().ok().map(|g| g.clone());
        let Some(map) = map else {
            continue;
        };
        let kind_is_jukebox = map
            .get("kind")
            .and_then(|k| k.clone().into_string().ok())
            .is_some_and(|k| k == "JukeBox");
        if !kind_is_jukebox {
            continue;
        }
        let name_match = map
            .get("metadata")
            .and_then(|v| v.as_map_ref().ok().map(|g| g.clone()))
            .and_then(|meta| meta.get("name").and_then(|n| n.clone().into_string().ok()))
            .is_some_and(|n| n == name);
        if name_match {
            return Ok(K8sJukeBoxMock { obj: m });
        }
    }
    Err(format!("Failed to find JukeBox {name} in the Mock database").into())
}

fn list_jukebox_mocks(mocks: &Arc<Mutex<Vec<Dynamic>>>) -> RhaiRes<Dynamic> {
    let items: Vec<K8sJukeBoxMock> = lock_clone(mocks)
        .iter()
        .filter(|m| {
            m.as_map_ref().is_ok_and(|map| {
                map.get("kind")
                    .and_then(|k| k.clone().into_string().ok())
                    .is_some_and(|k| k == "JukeBox")
            })
        })
        .map(|m| K8sJukeBoxMock { obj: m.clone() })
        .collect();
    to_dynamic(serde_json::json!({"items": items}))
}

fn register_instance_common(engine: &mut Engine) {
    engine
        .register_fn("options_digest", K8sInstanceMock::get_options_digest)
        .register_fn("get_tfstate", K8sInstanceMock::get_tfstate)
        .register_fn("get_rhaistate", K8sInstanceMock::get_rhaistate)
        .register_fn("set_agent_started", K8sInstanceMock::set_agent_started)
        .register_fn("set_missing_box", K8sInstanceMock::set_missing_box)
        .register_fn("set_missing_package", K8sInstanceMock::set_missing_package)
        .register_fn(
            "set_missing_requirement",
            K8sInstanceMock::set_missing_requirement,
        )
        .register_fn(
            "set_missing_init_version",
            K8sInstanceMock::set_missing_init_version,
        )
        .register_fn("set_status_ready", K8sInstanceMock::set_status_ready)
        .register_fn("set_tfstate", K8sInstanceMock::set_tfstate)
        .register_fn("set_status_tofu_failed", K8sInstanceMock::set_status_tofu_failed)
        .register_fn("set_rhaistate", K8sInstanceMock::set_rhaistate)
        .register_fn("set_status_rhai_failed", K8sInstanceMock::set_status_rhai_failed)
        .register_get("metadata", K8sInstanceMock::get_metadata)
        .register_get("spec", K8sInstanceMock::get_spec)
        .register_get("status", K8sInstanceMock::get_status);
}

fn register_instance_children(engine: &mut Engine) {
    engine
        .register_fn("set_services", K8sInstanceMock::set_services)
        .register_fn("get_services", K8sInstanceMock::get_services_string)
        .register_fn("set_status_befores", K8sInstanceMock::set_status_befores)
        .register_fn(
            "set_status_before_failed",
            K8sInstanceMock::set_status_before_failed,
        )
        .register_fn("set_status_vitals", K8sInstanceMock::set_status_vitals)
        .register_fn(
            "set_status_vital_failed",
            K8sInstanceMock::set_status_vital_failed,
        )
        .register_fn("set_status_scalables", K8sInstanceMock::set_status_scalables)
        .register_fn(
            "set_status_scalable_failed",
            K8sInstanceMock::set_status_scalable_failed,
        )
        .register_fn("set_status_others", K8sInstanceMock::set_status_others)
        .register_fn(
            "set_status_other_failed",
            K8sInstanceMock::set_status_other_failed,
        )
        .register_fn("set_status_posts", K8sInstanceMock::set_status_posts)
        .register_fn("set_status_post_failed", K8sInstanceMock::set_status_post_failed)
        .register_fn("set_status_init_failed", K8sInstanceMock::set_status_init_failed)
        .register_fn(
            "set_status_schedule_backup_failed",
            K8sInstanceMock::set_status_schedule_backup_failed,
        );
}

pub fn k8smock_rhai_register(engine: &mut Engine, mocks: Vec<Dynamic>, created: Arc<Mutex<Vec<Dynamic>>>) {
    let arced_mocks = Arc::new(Mutex::new(mocks));

    // Register generic K8s mocks from core
    vynil_core::k8s_mock::k8s_mock_rhai_register(engine, arced_mocks.clone(), created);

    // ── Instance mocks ──────────────────────────────────────────────────

    // ServiceInstance
    let inst_mocks = arced_mocks.clone();
    let get_svc = move |ns: String, name: String| -> RhaiRes<K8sInstanceMock> {
        find_instance_mock(&inst_mocks, "ServiceInstance", &ns, &name)
    };
    let inst_mocks = arced_mocks.clone();
    let list_svc =
        move |ns: String| -> RhaiRes<Dynamic> { list_instance_mocks(&inst_mocks, "ServiceInstance", &ns) };
    engine
        .register_type_with_name::<K8sInstanceMock>("ServiceInstance")
        .register_fn("get_service_instance", get_svc)
        .register_fn("list_service_instance", list_svc)
        .register_fn("list_services_names", || -> RhaiRes<Vec<String>> { Ok(vec![]) })
        .register_fn("set_status_crds", K8sInstanceMock::set_status_crds)
        .register_fn("set_status_crd_failed", K8sInstanceMock::set_status_crd_failed);
    register_instance_common(engine);
    register_instance_children(engine);

    // SystemInstance
    let inst_mocks = arced_mocks.clone();
    let get_sys = move |ns: String, name: String| -> RhaiRes<K8sInstanceMock> {
        find_instance_mock(&inst_mocks, "SystemInstance", &ns, &name)
    };
    let inst_mocks = arced_mocks.clone();
    let list_sys =
        move |ns: String| -> RhaiRes<Dynamic> { list_instance_mocks(&inst_mocks, "SystemInstance", &ns) };
    engine
        .register_type_with_name::<K8sInstanceMock>("SystemInstance")
        .register_fn("get_system_instance", get_sys)
        .register_fn("list_system_instance", list_sys)
        .register_fn("set_status_crds", K8sInstanceMock::set_status_crds)
        .register_fn("set_status_crd_failed", K8sInstanceMock::set_status_crd_failed)
        .register_fn("set_status_systems", K8sInstanceMock::set_status_systems)
        .register_fn(
            "set_status_system_failed",
            K8sInstanceMock::set_status_system_failed,
        );
    register_instance_common(engine);

    // TenantInstance
    let inst_mocks = arced_mocks.clone();
    let get_tnt = move |ns: String, name: String| -> RhaiRes<K8sInstanceMock> {
        find_instance_mock(&inst_mocks, "TenantInstance", &ns, &name)
    };
    let inst_mocks = arced_mocks.clone();
    let list_tnt =
        move |ns: String| -> RhaiRes<Dynamic> { list_instance_mocks(&inst_mocks, "TenantInstance", &ns) };
    engine
        .register_type_with_name::<K8sInstanceMock>("TenantInstance")
        .register_fn("get_tenant_instance", get_tnt)
        .register_fn("list_tenant_instance", list_tnt)
        .register_fn("get_tenant_name", K8sInstanceMock::get_tenant_name)
        .register_fn("get_tenant_namespaces", K8sInstanceMock::get_tenant_namespaces)
        .register_fn(
            "get_tenant_services_names",
            K8sInstanceMock::get_tenant_services_names,
        );
    register_instance_common(engine);
    register_instance_children(engine);

    // JukeBox (cluster-scoped)
    let jb_mocks = arced_mocks.clone();
    let get_jb = move |name: String| -> RhaiRes<K8sJukeBoxMock> { find_jukebox_mock(&jb_mocks, &name) };
    let jb_mocks = arced_mocks;
    let list_jb = move || -> RhaiRes<Dynamic> { list_jukebox_mocks(&jb_mocks) };
    engine
        .register_type_with_name::<K8sJukeBoxMock>("JukeBox")
        .register_fn("get_jukebox", get_jb)
        .register_fn("list_jukebox", list_jb)
        .register_fn("set_status_updated", K8sJukeBoxMock::set_status_updated)
        .register_fn(
            "set_status_packages_merge",
            K8sJukeBoxMock::set_status_packages_merge,
        )
        .register_fn("set_status_failed", K8sJukeBoxMock::set_status_failed)
        .register_get("metadata", K8sJukeBoxMock::get_metadata)
        .register_get("spec", K8sJukeBoxMock::get_spec)
        .register_get("status", K8sJukeBoxMock::get_status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Mock `Namespace` `name`, labellisé `vynil.solidite.fr/tenant: tenant` quand `tenant` est donné.
    fn namespace(name: &str, tenant: Option<&str>) -> Dynamic {
        let labels = tenant.map_or_else(|| json!({}), |t| json!({ "vynil.solidite.fr/tenant": t }));
        to_dynamic(json!({ "apiVersion": "v1", "kind": "Namespace", "metadata": { "name": name, "labels": labels } }))
            .expect("Namespace convertible")
    }

    /// `TenantInstance` mockée dans `namespace` (sans `metadata.namespace` si `None`), sur la base `db`.
    fn tenant_instance(namespace: Option<&str>, db: Vec<Dynamic>) -> K8sInstanceMock {
        let metadata = namespace.map_or_else(
            || json!({ "name": "t" }),
            |ns| json!({ "name": "t", "namespace": ns }),
        );
        K8sInstanceMock {
            obj: to_dynamic(json!({ "kind": "TenantInstance", "metadata": metadata }))
                .expect("instance convertible"),
            mocks: Arc::new(Mutex::new(db)),
        }
    }

    fn names(list: Dynamic) -> Vec<String> {
        list.into_array()
            .expect("tableau")
            .into_iter()
            .map(|d| d.to_string())
            .collect()
    }

    #[test]
    fn tenant_namespaces_list_every_namespace_sharing_the_label() {
        let mut inst = tenant_instance(Some("team-a"), vec![
            namespace("other", Some("globex")),
            namespace("team-a", Some("acme")),
            namespace("team-b", Some("acme")),
            namespace("team-b", Some("acme")),
            namespace("plain", None),
        ]);
        assert_eq!(inst.get_tenant_name().expect("nom"), "acme");
        assert_eq!(names(inst.get_tenant_namespaces().expect("namespaces")), [
            "team-a", "team-b"
        ]);
    }

    #[test]
    fn tenant_namespaces_fall_back_to_the_own_namespace() {
        let mut unlabelled = tenant_instance(Some("team-a"), vec![namespace("team-a", None)]);
        assert_eq!(unlabelled.get_tenant_name().expect("nom"), "team-a");
        assert_eq!(names(unlabelled.get_tenant_namespaces().expect("namespaces")), [
            "team-a"
        ]);

        let mut unmocked = tenant_instance(Some("team-a"), vec![namespace("team-b", Some("acme"))]);
        assert_eq!(names(unmocked.get_tenant_namespaces().expect("namespaces")), [
            "team-a"
        ]);

        let mut no_namespace = tenant_instance(None, vec![namespace("team-a", Some("acme"))]);
        assert_eq!(no_namespace.get_tenant_name().expect("nom"), "");
        assert_eq!(
            names(no_namespace.get_tenant_namespaces().expect("namespaces")),
            [""]
        );
    }
}
