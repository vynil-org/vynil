use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, header::HeaderMap, request::Parts},
    response::{IntoResponse, Response},
    routing::{Router, get},
};
use axum_server::tls_rustls::RustlsConfig;
use kube::Client;
use regex::Regex;
use std::{
    net::SocketAddr,
    sync::{Arc, LazyLock},
};

use crate::{
    auth::extract_identity,
    authz::check_item_access,
    collect::{
        children::get_children,
        clusterinfo::get_cluster_info,
        logs::{get_agent_log, get_child_logs, get_operator_log},
        packages::get_packages,
        popularity_contest::get_popularity_contest,
        state::get_instance_state,
        vynilconfig::get_vynil_config,
    },
    config::Config,
    discovery::discovery_handler,
    dto::ScrubStats,
    error::DiagError,
    state::AppState,
};

// Custom extractor for HeaderMap
#[derive(Debug)]
pub struct RequestHeaders(pub HeaderMap);

impl<S> FromRequestParts<S> for RequestHeaders
where
    S: Send + Sync + 'static,
{
    type Rejection = DiagError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(Self(parts.headers.clone())))
    }
}

pub const API_GROUP: &str = "admin.vynil.solidite.fr";
pub const API_VERSION: &str = "v1";

const API_PATH: &str = "/apis/admin.vynil.solidite.fr/v1";
const INSTANCE_PATH: &str = "/apis/admin.vynil.solidite.fr/v1/namespaces/{ns}/{kind}/{name}/{item}";

/// Valid kinds for the diagnostic API
const VALID_KINDS: [&str; 3] = ["tenantinstances", "serviceinstances", "systeminstances"];

/// Valid items for the diagnostic API
const VALID_ITEMS: [&str; 9] = [
    "clusterinfo",
    "vynilconfig",
    "packages",
    "popularitycontest",
    "state",
    "children",
    "agentlog",
    "childlogs",
    "operatorlog",
];

/// DNS-1123 regex for validating namespace and name
static DNS_1123_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9]([-a-z0-9.]*[a-z0-9])?$")
        .unwrap_or_else(|e| panic!("invalid static DNS-1123 pattern: {e}"))
});

/// Validate namespace and name format
fn validate_dns1123(name: &str) -> bool {
    DNS_1123_RE.is_match(name)
}

/// Create the main router with state
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz_handler))
        .route(API_PATH, get(discovery_handler))
        .route(INSTANCE_PATH, get(instance_handler))
        .with_state(state)
}

/// Run the server
///
/// # Errors
///
/// Returns [`DiagError::InternalError`] when the bind address is invalid or TLS material cannot
/// be loaded, and aborts startup when trusting front-proxy headers is requested without TLS.
pub async fn run_server(state: AppState, config: Config) -> Result<(), DiagError> {
    let router = create_router(state.clone());

    let addr: SocketAddr = config
        .bind
        .parse()
        .map_err(|e| DiagError::InternalError(format!("Invalid bind address: {e}")))?;

    // SECURITY guardrail: trusting front-proxy identity headers is only safe behind mandatory
    // mTLS verification of the apiserver client-cert. Refuse to start without TLS.
    if config.trust_request_header && config.insecure_no_tls {
        return Err(DiagError::InternalError(
            "DIAG-CONFIG: --trust-request-header requires TLS (mandatory mTLS). \
             Refusing to start in an impersonable configuration."
                .to_string(),
        ));
    }

    if config.insecure_no_tls {
        tracing::warn!(
            "Starting HTTP server (no TLS) on {} — Bearer/TokenReview auth only",
            addr
        );
        axum::serve(
            tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| DiagError::InternalError(format!("HTTP bind error: {e}")))?,
            router,
        )
        .await
        .map_err(|e| DiagError::InternalError(format!("HTTP server error: {e}")))?;
        return Ok(());
    }

    let (Some(tls_cert), Some(tls_key)) = (&config.tls_cert, &config.tls_key) else {
        return Err(DiagError::InternalError(
            "TLS certificates not provided for HTTPS server".to_string(),
        ));
    };

    let tls_config = if config.trust_request_header {
        let ca_pem = load_requestheader_ca(&state.client, &config).await?;
        tracing::info!("Starting HTTPS server with MANDATORY mTLS (front-proxy CA self-loaded)");
        build_mtls_config(tls_cert, tls_key, &ca_pem)?
    } else {
        tracing::info!("Starting HTTPS server (server-side TLS) on {}", addr);
        RustlsConfig::from_pem_file(tls_cert, tls_key)
            .await
            .map_err(|e| DiagError::InternalError(format!("TLS config error: {e}")))?
    };

    axum_server::bind_rustls(addr, tls_config)
        .serve(router.into_make_service())
        .await
        .map_err(|e| DiagError::InternalError(format!("HTTPS server error: {e}")))?;

    Ok(())
}

/// Load the front-proxy CA used to verify the apiserver's client cert (LEG 2 of the aggregation
/// mTLS). Default: self-load from the `extension-apiserver-authentication` `ConfigMap` in
/// `kube-system` (requires the `extension-apiserver-authentication-reader` `RoleBinding`). A file
/// path via `--requestheader-client-ca` overrides it (escape hatch / offline).
async fn load_requestheader_ca(client: &Client, config: &Config) -> Result<String, DiagError> {
    if let Some(path) = &config.requestheader_ca {
        return std::fs::read_to_string(path)
            .map_err(|e| DiagError::InternalError(format!("requestheader CA file: {e}")));
    }
    let api: kube::Api<k8s_openapi::api::core::v1::ConfigMap> =
        kube::Api::namespaced(client.clone(), "kube-system");
    let cm = api
        .get("extension-apiserver-authentication")
        .await
        .map_err(DiagError::KubeError)?;
    cm.data
        .and_then(|d| d.get("requestheader-client-ca-file").cloned())
        .ok_or_else(|| {
            DiagError::InternalError(
                "requestheader-client-ca-file absent de extension-apiserver-authentication".into(),
            )
        })
}

/// Build a rustls config that **requires and verifies** the client certificate against the
/// front-proxy CA (PEM). Reaching the application then proves the request came from the apiserver
/// front-proxy, which is what makes trusting `X-Remote-*` headers safe.
fn build_mtls_config(
    cert: &std::path::Path,
    key: &std::path::Path,
    ca_pem: &str,
) -> Result<RustlsConfig, DiagError> {
    use std::{fs::File, io::BufReader};

    let err = |m: String| DiagError::InternalError(format!("mTLS config: {m}"));
    let provider = Arc::new(rustls::crypto::ring::default_provider());

    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(
        File::open(cert).map_err(|e| err(e.to_string()))?,
    ))
    .collect::<Result<_, _>>()
    .map_err(|e| err(e.to_string()))?;

    let key = rustls_pemfile::private_key(&mut BufReader::new(
        File::open(key).map_err(|e| err(e.to_string()))?,
    ))
    .map_err(|e| err(e.to_string()))?
    .ok_or_else(|| err("no private key in key file".to_string()))?;

    let mut roots = rustls::RootCertStore::empty();
    for c in rustls_pemfile::certs(&mut ca_pem.as_bytes()) {
        roots
            .add(c.map_err(|e| err(e.to_string()))?)
            .map_err(|e| err(e.to_string()))?;
    }

    let verifier =
        rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
            .build()
            .map_err(|e| err(e.to_string()))?;

    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| err(e.to_string()))?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|e| err(e.to_string()))?;

    Ok(RustlsConfig::from_config(Arc::new(config)))
}

/// Health check handler - no authentication required
async fn healthz_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Instance endpoint handler
async fn instance_handler(
    State(state): State<AppState>,
    RequestHeaders(headers): RequestHeaders,
    Path((ns, kind, name, item)): Path<(String, String, String, String)>,
) -> Result<Response, DiagError> {
    // Validate namespace and name format
    if !validate_dns1123(&ns) || !validate_dns1123(&name) {
        return Err(DiagError::InvalidNameFormat);
    }

    // Validate kind
    if !VALID_KINDS.contains(&kind.as_str()) {
        return Err(DiagError::UnknownKind);
    }

    // Validate item
    if !VALID_ITEMS.contains(&item.as_str()) {
        return Err(DiagError::UnknownItem);
    }

    // Authenticate FIRST — never touch the cluster with the server's rights before we know who
    // is calling (otherwise an unauthenticated caller gets an instance-existence oracle).
    let identity = extract_identity(&state.client, &headers, state.config.trust_request_header).await?;

    // Authorize (SAR for instance-scoped items; packages-disabled check).
    check_item_access(
        &state.client,
        &identity,
        &kind,
        &ns,
        &name,
        &item,
        state.config.enable_packages,
    )
    .await?;

    // Existence check only for instance-scoped items (and only after authz passed).
    if crate::authz::is_instance_scoped_item(&item)
        && !check_instance_exists(&state.client, &kind, &ns, &name).await?
    {
        return Err(DiagError::InstanceNotFound);
    }

    // Handle different items
    match item.as_str() {
        "clusterinfo" => {
            // Generic tier - no instance-specific authorization required
            let cluster_info = get_cluster_info(&state.client, &state.config.vynil_namespace).await?;
            Ok((StatusCode::OK, Json(cluster_info)).into_response())
        }
        "vynilconfig" => {
            // Generic tier - no instance-specific authorization required
            let (vynil_config, scrub_stats) =
                get_vynil_config(&state.client, &state.config.vynil_namespace, &ns).await?;
            let mut response = yaml_response(vynil_config);
            add_scrub_header(response.headers_mut(), &scrub_stats);
            Ok(response)
        }
        "packages" => {
            // Generic tier - authorization already checked above
            let packages = get_packages(&state.client).await?;
            Ok((StatusCode::OK, Json(packages)).into_response())
        }
        "popularitycontest" => {
            // Generic tier - authorization already checked above
            let popularity = get_popularity_contest(&state.client).await?;
            Ok((StatusCode::OK, Json(popularity)).into_response())
        }
        "state" => {
            let (yaml, scrub_stats) =
                get_instance_state(&state.client, &kind, &ns, &name, &state.config.vynil_namespace).await?;

            let mut response = yaml_response(yaml);
            add_scrub_header(response.headers_mut(), &scrub_stats);
            Ok(response)
        }
        "children" => {
            let (children, scrub_stats) =
                get_children(&state.client, &kind, &ns, &name, &state.config.vynil_namespace).await?;

            let mut response = Json(children).into_response();
            add_scrub_header(response.headers_mut(), &scrub_stats);
            Ok(response)
        }
        "agentlog" => {
            let (logs, scrub_stats) =
                get_agent_log(&state.client, &kind, &ns, &name, &state.config.vynil_namespace).await?;

            let mut response = logs.into_response();
            add_scrub_header(response.headers_mut(), &scrub_stats);
            Ok(response)
        }
        "childlogs" => {
            let (logs, scrub_stats) = get_child_logs(
                &state.client,
                &kind,
                &ns,
                &name,
                state.config.log_since_hours,
                state.config.log_cap_bytes,
                &state.config.vynil_namespace,
            )
            .await?;

            let mut response = logs.into_response();
            add_scrub_header(response.headers_mut(), &scrub_stats);
            Ok(response)
        }
        "operatorlog" => {
            let (logs, scrub_stats) =
                get_operator_log(&state.client, &kind, &ns, &name, &state.config.vynil_namespace).await?;

            let mut response = logs.into_response();
            add_scrub_header(response.headers_mut(), &scrub_stats);
            Ok(response)
        }
        _ => Err(DiagError::UnknownItem),
    }
}

/// Check if an instance exists
async fn check_instance_exists(
    client: &Client,
    kind: &str,
    namespace: &str,
    name: &str,
) -> Result<bool, DiagError> {
    match kind {
        "tenantinstances" => {
            let api: kube::Api<common::instancetenant::TenantInstance> =
                kube::Api::namespaced(client.clone(), namespace);
            match api.get(name).await {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        "serviceinstances" => {
            let api: kube::Api<common::instanceservice::ServiceInstance> =
                kube::Api::namespaced(client.clone(), namespace);
            match api.get(name).await {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        "systeminstances" => {
            let api: kube::Api<common::instancesystem::SystemInstance> =
                kube::Api::namespaced(client.clone(), namespace);
            match api.get(name).await {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        _ => Ok(false),
    }
}

/// Build a response carrying a YAML body with the right Content-Type (a bare `String` would
/// default to `text/plain`, which mislabels the artifact for the CLI/maintainer).
fn yaml_response(body: String) -> Response {
    ([(axum::http::header::CONTENT_TYPE, "application/yaml")], body).into_response()
}

/// Add scrub statistics header to response
fn add_scrub_header(headers: &mut HeaderMap, scrub_stats: &ScrubStats) {
    let header_value = format!(
        "distinct={};occurrences={}",
        scrub_stats.distinct, scrub_stats.occurrences
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&header_value) {
        headers.insert("X-Diag-Redactions", value);
    }
}
