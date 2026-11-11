use axum::http::{HeaderMap, header::AUTHORIZATION};
use k8s_openapi::api::authentication::v1::TokenReview;
use kube::{Api, Client, api::PostParams};

use crate::error::DiagError;

/// Identity information extracted from the request.
#[derive(Debug, Clone)]
pub struct Identity {
    pub user: String,
    pub groups: Vec<String>,
}

/// Extract the caller identity.
///
/// Two paths, in order:
/// 1. **Front-proxy headers** (`X-Remote-User` / `X-Remote-Group`) — honoured **only** when
///    `trust_request_header` is true. That flag is wired (see `server::run_server`) to **mandatory
///    mTLS** verification of the apiserver client-cert against the requestheader CA, so reaching
///    this code already proves the request came from the aggregation layer. When the flag is off,
///    these headers are **completely ignored** (otherwise anyone could impersonate any user).
/// 2. **Bearer token** → `TokenReview` (direct in-cluster / test path).
///
/// # Errors
///
/// Returns [`DiagError::AuthenticationRequired`] when no usable identity is present in the
/// request headers.
pub async fn extract_identity(
    client: &Client,
    headers: &HeaderMap,
    trust_request_header: bool,
) -> Result<Identity, DiagError> {
    if trust_request_header && let Some(identity) = request_header_identity(headers) {
        return Ok(identity);
    }

    if let Some(auth_header) = headers.get(AUTHORIZATION) {
        let auth_value = auth_header
            .to_str()
            .map_err(|_| DiagError::AuthenticationRequired)?;
        if let Some(token) = auth_value.strip_prefix("Bearer ") {
            return validate_token(client, token).await;
        }
    }

    Err(DiagError::AuthenticationRequired)
}

/// Pure parse of front-proxy identity headers (no trust decision here).
#[must_use]
pub fn request_header_identity(headers: &HeaderMap) -> Option<Identity> {
    let user = headers.get("X-Remote-User")?.to_str().ok()?.to_string();
    if user.is_empty() {
        return None;
    }
    let groups = headers
        .get_all("X-Remote-Group")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .map(std::string::ToString::to_string)
        .collect();
    Some(Identity { user, groups })
}

/// Validate a bearer token via the Kubernetes `TokenReview` API.
///
/// # Errors
///
/// Returns [`DiagError::AuthenticationRequired`] when the token review call fails or the token
/// is not authenticated.
pub async fn validate_token(client: &Client, token: &str) -> Result<Identity, DiagError> {
    let api: Api<TokenReview> = Api::all(client.clone());
    let review = TokenReview {
        spec: k8s_openapi::api::authentication::v1::TokenReviewSpec {
            token: token.to_string(),
            ..Default::default()
        },
        ..Default::default()
    };

    let result = api.create(&PostParams::default(), &review).await.map_err(|e| {
        tracing::error!("TokenReview failed: {}", e);
        DiagError::AuthenticationRequired
    })?;

    let status = result.status.unwrap_or_default();
    if status.authenticated.unwrap_or(false) {
        let user_info = status.user.unwrap_or_default();
        Ok(Identity {
            user: user_info.username.unwrap_or_default(),
            groups: user_info.groups.unwrap_or_default(),
        })
    } else {
        tracing::warn!("Token authentication failed");
        Err(DiagError::AuthenticationRequired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use std::str::FromStr;

    fn headers_with_remote_user() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("X-Remote-User", HeaderValue::from_static("alice"));
        let name = HeaderName::from_str("X-Remote-Group").unwrap();
        headers.append(&name, HeaderValue::from_static("g1"));
        headers.append(&name, HeaderValue::from_static("g2"));
        headers
    }

    #[test]
    fn parses_request_header_identity() {
        let id = request_header_identity(&headers_with_remote_user()).unwrap();
        assert_eq!(id.user, "alice");
        assert_eq!(id.groups, vec!["g1", "g2"]);
    }

    #[test]
    fn no_identity_without_headers() {
        assert!(request_header_identity(&HeaderMap::new()).is_none());
    }

    #[test]
    fn empty_remote_user_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Remote-User", HeaderValue::from_static(""));
        assert!(request_header_identity(&headers).is_none());
    }
}
