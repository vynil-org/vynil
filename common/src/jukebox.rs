use crate::{
    Error, Result, RhaiRes,
    context::{get_client_async, get_reporter, get_short_name},
    rhai_err,
    vynilpackage::VynilPackage,
};
use chrono::{DateTime, Utc};
use kube::{
    Client, CustomResource, Resource,
    api::{Api, ListParams, ObjectList, Patch, PatchParams},
    runtime::events::{Event, EventType, Recorder},
};
use rhai::{Dynamic, Engine};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{runtime::Handle, task::block_in_place};

/// `JukeBox` Source type
#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JukeBoxDef {
    /// List of oci images
    List(Vec<String>),
    /// Harbor project to list image from
    Harbor { registry: String, project: String },
    /// GitLab Container Registry project to list images from
    Gitlab {
        /// GitLab instance API URL (e.g. <https://gitlab.com>)
        url: String,
        /// OCI registry URL (e.g. registry.gitlab.com)
        registry: String,
        /// Project path with namespace (e.g. my-group/my-project)
        project: String,
    },
    /// Custom script that produce the image list
    Script(String),
    /// HTTP server hosting an index.yaml package cache
    Http {
        /// Base URL containing index.yaml
        url: String,
        /// K8s Opaque secret: keys `username`+`password` (Basic) or `token` (Bearer)
        secret: Option<String>,
    },
    /// S3 bucket hosting a package cache
    S3 {
        bucket: String,
        region: String,
        /// Prefix in the bucket (e.g. "vynil/packages/")
        prefix: Option<String>,
        /// S3-compatible endpoint (`MinIO`, OVH, etc.)
        endpoint: Option<String>,
        /// K8s Opaque secret: keys `access_key_id` and `secret_access_key`
        /// Absent = IAM role / instance profile
        secret: Option<String>,
    },
}
impl Default for JukeBoxDef {
    fn default() -> Self {
        Self::List(["ghcr.io/vynil-org/vynil".to_string()].into())
    }
}

/// `JukeBox` Maturity
#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum JukeBoxMaturity {
    #[default]
    Stable,
    Beta,
    Alpha,
}

/// Describe a source of vynil packages jukebox
#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    kind = "JukeBox",
    status = "JukeBoxStatus",
    shortname = "box",
    group = "vynil.solidite.fr",
    version = "v1"
)]
#[kube(
    doc = "Custom resource representing a JukeBox, source of vynil packages",
    printcolumn = r#"{"name":"schedule",    "type":"string", "description":"Update schedule",   "jsonPath":".spec.schedule"}"#,
    printcolumn = r#"{"name":"last_updated","type":"date",   "description":"Last update date",  "format":"date-time", "jsonPath":".status.conditions[?(@.type == 'Ready')].lastTransitionTime"}"#,
    printcolumn = r#"{"name":"message",     "type":"string", "description":"Message",           "jsonPath":".status.conditions[?(@.type == 'Updated')].message"}"#
)]
pub struct JukeBoxSpec {
    /// Source type
    pub source: Option<JukeBoxDef>,
    /// Jukebox maturity (stable/beta/alpha)
    pub maturity: Option<JukeBoxMaturity>,
    /// `ImagePullSecret` name in the vynil-system namespace
    pub pull_secret: Option<String>,
    /// Actual cron-type expression that defines the interval of the updates.
    pub schedule: String,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema, Default)]
pub enum ConditionsType {
    #[default]
    Ready,
    Updated,
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

impl ApplicationCondition {
    #[must_use]
    pub fn new(
        message: &str,
        status: ConditionsStatus,
        condition_type: ConditionsType,
        generation: i64,
    ) -> Self {
        Self {
            last_transition_time: Some(chrono::offset::Utc::now()),
            status,
            condition_type,
            message: message.to_string(),
            generation,
        }
    }

    #[must_use]
    pub fn ready_ok(generation: i64) -> Self {
        Self::new(
            "Updated succesfully",
            ConditionsStatus::True,
            ConditionsType::Ready,
            generation,
        )
    }

    #[must_use]
    pub fn ready_ko(generation: i64) -> Self {
        Self::new(
            "No successful update",
            ConditionsStatus::False,
            ConditionsType::Ready,
            generation,
        )
    }

    #[must_use]
    pub fn updated_ko(message: &str, generation: i64) -> Self {
        Self::new(
            message,
            ConditionsStatus::False,
            ConditionsType::Updated,
            generation,
        )
    }

    #[must_use]
    pub fn updated_ok(generation: i64) -> Self {
        Self::new(
            "Updated succesfully",
            ConditionsStatus::True,
            ConditionsType::Updated,
            generation,
        )
    }
}

/// The status object of `JukeBox`
#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct JukeBoxStatus {
    /// `JukeBox` Conditions
    pub conditions: Vec<ApplicationCondition>,
    /// Vynil packages for this box
    pub packages: Vec<VynilPackage>,
}

impl JukeBox {
    ///
    /// # Errors
    ///
    /// Returns `Error::KubeError` when the Kubernetes API call fails.
    pub async fn get(name: String) -> Result<Self> {
        let api = Api::<Self>::all(get_client_async().await);
        api.get(&name).await.map_err(Error::KubeError)
    }

    ///
    /// # Errors
    ///
    /// Returns `Error::KubeError` when the Kubernetes API call fails.
    pub async fn list() -> Result<ObjectList<Self>> {
        let api = Api::<Self>::all(get_client_async().await);
        let lp = ListParams::default();
        api.list(&lp).await.map_err(Error::KubeError)
    }

    ///
    /// # Errors
    ///
    /// Returns `Error::KubeError` when the Kubernetes API call fails.
    pub async fn list_with_client(cl: Client) -> Result<ObjectList<Self>> {
        let api = Api::<Self>::all(cl);
        let lp = ListParams::default();
        api.list(&lp).await.map_err(Error::KubeError)
    }

    fn get_conditions_excluding(&self, exclude: &[ConditionsType]) -> Vec<ApplicationCondition> {
        let mut ret = Vec::new();
        if let Some(status) = self.status.clone() {
            for c in status.conditions {
                if !exclude.contains(&c.condition_type) {
                    ret.push(c);
                }
            }
        }
        ret
    }

    async fn patch_status(&self, client: Client, patch: serde_json::Value) -> Result<Self> {
        let api = Api::<Self>::all(client.clone());
        let name = self
            .metadata
            .name
            .clone()
            .ok_or_else(|| Error::Other("JukeBox has no name".to_string()))?;
        let new_status: Patch<serde_json::Value> = Patch::Merge(json!({
            "apiVersion": "vynil.solidite.fr/v1",
            "kind": "JukeBox",
            "status": patch
        }));
        let ps = PatchParams::apply(get_short_name().as_str());
        api.patch_status(&name, &ps, &new_status)
            .await
            .map_err(Error::KubeError)
    }

    async fn send_event(&self, client: Client, ev: Event) -> Result<()> {
        let recorder = Recorder::new(client.clone(), get_reporter());
        let oref = self.object_ref(&());
        match recorder.publish(&ev, &oref).await {
            Ok(()) => Ok(()),
            Err(e) => match e {
                kube::Error::Api(src) => {
                    if !src
                        .message
                        .as_str()
                        .contains("unable to create new content in namespace")
                        || !src.message.as_str().contains("being terminated")
                    {
                        tracing::warn!("Ignoring {:?} while sending an event", src);
                    }
                    Ok(())
                }
                _ => Err(Error::KubeError(e)),
            },
        }
    }

    ///
    /// # Errors
    ///
    /// Returns `Error::KubeError` when the Kubernetes API call fails.
    pub async fn set_status_updated(&mut self, packages: Vec<VynilPackage>) -> Result<Self> {
        let count = packages.len();
        let client = get_client_async().await;
        let generation = self.metadata.generation.unwrap_or(1);
        let conditions: Vec<ApplicationCondition> = vec![
            ApplicationCondition::updated_ok(generation),
            ApplicationCondition::ready_ok(generation),
        ];
        let result = self
            .patch_status(
                client.clone(),
                json!({
                    "conditions": conditions,
                    "packages": packages,
                }),
            )
            .await?;
        self.send_event(client, Event {
            type_: EventType::Normal,
            reason: "ScanSucceed".to_string(),
            note: Some(format!("Found {count} packages")),
            action: "Scan".to_string(),
            secondary: None,
        })
        .await?;
        Ok(result)
    }

    ///
    /// # Errors
    ///
    /// Returns `Error::KubeError` when the Kubernetes API call fails.
    pub async fn set_status_failed(&mut self, reason: String) -> Result<Self> {
        let client = get_client_async().await;
        let generation = self.metadata.generation.unwrap_or(1);
        let mut conditions: Vec<ApplicationCondition> =
            self.get_conditions_excluding(&[ConditionsType::Updated]);
        conditions.push(ApplicationCondition::updated_ko(&reason, generation));
        if !conditions
            .clone()
            .into_iter()
            .any(|c| c.condition_type == ConditionsType::Ready)
        {
            conditions.push(ApplicationCondition::ready_ko(generation));
        }
        let existing_packages = self
            .status
            .as_ref()
            .map(|s| s.packages.clone())
            .unwrap_or_default();
        let result = self
            .patch_status(
                client.clone(),
                json!({
                    "conditions": conditions,
                    "packages": existing_packages,
                }),
            )
            .await?;
        let mut note = reason;
        note.truncate(1023);
        self.send_event(client, Event {
            type_: EventType::Warning,
            reason: "ScanFailed".to_string(),
            note: Some(note),
            action: "Scan".to_string(),
            secondary: None,
        })
        .await?;
        Ok(result)
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the underlying operation fails.
    pub fn rhai_get(name: String) -> RhaiRes<Self> {
        block_in_place(|| Handle::current().block_on(async move { Self::get(name).await })).map_err(rhai_err)
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the underlying operation fails.
    pub fn rhai_list() -> RhaiRes<Vec<Self>> {
        block_in_place(|| Handle::current().block_on(async move { Self::list().await }))
            .map_err(rhai_err)
            .map(|lst| lst.into_iter().collect())
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the section cannot be converted into a rhai `Dynamic`.
    pub fn get_metadata(&mut self) -> RhaiRes<Dynamic> {
        let v = serde_json::to_string(&self.metadata).map_err(|e| rhai_err(Error::SerializationError(e)))?;
        serde_json::from_str(&v).map_err(|e| rhai_err(Error::SerializationError(e)))
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the section cannot be converted into a rhai `Dynamic`.
    pub fn get_spec(&mut self) -> RhaiRes<Dynamic> {
        let v = serde_json::to_string(&self.spec).map_err(|e| rhai_err(Error::SerializationError(e)))?;
        serde_json::from_str(&v).map_err(|e| rhai_err(Error::SerializationError(e)))
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the section cannot be converted into a rhai `Dynamic`.
    pub fn get_status(&mut self) -> RhaiRes<Dynamic> {
        let v = serde_json::to_string(&self.status).map_err(|e| rhai_err(Error::SerializationError(e)))?;
        serde_json::from_str(&v).map_err(|e| rhai_err(Error::SerializationError(e)))
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the rhai argument cannot be converted or the underlying
    /// operation fails.
    pub fn rhai_set_status_updated(&mut self, list: Dynamic) -> RhaiRes<Self> {
        block_in_place(|| {
            Handle::current().block_on(async move {
                let v = serde_json::to_string(&list).map_err(Error::SerializationError)?;
                let parsed = serde_json::from_str(&v).map_err(Error::SerializationError)?;
                self.set_status_updated(parsed).await
            })
        })
        .map_err(rhai_err)
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the underlying operation fails.
    pub fn rhai_set_status_failed(&mut self, reason: String) -> RhaiRes<Self> {
        block_in_place(|| Handle::current().block_on(async move { self.set_status_failed(reason).await }))
            .map_err(rhai_err)
    }

    ///
    /// # Errors
    ///
    /// Returns `Error::KubeError` when the Kubernetes API call fails.
    pub async fn set_status_packages_merge(
        &mut self,
        filter: String,
        packages: Vec<VynilPackage>,
    ) -> Result<Self> {
        let client = get_client_async().await;
        let generation = self.metadata.generation.unwrap_or(1);

        let (filter_category, filter_name): (String, Option<String>) = filter.find('/').map_or_else(
            || (filter.clone(), None),
            |pos| {
                (
                    filter[..pos].to_string(),
                    filter.get(pos.saturating_add(1)..).map(ToString::to_string),
                )
            },
        );

        let existing = self
            .status
            .as_ref()
            .map(|s| s.packages.clone())
            .unwrap_or_default();
        let merged = filter_packages(existing, &filter_category, filter_name.as_deref(), packages);

        let conditions = vec![
            ApplicationCondition::updated_ok(generation),
            ApplicationCondition::ready_ok(generation),
        ];
        let result = self
            .patch_status(
                client.clone(),
                json!({
                    "conditions": conditions,
                    "packages": merged,
                }),
            )
            .await?;

        self.send_event(client, Event {
            type_: EventType::Normal,
            reason: "ScanSucceed".to_string(),
            note: Some(format!("Partial scan updated filter: {filter}")),
            action: "Scan".to_string(),
            secondary: None,
        })
        .await?;
        Ok(result)
    }

    ///
    /// # Errors
    ///
    /// Returns a Rhai error when the rhai argument cannot be converted or the underlying
    /// operation fails.
    pub fn rhai_set_status_packages_merge(&mut self, filter: String, list: Dynamic) -> RhaiRes<Self> {
        block_in_place(|| {
            Handle::current().block_on(async move {
                let v = serde_json::to_string(&list).map_err(Error::SerializationError)?;
                let parsed = serde_json::from_str(&v).map_err(Error::SerializationError)?;
                self.set_status_packages_merge(filter, parsed).await
            })
        })
        .map_err(rhai_err)
    }
}

fn filter_packages(
    mut existing: Vec<VynilPackage>,
    filter_category: &str,
    filter_name: Option<&str>,
    new_packages: Vec<VynilPackage>,
) -> Vec<VynilPackage> {
    existing.retain(|p| {
        let cat_match = p.metadata.category == filter_category;
        let name_match = filter_name.is_none_or(|n| p.metadata.name == n);
        !(cat_match && name_match)
    });
    existing.extend(new_packages);
    existing
}

pub fn jukebox_rhai_register(engine: &mut Engine) {
    engine
        .register_type_with_name::<JukeBox>("JukeBox")
        .register_fn("get_jukebox", JukeBox::rhai_get)
        .register_fn("list_jukebox", JukeBox::rhai_list)
        .register_fn("set_status_updated", JukeBox::rhai_set_status_updated)
        .register_fn("set_status_failed", JukeBox::rhai_set_status_failed)
        .register_fn(
            "set_status_packages_merge",
            JukeBox::rhai_set_status_packages_merge,
        )
        .register_get("metadata", JukeBox::get_metadata)
        .register_get("spec", JukeBox::get_spec)
        .register_get("status", JukeBox::get_status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vynilpackage::{VynilPackageMeta, VynilPackageType};

    #[test]
    fn jukebox_def_http_serde_roundtrip() {
        let val = JukeBoxDef::Http {
            url: "https://example.com/cache".to_string(),
            secret: None,
        };
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("\"http\""));
        assert!(json.contains("https://example.com/cache"));
        let back: JukeBoxDef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, val);
    }

    #[test]
    fn jukebox_def_s3_serde_roundtrip() {
        let val = JukeBoxDef::S3 {
            bucket: "my-bucket".to_string(),
            region: "eu-west-1".to_string(),
            prefix: Some("vynil/".to_string()),
            endpoint: None,
            secret: None,
        };
        let json = serde_json::to_string(&val).unwrap();
        assert!(json.contains("\"s3\""));
        let back: JukeBoxDef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, val);
    }

    #[test]
    fn jukebox_def_http_yaml_roundtrip() {
        let val = JukeBoxDef::Http {
            url: "https://example.com/cache".to_string(),
            secret: None,
        };
        let yaml = serde_yaml::to_string(&val).unwrap();
        let back: JukeBoxDef = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back, val);
    }

    #[test]
    fn default_source_is_maintainer_image_list() {
        assert_eq!(
            JukeBoxDef::default(),
            JukeBoxDef::List(vec!["ghcr.io/vynil-org/vynil".to_string()])
        );
    }

    fn make_pkg(category: &str, name: &str) -> VynilPackage {
        VynilPackage {
            registry: String::new(),
            image: String::new(),
            tag: String::new(),
            metadata: VynilPackageMeta {
                name: name.to_string(),
                category: category.to_string(),
                description: String::new(),
                app_version: None,
                usage: VynilPackageType::default(),
                features: vec![],
                backup_affinity: None,
            },
            requirements: vec![],
            recommandations: None,
            options: None,
            value_script: None,
        }
    }

    fn initial_packages() -> Vec<VynilPackage> {
        vec![
            make_pkg("db", "pg"),
            make_pkg("db", "mysql"),
            make_pkg("monitoring", "prom"),
        ]
    }

    #[test]
    fn filter_by_category_removes_all_matching() {
        let result = filter_packages(initial_packages(), "db", None, vec![make_pkg("db", "pg")]);
        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .any(|p| p.metadata.category == "monitoring" && p.metadata.name == "prom")
        );
        assert!(
            result
                .iter()
                .any(|p| p.metadata.category == "db" && p.metadata.name == "pg")
        );
        assert!(!result.iter().any(|p| p.metadata.name == "mysql"));
    }

    #[test]
    fn filter_by_category_name_removes_only_matching() {
        let result = filter_packages(initial_packages(), "db", Some("pg"), vec![make_pkg("db", "pg")]);
        assert_eq!(result.len(), 3);
        assert!(result.iter().any(|p| p.metadata.name == "mysql"));
        assert!(result.iter().any(|p| p.metadata.name == "prom"));
    }

    #[test]
    fn filter_no_match_appends_new_packages() {
        let result = filter_packages(initial_packages(), "storage", None, vec![make_pkg(
            "storage", "ceph",
        )]);
        assert_eq!(result.len(), 4);
        assert!(result.iter().any(|p| p.metadata.name == "ceph"));
    }

    #[test]
    fn filter_empty_existing_returns_new_packages() {
        let result = filter_packages(vec![], "db", None, vec![make_pkg("db", "pg")]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].metadata.name, "pg");
    }
}
