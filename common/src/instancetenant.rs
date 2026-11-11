use crate::{Error, Published, Result, RhaiRes, context::get_client_async, rhai_err};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Namespace;
use kube::{
    CustomResource, Resource, ResourceExt,
    api::{Api, ListParams},
};
use rhai::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `InitFrom` contains the informations for the backup to use to initialize the installation
#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InitFrom {
    /// Name of the secret containing: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `BASE_REPO_URL` and `RESTIC_PASSWORD`. Default to "backup-settings"
    pub secret_name: Option<String>,
    /// Path within the bucket containing the backup to use for recovery. Default to "<namespace-name>/<app-slug>"
    pub sub_path: Option<String>,
    /// Snapshot id for restoration
    pub snapshot: String,
    /// Version du package à utiliser pour la restauration
    pub version: Option<String>,
}

/// Describe a source of vynil packages jukebox
#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    kind = "TenantInstance",
    status = "TenantInstanceStatus",
    shortname = "vti",
    group = "vynil.solidite.fr",
    version = "v1",
    namespaced
)]
#[kube(
    doc = "Custom resource representing an Vynil tenant package installation",
    printcolumn = r#"{"name":"Juke",        "type":"string", "description":"JukeBox",          "jsonPath":".spec.jukebox"}"#,
    printcolumn = r#"{"name":"cat",         "type":"string", "description":"Category",          "jsonPath":".spec.category"}"#,
    printcolumn = r#"{"name":"pkg",         "type":"string", "description":"Package",           "jsonPath":".spec.package"}"#,
    printcolumn = r#"{"name":"tag",         "type":"string", "description":"Version",           "jsonPath":".status.tag"}"#,
    printcolumn = r#"{"name":"last_updated","type":"date",   "description":"Last update date",  "format":"date-time", "jsonPath":".status.conditions[?(@.type == 'Ready')].lastTransitionTime"}"#,
    printcolumn = r#"{"name":"stage",       "type":"string", "description":"Stage",             "jsonPath":".status.conditions[-1:].type"}"#,
    printcolumn = r#"{"name":"errors",      "type":"string", "description":"Errors",            "jsonPath":".status.conditions[?(@.status == 'False')].message"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct TenantInstanceSpec {
    /// The jukebox source name
    pub jukebox: String,
    /// The category name
    pub category: String,
    /// The package name
    pub package: String,
    /// Init from a previous backup
    pub init_from: Option<InitFrom>,
    /// Parameters
    pub options: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema, Default)]
pub enum ConditionsType {
    #[default]
    Ready,
    Installed,
    Backuped,
    Restored,
    AgentStarted,
    TofuInstalled,
    BeforeApplied,
    VitalApplied,
    ScalableApplied,
    InitFrom,
    ScheduleBackup,
    OtherApplied,
    RhaiApplied,
    PostApplied,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema, Default)]
pub enum ConditionsStatus {
    #[default]
    True,
    False,
}

/// `ApplicationCondition` contains details about an application condition, which is usually an error or warning
#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationCondition {
    /// `LastTransitionTime` is the time the condition was last observed
    pub last_transition_time: Option<DateTime<Utc>>,
    /// Message contains human-readable message indicating details about condition
    pub message: String,
    /// Type is an application condition type
    #[serde(rename = "type")]
    pub condition_type: ConditionsType,
    /// Status ("True" or "False") describe if the condition is enbled
    pub status: ConditionsStatus,
    /// Generation for that status
    pub generation: i64,
}

impl_condition_common!();
impl_condition_children!();

/// The status object of `TenantInstance`
#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct TenantInstanceStatus {
    /// `TenantInstance` Conditions
    pub conditions: Vec<ApplicationCondition>,
    /// Current tag
    pub tag: Option<String>,
    /// Options digests
    pub digest: Option<String>,
    /// Current terraform status (gzip+base64)
    pub tfstate: Option<String>,
    /// Current rhai status (gzip+base64) (for custom package information)
    pub rhaistate: Option<String>,
    /// List of before children
    pub befores: Option<Vec<crate::Children>>,
    /// List of vital children
    pub vitals: Option<Vec<crate::Children>>,
    /// List of scalable children
    pub scalables: Option<Vec<crate::Children>>,
    /// List of other children
    pub others: Option<Vec<crate::Children>>,
    /// List of post children
    pub posts: Option<Vec<crate::Children>>,
    /// List of the services
    pub services: Option<Vec<Published>>,
}

impl TenantInstance {
    #[must_use]
    pub fn have_child(&self) -> bool {
        if let Some(status) = self.status.clone() {
            if status.rhaistate.is_some() {
                return true;
            }
            if status.tfstate.is_some() {
                return true;
            }
            if let Some(child) = status.befores.clone()
                && !child.is_empty()
            {
                return true;
            }
            if let Some(child) = status.vitals.clone()
                && !child.is_empty()
            {
                return true;
            }
            if let Some(child) = status.others.clone()
                && !child.is_empty()
            {
                return true;
            }
            if let Some(child) = status.scalables.clone()
                && !child.is_empty()
            {
                return true;
            }
            if let Some(child) = status.posts
                && !child.is_empty()
            {
                return true;
            }
        }
        false
    }

    /// Returns the tenant name, read from the namespace tenant label when present,
    /// or from the instance namespace otherwise.
    ///
    /// # Errors
    ///
    /// Returns `Error::Other` when the instance has no namespace, and `Error::KubeError`
    /// when the namespace metadata cannot be read.
    pub async fn get_tenant_name(&self) -> Result<String> {
        let my_ns = self
            .metadata
            .namespace
            .clone()
            .ok_or_else(|| Error::Other("TenantInstance has no namespace".to_string()))?;
        let ns_api: Api<Namespace> = Api::all(get_client_async().await);
        let my_ns_meta = ns_api.get_metadata(&my_ns).await.map_err(Error::from)?;
        let label_key =
            std::env::var("TENANT_LABEL").unwrap_or_else(|_| "vynil.solidite.fr/tenant".to_string());
        if let Some(labels) = my_ns_meta.metadata.labels
            && let Some(tenant) = labels.get(&label_key)
        {
            return Ok(tenant.clone());
        }
        Ok(my_ns)
    }

    /// Returns every namespace owned by the instance tenant, or only the instance
    /// namespace when no tenant label is set.
    ///
    /// # Errors
    ///
    /// Returns `Error::Other` when the instance has no namespace, and `Error::KubeError`
    /// when the namespaces cannot be listed.
    pub async fn get_tenant_namespaces(&self) -> Result<Vec<String>> {
        let my_ns = self
            .metadata
            .namespace
            .clone()
            .ok_or_else(|| Error::Other("TenantInstance has no namespace".to_string()))?;
        let ns_api: Api<Namespace> = Api::all(get_client_async().await);
        let my_ns_meta = ns_api.get_metadata(&my_ns).await.map_err(Error::from)?;
        let label_key =
            std::env::var("TENANT_LABEL").unwrap_or_else(|_| "vynil.solidite.fr/tenant".to_string());
        let res = vec![my_ns];
        let tenant_name = my_ns_meta
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(&label_key));
        if let Some(tenant_name) = tenant_name {
            let mut lp = ListParams::default();
            lp = lp.labels(format!("{label_key}=={tenant_name}").as_str());
            let my_nss = ns_api.list_metadata(&lp).await.map_err(Error::from)?;
            return Ok(my_nss.items.into_iter().filter_map(|n| n.metadata.name).collect());
        }
        Ok(res)
    }

    /// Returns the sorted published service names of every tenant namespace.
    ///
    /// # Errors
    ///
    /// Returns `Error::KubeError` when the tenant instances cannot be listed.
    pub async fn get_tenant_services_names(&self) -> Result<Vec<String>> {
        let mut res: Vec<String> = Vec::new();
        let cli = get_client_async().await;
        for ns in self.get_tenant_namespaces().await? {
            let api = Api::<Self>::namespaced(cli.clone(), &ns);
            let lp = ListParams::default();
            for tnt in api.list(&lp).await.map_err(Error::from)? {
                let mut svcs: Vec<String> = tnt.get_services().iter().map(|i| i.key.clone()).collect();
                res.append(&mut svcs);
            }
        }
        res.sort();
        Ok(res)
    }

    /// Rhai wrapper over `get_tenant_name`.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when `get_tenant_name` fails.
    pub fn rhai_get_tenant_name(&mut self) -> RhaiRes<String> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move { self.get_tenant_name().await })
        })
        .map_err(rhai_err)
    }

    /// Rhai wrapper over `get_tenant_namespaces`, returning a rhai array.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the namespaces cannot be serialized.
    pub fn rhai_get_tenant_namespaces(&mut self) -> RhaiRes<rhai::Dynamic> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let arr = self.get_tenant_namespaces().await;
                if let Ok(arr) = arr {
                    let v = serde_json::to_string(&arr).map_err(Error::SerializationError)?;
                    serde_json::from_str::<rhai::Dynamic>(&v).map_err(Error::SerializationError)
                } else {
                    Ok(rhai::Dynamic::from(""))
                }
            })
        })
        .map_err(rhai_err)
    }

    /// Rhai wrapper over `get_tenant_services_names`, returning a rhai array.
    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the service names cannot be serialized.
    pub fn rhai_get_tenant_services_names(&mut self) -> RhaiRes<rhai::Dynamic> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                let arr = self.get_tenant_services_names().await;
                if let Ok(arr) = arr {
                    let v = serde_json::to_string(&arr).map_err(Error::SerializationError)?;
                    serde_json::from_str::<rhai::Dynamic>(&v).map_err(Error::SerializationError)
                } else {
                    Ok(rhai::Dynamic::from(""))
                }
            })
        })
        .map_err(rhai_err)
    }
}

impl_instance_common!(TenantInstance, "TenantInstance");
impl_instance_befores!(TenantInstance);

/// Registers `TenantInstance` accessors on a rhai engine.
pub fn tenant_rhai_register(engine: &mut Engine) {
    engine
        .register_type_with_name::<TenantInstance>("TenantInstance")
        .register_fn("get_tenant_instance", TenantInstance::rhai_get)
        .register_fn("get_tenant_name", TenantInstance::rhai_get_tenant_name)
        .register_fn(
            "get_tenant_namespaces",
            TenantInstance::rhai_get_tenant_namespaces,
        )
        .register_fn(
            "get_tenant_services_names",
            TenantInstance::rhai_get_tenant_services_names,
        )
        .register_fn("list_tenant_instance", TenantInstance::rhai_list)
        .register_fn("options_digest", TenantInstance::get_options_digest)
        .register_fn("get_tfstate", TenantInstance::rhai_get_tfstate)
        .register_fn("get_rhaistate", TenantInstance::rhai_get_rhaistate)
        .register_fn("set_agent_started", TenantInstance::rhai_set_agent_started)
        .register_fn("set_missing_box", TenantInstance::rhai_set_missing_box)
        .register_fn("set_missing_package", TenantInstance::rhai_set_missing_package)
        .register_fn(
            "set_missing_requirement",
            TenantInstance::rhai_set_missing_requirement,
        )
        .register_fn(
            "set_missing_init_version",
            TenantInstance::rhai_set_missing_init_version,
        )
        .register_fn("set_status_ready", TenantInstance::rhai_set_status_ready)
        .register_fn("set_status_befores", TenantInstance::rhai_set_status_befores)
        .register_fn(
            "set_status_before_failed",
            TenantInstance::rhai_set_status_before_failed,
        )
        .register_fn("set_status_vitals", TenantInstance::rhai_set_status_vitals)
        .register_fn(
            "set_status_vital_failed",
            TenantInstance::rhai_set_status_vital_failed,
        )
        .register_fn("set_status_scalables", TenantInstance::rhai_set_status_scalables)
        .register_fn(
            "set_status_scalable_failed",
            TenantInstance::rhai_set_status_scalable_failed,
        )
        .register_fn("set_status_others", TenantInstance::rhai_set_status_others)
        .register_fn(
            "set_status_other_failed",
            TenantInstance::rhai_set_status_other_failed,
        )
        .register_fn("set_status_posts", TenantInstance::rhai_set_status_posts)
        .register_fn(
            "set_status_post_failed",
            TenantInstance::rhai_set_status_post_failed,
        )
        .register_fn("set_tfstate", TenantInstance::rhai_set_tfstate)
        .register_fn(
            "set_status_tofu_failed",
            TenantInstance::rhai_set_status_tofu_failed,
        )
        .register_fn("set_rhaistate", TenantInstance::rhai_set_rhaistate)
        .register_fn("set_services", TenantInstance::rhai_set_services)
        .register_fn("get_services", TenantInstance::rhai_get_services)
        .register_fn(
            "set_status_rhai_failed",
            TenantInstance::rhai_set_status_rhai_failed,
        )
        .register_fn(
            "set_status_schedule_backup_failed",
            TenantInstance::rhai_set_status_schedule_backup_failed,
        )
        .register_fn(
            "set_status_init_failed",
            TenantInstance::rhai_set_status_init_failed,
        )
        .register_get("metadata", TenantInstance::get_metadata)
        .register_get("spec", TenantInstance::get_spec)
        .register_get("status", TenantInstance::get_status);
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_from_with_version() {
        let yaml = r#"
snapshot: "abc123"
version: "1.5.0"
"#;
        let init_from: InitFrom = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(init_from.version, Some("1.5.0".to_string()));
    }

    #[test]
    fn test_init_from_without_version() {
        let yaml = r#"
snapshot: "abc123"
"#;
        let init_from: InitFrom = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(init_from.version, None);
    }

    #[test]
    fn test_spec_version_top_level_ignored() {
        let yaml = r#"
jukebox: "my-juke"
category: "apps"
package: "nginx"
version: "1.0.0"
"#;
        let spec: TenantInstanceSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.jukebox, "my-juke");
        assert_eq!(spec.init_from, None);
    }

    #[test]
    fn test_init_version_ko_message_contains_error_id_and_version() {
        let cond = ApplicationCondition::init_version_ko("1.5.0", 3);
        assert_eq!(cond.condition_type, ConditionsType::InitFrom);
        assert_eq!(cond.status, ConditionsStatus::False);
        assert!(
            cond.message.contains("E_INIT_VERSION_NOT_FOUND"),
            "message should contain error id: {}",
            cond.message
        );
        assert!(
            cond.message.contains("1.5.0"),
            "message should contain version: {}",
            cond.message
        );
        assert_eq!(cond.generation, 3);
    }

    #[test]
    fn test_have_condition_detects_init_version_ko() {
        let cond = ApplicationCondition::init_version_ko("1.5.0", 1);
        let instance = TenantInstance {
            metadata: kube::api::ObjectMeta {
                name: Some("test".to_string()),
                namespace: Some("default".to_string()),
                generation: Some(1),
                ..Default::default()
            },
            spec: TenantInstanceSpec {
                jukebox: "jb".to_string(),
                category: "cat".to_string(),
                package: "pkg".to_string(),
                init_from: None,
                options: None,
            },
            status: Some(TenantInstanceStatus {
                conditions: vec![cond.clone()],
                tag: None,
                digest: None,
                tfstate: None,
                rhaistate: None,
                befores: None,
                vitals: None,
                scalables: None,
                others: None,
                posts: None,
                services: None,
            }),
        };
        assert!(
            instance.have_condition(&cond),
            "should detect init_version_ko condition already present"
        );
    }
}
