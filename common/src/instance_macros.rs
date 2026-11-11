/// Generates the common `ApplicationCondition` constructors shared by all three instance types.
///
/// Call this at module scope (not inside an `impl` block) in any instance module.
/// Requires: local `ApplicationCondition`, `ConditionsStatus`, `ConditionsType` in scope,
/// with at least: Ready, Installed, `AgentStarted`, `TofuInstalled`, `RhaiApplied` variants.
#[macro_export]
macro_rules! impl_condition_common {
    () => {
        impl ApplicationCondition {
            #[must_use]
            pub fn new(
                message: &str,
                status: ConditionsStatus,
                condition_type: ConditionsType,
                generation: i64,
            ) -> ApplicationCondition {
                ApplicationCondition {
                    last_transition_time: Some(::chrono::Utc::now()),
                    status,
                    condition_type,
                    message: message.to_string(),
                    generation,
                }
            }

            pub fn ready_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "Installed succesfully",
                    ConditionsStatus::True,
                    ConditionsType::Ready,
                    generation,
                )
            }

            pub fn ready_ko(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "No successful install",
                    ConditionsStatus::False,
                    ConditionsType::Ready,
                    generation,
                )
            }

            pub fn installed_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::Installed,
                    generation,
                )
            }

            pub fn installed_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "Installed succesfully",
                    ConditionsStatus::True,
                    ConditionsType::Installed,
                    generation,
                )
            }

            pub fn agent_started(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "Agent started",
                    ConditionsStatus::True,
                    ConditionsType::AgentStarted,
                    generation,
                )
            }

            pub fn missing_package(cat: &str, name: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    &format!("Package {cat}/{name} is missing"),
                    ConditionsStatus::False,
                    ConditionsType::AgentStarted,
                    generation,
                )
            }

            pub fn missing_box(name: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    &format!("JukeBox {name} is missing"),
                    ConditionsStatus::False,
                    ConditionsType::AgentStarted,
                    generation,
                )
            }

            pub fn missing_requirement(error: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    error,
                    ConditionsStatus::False,
                    ConditionsType::AgentStarted,
                    generation,
                )
            }

            pub fn tofu_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::TofuInstalled,
                    generation,
                )
            }

            pub fn tofu_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "Tofu layer applied succesfully",
                    ConditionsStatus::True,
                    ConditionsType::TofuInstalled,
                    generation,
                )
            }

            pub fn rhai_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::RhaiApplied,
                    generation,
                )
            }

            pub fn rhai_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "Custom rhai script succeed",
                    ConditionsStatus::True,
                    ConditionsType::RhaiApplied,
                    generation,
                )
            }
        }
    };
}

/// Generates `ApplicationCondition` constructors for the "children" condition types.
///
/// Covers `BeforeApplied`, `VitalApplied`, `ScalableApplied`, `OtherApplied`, `InitFrom`,
/// `ScheduleBackup`. Used by `ServiceInstance` and `TenantInstance` (not `SystemInstance`).
#[macro_export]
macro_rules! impl_condition_children {
    () => {
        impl ApplicationCondition {
            pub fn vital_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::VitalApplied,
                    generation,
                )
            }

            pub fn vital_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "vitals templates applied succesfully",
                    ConditionsStatus::True,
                    ConditionsType::VitalApplied,
                    generation,
                )
            }

            pub fn before_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::BeforeApplied,
                    generation,
                )
            }

            pub fn before_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "befores templates applied succesfully",
                    ConditionsStatus::True,
                    ConditionsType::BeforeApplied,
                    generation,
                )
            }

            pub fn scalable_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::ScalableApplied,
                    generation,
                )
            }

            pub fn scalable_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "Scalables templates applied succesfully",
                    ConditionsStatus::True,
                    ConditionsType::ScalableApplied,
                    generation,
                )
            }

            pub fn post_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::PostApplied,
                    generation,
                )
            }

            pub fn post_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "Templates applied succesfully",
                    ConditionsStatus::True,
                    ConditionsType::PostApplied,
                    generation,
                )
            }

            pub fn other_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::OtherApplied,
                    generation,
                )
            }

            pub fn other_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "Templates applied succesfully",
                    ConditionsStatus::True,
                    ConditionsType::OtherApplied,
                    generation,
                )
            }

            pub fn init_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::InitFrom,
                    generation,
                )
            }

            pub fn init_version_ko(version: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    &format!("[E_INIT_VERSION_NOT_FOUND] Version {version} introuvable dans le registre OCI"),
                    ConditionsStatus::False,
                    ConditionsType::InitFrom,
                    generation,
                )
            }

            pub fn schedule_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::ScheduleBackup,
                    generation,
                )
            }
        }
    };
}

/// Generates `ApplicationCondition` constructors for CRD conditions.
/// Used by `ServiceInstance` and `SystemInstance` (not `TenantInstance`).
#[macro_export]
macro_rules! impl_condition_crds {
    () => {
        impl ApplicationCondition {
            pub fn crd_ko(message: &str, generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    message,
                    ConditionsStatus::False,
                    ConditionsType::CrdApplied,
                    generation,
                )
            }

            pub fn crd_ok(generation: i64) -> ApplicationCondition {
                ApplicationCondition::new(
                    "CRD(s) applied succesfully",
                    ConditionsStatus::True,
                    ConditionsType::CrdApplied,
                    generation,
                )
            }
        }
    };
}

/// Generates the `impl $type` block with methods common to all three instance types:
/// CRUD helpers, status patching, event sending, tfstate/rhaistate, agent/missing-* setters,
/// and their rhai wrappers.
///
/// Required at call site (call-site imports used by the macro expansion):
/// - `use kube::{Resource, ResourceExt, api::{Api, ListParams, ObjectList, Patch, PatchParams}, ...}`
/// - local `ConditionsType`, `ApplicationCondition`
#[macro_export]
macro_rules! impl_instance_common {
    ($type:ty, $kind_str:literal) => {
        impl $type {
            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn get(namespace: String, name: String) -> $crate::Result<Self> {
                let api = ::kube::api::Api::<Self>::namespaced(
                    $crate::context::get_client_async().await,
                    &namespace,
                );
                api.get(&name).await.map_err($crate::Error::from)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn list(namespace: String) -> $crate::Result<::kube::api::ObjectList<Self>> {
                let api = ::kube::api::Api::<Self>::namespaced(
                    $crate::context::get_client_async().await,
                    &namespace,
                );
                let lp = ::kube::api::ListParams::default();
                api.list(&lp).await.map_err($crate::Error::from)
            }

            pub fn get_options_digest(&mut self) -> String {
                if let Some(ref opt) = self.spec.options {
                    sha256::digest(serde_json::to_string(opt).unwrap())
                } else {
                    sha256::digest("")
                }
            }

            /// # Errors
            ///
            /// Returns an error when the stored state cannot be decoded from base64+gzip.
            pub fn get_tfstate(&self) -> $crate::Result<Option<String>> {
                if let Some(status) = self.status.clone() {
                    if let Some(tf) = status.tfstate {
                        let decoded = $crate::tools::base64_gz_decode(tf)?;
                        Ok(Some(decoded))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }

            /// # Errors
            ///
            /// Returns an error when the stored state cannot be decoded from base64+gzip.
            pub fn get_rhaistate(&self) -> $crate::Result<Option<String>> {
                if let Some(status) = self.status.clone() {
                    if let Some(tf) = status.rhaistate {
                        let decoded = $crate::tools::base64_gz_decode(tf)?;
                        Ok(Some(decoded))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }

            fn have_condition(&self, cond: &ApplicationCondition) -> bool {
                if let Some(status) = self.status.clone() {
                    status.conditions.clone().into_iter().any(|c| {
                        c.condition_type == cond.condition_type
                            && c.generation == cond.generation
                            && c.status == cond.status
                            && c.message == cond.message
                    })
                } else {
                    false
                }
            }

            fn get_conditions_excluding(
                &self,
                exclude: Vec<ConditionsType>,
            ) -> Vec<ApplicationCondition> {
                let mut ret = Vec::new();
                if let Some(status) = self.status.clone() {
                    for c in status.conditions {
                        if !exclude.clone().into_iter().any(|exc| c.condition_type == exc) {
                            ret.push(c);
                        }
                    }
                }
                ret
            }

            async fn patch_status(
                &mut self,
                client: ::kube::Client,
                patch: serde_json::Value,
            ) -> $crate::Result<Self> {
                let api = ::kube::api::Api::<Self>::namespaced(
                    client.clone(),
                    &self.namespace().unwrap(),
                );
                let name = self.metadata.name.clone().unwrap();
                let new_status: ::kube::api::Patch<serde_json::Value> =
                    ::kube::api::Patch::Merge(serde_json::json!({
                        "apiVersion": "vynil.solidite.fr/v1",
                        "kind": $kind_str,
                        "status": patch
                    }));
                let ps = ::kube::api::PatchParams::apply(
                    $crate::context::get_short_name().as_str(),
                );
                api.patch_status(&name, &ps, &new_status)
                    .await
                    .map_err($crate::Error::from)
            }

            async fn send_event(
                &mut self,
                client: ::kube::Client,
                ev: ::kube::runtime::events::Event,
            ) -> $crate::Result<()> {
                let recorder = ::kube::runtime::events::Recorder::new(
                    client.clone(),
                    $crate::context::get_reporter(),
                );
                let oref = self.object_ref(&());
                match recorder.publish(&ev, &oref).await {
                    Ok(_) => Ok(()),
                    Err(e) => match e {
                        ::kube::Error::Api(src) => {
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
                        _ => Err($crate::Error::from(e)),
                    },
                }
            }

            /// # Errors
            ///
            /// Returns an error when the state cannot be encoded, or `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_tfstate(&mut self, tfstate: String) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let encoded = $crate::tools::encode_base64_gz(tfstate)?;
                let mut conditions: Vec<ApplicationCondition> =
                    self.get_conditions_excluding(vec![ConditionsType::TofuInstalled]);
                conditions.push(ApplicationCondition::tofu_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({
                            "conditions": conditions,
                            "tfstate": encoded
                        }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "TofuApplySucceed".to_string(),
                    note: None,
                    action: "TofuApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns an error when the state cannot be encoded, or `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_tofu_failed(
                &mut self,
                tfstate: String,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let encoded = $crate::tools::encode_base64_gz(tfstate)?;
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::TofuInstalled,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::tofu_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({
                            "conditions": conditions,
                            "tfstate": encoded
                        }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "TofuApplyFailed".to_string(),
                    note: Some(note),
                    action: "TofuApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns an error when the state cannot be encoded, or `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_rhaistate(&mut self, rhaistate: String) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let encoded = $crate::tools::encode_base64_gz(rhaistate)?;
                let mut conditions: Vec<ApplicationCondition> =
                    self.get_conditions_excluding(vec![ConditionsType::RhaiApplied]);
                conditions.push(ApplicationCondition::rhai_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({
                            "conditions": conditions,
                            "rhaistate": encoded
                        }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "RhaiApplySucceed".to_string(),
                    note: None,
                    action: "RhaiApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns an error when the state cannot be encoded, or `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_rhai_failed(
                &mut self,
                rhaistate: String,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let encoded = $crate::tools::encode_base64_gz(rhaistate)?;
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::RhaiApplied,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::rhai_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({
                            "conditions": conditions,
                            "rhaistate": encoded
                        }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "RhaiApplyFailed".to_string(),
                    note: Some(note),
                    action: "RhaiApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_agent_started(&mut self) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let cond = ApplicationCondition::agent_started(generation);
                if !self.have_condition(&cond) {
                    let mut conditions: Vec<ApplicationCondition> =
                        self.get_conditions_excluding(vec![ConditionsType::AgentStarted]);
                    conditions.push(cond);
                    let result = self
                        .patch_status(
                            client.clone(),
                            serde_json::json!({ "conditions": conditions }),
                        )
                        .await?;
                    self.send_event(client, ::kube::runtime::events::Event {
                        type_: ::kube::runtime::events::EventType::Normal,
                        reason: "AgentStarted".to_string(),
                        note: None,
                        action: "AgentStart".to_string(),
                        secondary: None,
                    })
                    .await?;
                    Ok(result)
                } else {
                    Ok(self.clone())
                }
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_missing_box(&mut self, jukebox: String) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let cond = ApplicationCondition::missing_box(&jukebox, generation);
                if !self.have_condition(&cond) {
                    let mut conditions: Vec<ApplicationCondition> =
                        self.get_conditions_excluding(vec![ConditionsType::AgentStarted]);
                    conditions.push(cond);
                    let result = self
                        .patch_status(
                            client.clone(),
                            serde_json::json!({ "conditions": conditions }),
                        )
                        .await?;
                    self.send_event(client, ::kube::runtime::events::Event {
                        type_: ::kube::runtime::events::EventType::Warning,
                        reason: "MissingJukebox".to_string(),
                        note: Some(format!("JukeBox {jukebox} doesnt exist")),
                        action: "AgentStart".to_string(),
                        secondary: None,
                    })
                    .await?;
                    Ok(result)
                } else {
                    Ok(self.clone())
                }
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_missing_package(
                &mut self,
                category: String,
                package: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let cond = ApplicationCondition::missing_package(&category, &package, generation);
                if !self.have_condition(&cond) {
                    let mut conditions: Vec<ApplicationCondition> =
                        self.get_conditions_excluding(vec![ConditionsType::AgentStarted]);
                    conditions.push(cond);
                    let result = self
                        .patch_status(
                            client.clone(),
                            serde_json::json!({ "conditions": conditions }),
                        )
                        .await?;
                    self.send_event(client, ::kube::runtime::events::Event {
                        type_: ::kube::runtime::events::EventType::Warning,
                        reason: "MissingPackage".to_string(),
                        note: Some(format!("Package {category}/{package} doesnt exist")),
                        action: "AgentStart".to_string(),
                        secondary: None,
                    })
                    .await?;
                    Ok(result)
                } else {
                    Ok(self.clone())
                }
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_missing_requirement(
                &mut self,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let cond = ApplicationCondition::missing_requirement(&reason, generation);
                if !self.have_condition(&cond) {
                    let mut conditions: Vec<ApplicationCondition> =
                        self.get_conditions_excluding(vec![ConditionsType::AgentStarted]);
                    conditions.push(cond);
                    let result = self
                        .patch_status(
                            client.clone(),
                            serde_json::json!({ "conditions": conditions }),
                        )
                        .await?;
                    let mut note = reason;
                    note.truncate(1023);
                    self.send_event(client, ::kube::runtime::events::Event {
                        type_: ::kube::runtime::events::EventType::Warning,
                        reason: "MissingRequirement".to_string(),
                        note: Some(note),
                        action: "AgentStart".to_string(),
                        secondary: None,
                    })
                    .await?;
                    Ok(result)
                } else {
                    Ok(self.clone())
                }
            }

            // ── Rhai wrappers ─────────────────────────────────────────────────────────

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_get(namespace: String, name: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { Self::get(namespace, name).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_list(namespace: String) -> $crate::RhaiRes<Vec<Self>> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { Self::list(namespace).await })
                })
                .map_err($crate::rhai_err)
                .map(|lst| lst.into_iter().collect())
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub fn get_metadata(&mut self) -> $crate::RhaiRes<::rhai::Dynamic> {
                let v = serde_json::to_string(&self.metadata)
                    .map_err(|e| $crate::rhai_err($crate::Error::SerializationError(e)))?;
                serde_json::from_str(&v)
                    .map_err(|e| $crate::rhai_err($crate::Error::SerializationError(e)))
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub fn get_spec(&mut self) -> $crate::RhaiRes<::rhai::Dynamic> {
                let v = serde_json::to_string(&self.spec)
                    .map_err(|e| $crate::rhai_err($crate::Error::SerializationError(e)))?;
                serde_json::from_str(&v)
                    .map_err(|e| $crate::rhai_err($crate::Error::SerializationError(e)))
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub fn get_status(&mut self) -> $crate::RhaiRes<::rhai::Dynamic> {
                let v = serde_json::to_string(&self.status)
                    .map_err(|e| $crate::rhai_err($crate::Error::SerializationError(e)))?;
                serde_json::from_str(&v)
                    .map_err(|e| $crate::rhai_err($crate::Error::SerializationError(e)))
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_get_tfstate(&mut self) -> $crate::RhaiRes<String> {
                self.get_tfstate()
                    .map_err($crate::rhai_err)
                    .map(|r| r.unwrap_or_default())
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_get_rhaistate(&mut self) -> $crate::RhaiRes<String> {
                self.get_rhaistate()
                    .map_err($crate::rhai_err)
                    .map(|r| r.unwrap_or_default())
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_ready(&mut self, tag: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_status_ready(tag).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_tfstate(&mut self, tfstate: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_tfstate(tfstate).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_tofu_failed(
                &mut self,
                tfstate: String,
                reason: String,
            ) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        self.set_status_tofu_failed(tfstate, reason).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_rhaistate(&mut self, rhaistate: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_rhaistate(rhaistate).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_rhai_failed(
                &mut self,
                rhaistate: String,
                reason: String,
            ) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        self.set_status_rhai_failed(rhaistate, reason).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_agent_started(&mut self) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_agent_started().await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_missing_box(&mut self, jukebox: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_missing_box(jukebox).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_missing_package(
                &mut self,
                category: String,
                package: String,
            ) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        self.set_missing_package(category, package).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_missing_requirement(
                &mut self,
                reason: String,
            ) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        self.set_missing_requirement(reason).await
                    })
                })
                .map_err($crate::rhai_err)
            }
        }
    };
}

/// Generates the `impl $type` block for "full application" instance types.
///
/// Those types manage children resources: befores, vitals, scalables, others, services, init,
/// schedule. Used by `ServiceInstance` and `TenantInstance` (not `SystemInstance`).
/// Also generates `set_status_ready` which excludes BeforeApplied/InitFrom/ScheduleBackup.
#[macro_export]
macro_rules! impl_instance_befores {
    ($type:ty) => {
        impl $type {
            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_ready(&mut self, tag: String) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::BeforeApplied,
                    ConditionsType::Ready,
                    ConditionsType::Installed,
                    ConditionsType::InitFrom,
                    ConditionsType::ScheduleBackup,
                ]);
                conditions.push(ApplicationCondition::ready_ok(generation));
                conditions.push(ApplicationCondition::installed_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({
                            "conditions": conditions,
                            "tag": tag,
                            "digest": self.clone().get_options_digest()
                        }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "InstallSucceed".to_string(),
                    note: None,
                    action: "Install".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            pub fn get_services(&self) -> Vec<$crate::Published> {
                if let Some(status) = self.status.clone() {
                    status.services.unwrap_or_default()
                } else {
                    Vec::new()
                }
            }

            pub fn get_services_string(&self) -> String {
                if let Some(status) = self.status.clone() {
                    if let Some(svcs) = status.services {
                        let mut tmp: Vec<String> = svcs.iter().map(|s| s.key.clone()).collect();
                        tmp.sort();
                        tmp.join(",")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_services(
                &mut self,
                services: Vec<$crate::Published>,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let result = self
                    .patch_status(client.clone(), serde_json::json!({ "services": services }))
                    .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_befores(
                &mut self,
                befores: Vec<$crate::Children>,
            ) -> $crate::Result<Self> {
                let count = befores.len();
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> =
                    self.get_conditions_excluding(vec![ConditionsType::BeforeApplied]);
                conditions.push(ApplicationCondition::before_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions, "befores": befores }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "BeforeApplySucceed".to_string(),
                    note: Some(format!("Applied {} Objects", count)),
                    action: "BeforeApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_before_failed(
                &mut self,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::BeforeApplied,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::before_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "BeforeApplyFailed".to_string(),
                    note: Some(note),
                    action: "BeforeApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_vitals(
                &mut self,
                vitals: Vec<$crate::Children>,
            ) -> $crate::Result<Self> {
                let count = vitals.len();
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> =
                    self.get_conditions_excluding(vec![ConditionsType::VitalApplied]);
                conditions.push(ApplicationCondition::vital_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions, "vitals": vitals }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "VitalApplySucceed".to_string(),
                    note: Some(format!("Applied {} Objects", count)),
                    action: "VitalApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_vital_failed(
                &mut self,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::VitalApplied,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::vital_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "VitalApplyFailed".to_string(),
                    note: Some(note),
                    action: "VitalApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_scalables(
                &mut self,
                scalables: Vec<$crate::Children>,
            ) -> $crate::Result<Self> {
                let count = scalables.len();
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> =
                    self.get_conditions_excluding(vec![ConditionsType::ScalableApplied]);
                conditions.push(ApplicationCondition::scalable_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions, "scalables": scalables }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "ScalableApplySucceed".to_string(),
                    note: Some(format!("Applied {} Objects", count)),
                    action: "ScalableApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_scalable_failed(
                &mut self,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::ScalableApplied,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::scalable_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "ScalableApplyFailed".to_string(),
                    note: Some(note),
                    action: "ScalableApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_others(
                &mut self,
                others: Vec<$crate::Children>,
            ) -> $crate::Result<Self> {
                let count = others.len();
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> =
                    self.get_conditions_excluding(vec![ConditionsType::OtherApplied]);
                conditions.push(ApplicationCondition::other_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions, "others": others }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "OtherApplySucceed".to_string(),
                    note: Some(format!("Applied {} Objects", count)),
                    action: "OtherApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_other_failed(
                &mut self,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::OtherApplied,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::other_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "OtherApplyFailed".to_string(),
                    note: Some(note),
                    action: "OtherApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_posts(
                &mut self,
                posts: Vec<$crate::Children>,
            ) -> $crate::Result<Self> {
                let count = posts.len();
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> =
                    self.get_conditions_excluding(vec![ConditionsType::PostApplied]);
                conditions.push(ApplicationCondition::post_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions, "posts": posts }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "PostApplySucceed".to_string(),
                    note: Some(format!("Applied {} Objects", count)),
                    action: "PostApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_post_failed(
                &mut self,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::PostApplied,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::post_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "OtherApplyFailed".to_string(),
                    note: Some(note),
                    action: "OtherApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_init_failed(&mut self, reason: String) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::InitFrom,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::init_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "InitFromFail".to_string(),
                    note: Some(note),
                    action: "InitFrom".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_schedule_backup_failed(
                &mut self,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::ScheduleBackup,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::schedule_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "ScheduleBackupFailed".to_string(),
                    note: Some(note),
                    action: "ScheduleBackup".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            // ── Rhai wrappers ─────────────────────────────────────────────────────────

            /// # Errors
            ///
            /// Returns a Rhai error when the `Dynamic` argument cannot be converted to JSON or the underlying Kubernetes operation fails.
            pub fn rhai_set_services(&mut self, services: ::rhai::Dynamic) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        let v = serde_json::to_string(&services)
                            .map_err($crate::Error::SerializationError)?;
                        let lst =
                            serde_json::from_str(&v).map_err($crate::Error::SerializationError)?;
                        self.set_services(lst).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            pub fn rhai_get_services(&mut self) -> String {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.get_services_string() })
                })
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the `Dynamic` argument cannot be converted to JSON or the underlying Kubernetes operation fails.
            pub fn rhai_set_status_befores(&mut self, list: ::rhai::Dynamic) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        let v = serde_json::to_string(&list)
                            .map_err($crate::Error::SerializationError)?;
                        let lst =
                            serde_json::from_str(&v).map_err($crate::Error::SerializationError)?;
                        self.set_status_befores(lst).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_before_failed(&mut self, reason: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_status_before_failed(reason).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the `Dynamic` argument cannot be converted to JSON or the underlying Kubernetes operation fails.
            pub fn rhai_set_status_vitals(&mut self, list: ::rhai::Dynamic) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        let v = serde_json::to_string(&list)
                            .map_err($crate::Error::SerializationError)?;
                        let lst =
                            serde_json::from_str(&v).map_err($crate::Error::SerializationError)?;
                        self.set_status_vitals(lst).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_vital_failed(&mut self, reason: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_status_vital_failed(reason).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the `Dynamic` argument cannot be converted to JSON or the underlying Kubernetes operation fails.
            pub fn rhai_set_status_scalables(
                &mut self,
                list: ::rhai::Dynamic,
            ) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        let v = serde_json::to_string(&list)
                            .map_err($crate::Error::SerializationError)?;
                        let lst =
                            serde_json::from_str(&v).map_err($crate::Error::SerializationError)?;
                        self.set_status_scalables(lst).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_scalable_failed(
                &mut self,
                reason: String,
            ) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_status_scalable_failed(reason).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the `Dynamic` argument cannot be converted to JSON or the underlying Kubernetes operation fails.
            pub fn rhai_set_status_others(&mut self, list: ::rhai::Dynamic) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        let v = serde_json::to_string(&list)
                            .map_err($crate::Error::SerializationError)?;
                        let lst =
                            serde_json::from_str(&v).map_err($crate::Error::SerializationError)?;
                        self.set_status_others(lst).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_other_failed(&mut self, reason: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_status_other_failed(reason).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the `Dynamic` argument cannot be converted to JSON or the underlying Kubernetes operation fails.
            pub fn rhai_set_status_posts(&mut self, list: ::rhai::Dynamic) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        let v = serde_json::to_string(&list)
                            .map_err($crate::Error::SerializationError)?;
                        let lst =
                            serde_json::from_str(&v).map_err($crate::Error::SerializationError)?;
                        self.set_status_posts(lst).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_post_failed(&mut self, reason: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_status_post_failed(reason).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_init_failed(&mut self, reason: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_status_init_failed(reason).await })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_schedule_backup_failed(
                &mut self,
                reason: String,
            ) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        self.set_status_schedule_backup_failed(reason).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_missing_init_version(
                &mut self,
                version: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let cond = ApplicationCondition::init_version_ko(&version, generation);
                if !self.have_condition(&cond) {
                    let mut conditions: Vec<ApplicationCondition> =
                        self.get_conditions_excluding(vec![ConditionsType::InitFrom]);
                    conditions.push(cond);
                    let result = self
                        .patch_status(
                            client.clone(),
                            serde_json::json!({ "conditions": conditions }),
                        )
                        .await?;
                    self.send_event(client, ::kube::runtime::events::Event {
                        type_: ::kube::runtime::events::EventType::Warning,
                        reason: "InitVersionNotFound".to_string(),
                        note: Some(format!(
                            "[E_INIT_VERSION_NOT_FOUND] Version {version} introuvable dans le registre OCI"
                        )),
                        action: "InitFrom".to_string(),
                        secondary: None,
                    })
                    .await?;
                    Ok(result)
                } else {
                    Ok(self.clone())
                }
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_missing_init_version(
                &mut self,
                version: String,
            ) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_missing_init_version(version).await })
                })
                .map_err($crate::rhai_err)
            }
        }
    };
}

/// Generates the `impl $type` block for CRD management.
/// Used by `ServiceInstance` and `SystemInstance` (not `TenantInstance`).
#[macro_export]
macro_rules! impl_instance_crds {
    ($type:ty) => {
        impl $type {
            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_crds(&mut self, crds: Vec<String>) -> $crate::Result<Self> {
                let count = crds.len();
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> =
                    self.get_conditions_excluding(vec![ConditionsType::CrdApplied]);
                conditions.push(ApplicationCondition::crd_ok(generation));
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions, "crds": crds }),
                    )
                    .await?;
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Normal,
                    reason: "CRDApplySucceed".to_string(),
                    note: Some(format!("Applied {} CustomResourceDefinition", count)),
                    action: "CRDApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns `Error::KubeError` when the Kubernetes API call fails.
            pub async fn set_status_crd_failed(
                &mut self,
                reason: String,
            ) -> $crate::Result<Self> {
                let client = $crate::context::get_client_async().await;
                let generation = self.metadata.generation.unwrap_or(1);
                let mut conditions: Vec<ApplicationCondition> = self.get_conditions_excluding(vec![
                    ConditionsType::AgentStarted,
                    ConditionsType::CrdApplied,
                    ConditionsType::Installed,
                ]);
                conditions.push(ApplicationCondition::crd_ko(&reason, generation));
                conditions.push(ApplicationCondition::installed_ko(&reason, generation));
                if !conditions
                    .clone()
                    .into_iter()
                    .any(|c| c.condition_type == ConditionsType::Ready)
                {
                    conditions.push(ApplicationCondition::ready_ko(generation));
                }
                let result = self
                    .patch_status(
                        client.clone(),
                        serde_json::json!({ "conditions": conditions }),
                    )
                    .await?;
                let mut note = reason;
                note.truncate(1023);
                self.send_event(client, ::kube::runtime::events::Event {
                    type_: ::kube::runtime::events::EventType::Warning,
                    reason: "CRDApplyFailed".to_string(),
                    note: Some(note),
                    action: "CRDApply".to_string(),
                    secondary: None,
                })
                .await?;
                Ok(result)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the `Dynamic` argument cannot be converted to JSON or the underlying Kubernetes operation fails.
            pub fn rhai_set_status_crds(&mut self, list: ::rhai::Dynamic) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current().block_on(async move {
                        let v = serde_json::to_string(&list)
                            .map_err($crate::Error::SerializationError)?;
                        let lst =
                            serde_json::from_str(&v).map_err($crate::Error::SerializationError)?;
                        self.set_status_crds(lst).await
                    })
                })
                .map_err($crate::rhai_err)
            }

            /// # Errors
            ///
            /// Returns a Rhai error when the underlying Kubernetes operation fails.
            pub fn rhai_set_status_crd_failed(&mut self, reason: String) -> $crate::RhaiRes<Self> {
                ::tokio::task::block_in_place(|| {
                    ::tokio::runtime::Handle::current()
                        .block_on(async move { self.set_status_crd_failed(reason).await })
                })
                .map_err($crate::rhai_err)
            }
        }
    };
}
