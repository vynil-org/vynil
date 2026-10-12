#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(not(test), warn(clippy::arithmetic_side_effects, clippy::indexing_slicing))]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

pub static DEFAULT_AGENT_IMAGE: &str = concat!("ghcr.io/vynil-org/vynil-agent:", env!("CARGO_PKG_VERSION"));

#[derive(Error, Debug)]
pub enum Error {
    #[error("SerializationError: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("YamlError: {0}")]
    YamlError(String),

    #[error("K8s error: {0}")]
    KubeError(#[from] kube::Error),

    #[error("K8s wait error: {0}")]
    KubeWaitError(#[from] kube::runtime::wait::Error),

    #[error("Elapsed wait error: {0}")]
    Elapsed(#[from] tokio::time::error::Elapsed),

    #[error("Finalizer error: {0}")]
    // NB: awkward type because finalizer::Error embeds the reconciler error (which is this)
    // so boxing this error to break cycles
    FinalizerError(#[from] Box<kube::runtime::finalizer::Error<Self>>),

    #[error("Registering template failed with error: {0}")]
    HbsTemplateError(#[from] handlebars::TemplateError),
    #[error("Renderer error: {0}")]
    HbsRenderError(#[from] handlebars::RenderError),

    #[error("Rhai script error: {0}")]
    RhaiError(#[from] Box<rhai::EvalAltResult>),

    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("Json decoding error: {0}")]
    JsonError(#[source] serde_json::Error),

    #[error("{0} query failed: {1}")]
    MethodFailed(String, u16, String),

    #[error("Unsupported method")]
    UnsupportedMethod,

    #[error("Missing script {0}")]
    MissingScript(PathBuf),

    #[error("Missing destination directory for {0}")]
    MissingDestination(PathBuf),

    #[error("Missing tests directory {0}")]
    MissingTestDirectory(PathBuf),

    #[error("UTF8 error {0}")]
    UTF8(#[from] std::string::FromUtf8Error),

    #[error("Semver error {0}")]
    Semver(#[from] semver::Error),

    #[error("Argon2 password_hash error {0}")]
    Argon2hash(#[from] argon2::password_hash::Error),

    #[error("Bcrypt hash error {0}")]
    BcryptError(#[from] bcrypt::BcryptError),

    #[error("Stdio error {0}")]
    Stdio(#[from] std::io::Error),

    #[error("Base64 decode error {0}")]
    Base64DecodeError(#[from] base64::DecodeError),

    #[error("RAW api error {0}")]
    RawHTTP(#[from] http::Error),

    #[error("ParseIntError {0}")]
    ParseInt(#[from] std::num::ParseIntError),

    #[error("Tests failed: {0} failed out of {1} assertions")]
    TestFailure(usize, usize),

    #[error("KEY-OPENSSL-001 OpenSSL error {0}")]
    OpenSSL(#[from] openssl::error::ErrorStack),

    #[error("KEY-ALGO-001 Unsupported key algorithm: {0}")]
    UnsupportedKeyAlgorithm(String),

    #[error("{0}")]
    PasswordSpec(String),
    #[error("INIT-VERSION-001 Init version {0} not found in registry")]
    MissingInitVersion(String),

    #[error("CLEANUP-JOB-001 Delete job failed for {0}")]
    CleanupJobFailed(String),

    #[error("CLEANUP-JOB-002 Delete job still running for {0}")]
    CleanupInProgress(String),

    #[error("Error: {0}")]
    Other(String),

    #[error(transparent)]
    Core(#[from] vynil_core::Error),
}
impl Error {
    pub fn metric_label(&self) -> String {
        format!("{self:?}").to_lowercase()
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
pub type RhaiRes<T> = std::result::Result<T, Box<rhai::EvalAltResult>>;
pub fn rhai_err(e: Error) -> Box<rhai::EvalAltResult> {
    let msg = e.to_string();
    drop(e);
    msg.into()
}
pub fn rhai_err_str(e: String) -> Box<rhai::EvalAltResult> {
    e.into()
}
pub mod context;
pub mod handlebarshandler;
pub mod k8smock;
pub mod ttl_cache;
#[macro_use]
pub mod instance_macros;
pub mod instanceservice;
pub mod instancesystem;
pub mod instancetenant;
pub mod jukebox;
pub mod jukebox_file;
pub mod rhaihandler;
mod tools;
pub mod vynilpackage;
pub mod yamlhandler;

// ── Re-exports from vynil-core (preserve old import paths) ──────────────────

pub use context::get_client_name;
pub use vynil_core::{
    Semver, chrono as chronohandler, glob as globhandler, hashes as hasheshandlers, http as httphandler,
    http_mock as httpmock, k8s as k8sgeneric, k8s as k8sraw, k8s as k8sworkload, key as ed25519handler,
    oci as ocihandler, password as passwordhandler, register_k8s_generic, register_k8s_object,
    register_k8s_raw, s3 as s3handler, semver as semverhandler, set_client_name, shell as shellhandler,
};

/// Children describe a k8s object
#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Children {
    /// apiVersion of k8s object
    pub api_version: Option<String>,
    /// kind of k8s object
    pub kind: String,
    /// Name of the object
    pub name: String,
    /// Namespace is only used for namespaced object
    pub namespace: Option<String>,
}

/// `GlobalPublished` describe a published service open to use
#[derive(Serialize, Deserialize, Eq, PartialEq, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GlobalPublished {
    /// FQDN of the service
    pub fqdn: String,
    /// Port of the service
    pub port: u32,
}

/// Published describe a published service
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Published {
    /// key of the service
    pub key: String,
    /// Tenant using this definition
    pub tenant: Option<String>,
    /// service as fqdn+port
    pub service: Option<GlobalPublished>,
    /// Definition of the service stored in a children object
    pub definition: Option<Children>,
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_AGENT_IMAGE;

    #[test]
    fn default_agent_image_follows_workspace_version() {
        assert_eq!(
            DEFAULT_AGENT_IMAGE,
            concat!("ghcr.io/vynil-org/vynil-agent:", env!("CARGO_PKG_VERSION"))
        );
    }
}
