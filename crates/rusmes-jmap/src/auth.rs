//! JMAP authentication middleware.
//!
//! Replaces the legacy "DEVELOPMENT ONLY" header parser that lived in
//! [`crate::api`]. Every JMAP request now flows through [`require_auth`],
//! which:
//!
//! 1. Extracts credentials from the `Authorization` header (Basic or Bearer).
//! 2. Authenticates them against an [`AuthBackend`] (Bearer tokens are
//!    dispatched to [`AuthBackend::verify_bearer_token`]; backends without
//!    OAuth2 support return the default implementation which rejects every
//!    token, while the OAuth2 backend performs real JWT introspection).
//! 3. Attaches the resulting [`Principal`] to the request extensions so
//!    downstream handlers can enforce account ownership via
//!    [`Principal::owns_account`].
//!
//! Method handlers obtain the principal via Axum's
//! [`axum::Extension<Principal>`] extractor or, in `dispatch_method`, as a
//! `&Principal` argument plumbed in from the API entry point.

use crate::types::{derive_account_id, JmapError, JmapErrorType, Principal};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose, Engine as _};
use rusmes_auth::AuthBackend;
use rusmes_proto::Username;
use std::sync::Arc;

/// Shared authenticator handle wrapped behind the trait object so the JMAP
/// crate can be wired with any backend (`file`, `sql`, `ldap`, `oauth2`,
/// …) at server bootstrap time.
pub type SharedAuth = Arc<dyn AuthBackend>;

/// Credential bundle parsed from the HTTP `Authorization` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credentials {
    /// HTTP Basic — the username/password pair extracted from
    /// `Authorization: Basic base64(user:pass)`.
    Basic { username: String, password: String },
    /// HTTP Bearer — opaque or signed token from
    /// `Authorization: Bearer <token>`.
    Bearer { token: String },
}

/// Parse credentials from request headers.
///
/// Returns [`None`] when the `Authorization` header is missing, malformed, or
/// uses an unsupported scheme; callers must treat that as "unauthenticated"
/// and answer 401.
pub fn extract_credentials(headers: &HeaderMap) -> Option<Credentials> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let trimmed = value.trim();

    if let Some(rest) = strip_scheme(trimmed, "Basic") {
        let decoded_bytes = general_purpose::STANDARD.decode(rest).ok()?;
        let decoded = String::from_utf8(decoded_bytes).ok()?;
        let mut parts = decoded.splitn(2, ':');
        let username = parts.next()?.to_string();
        let password = parts.next()?.to_string();
        if username.is_empty() {
            return None;
        }
        return Some(Credentials::Basic { username, password });
    }

    if let Some(rest) = strip_scheme(trimmed, "Bearer") {
        let token = rest.trim().to_string();
        if token.is_empty() {
            return None;
        }
        return Some(Credentials::Bearer { token });
    }

    None
}

fn strip_scheme<'a>(header_value: &'a str, scheme: &str) -> Option<&'a str> {
    let scheme_len = scheme.len();
    if header_value.len() <= scheme_len {
        return None;
    }
    let (prefix, rest) = header_value.split_at(scheme_len);
    if !prefix.eq_ignore_ascii_case(scheme) {
        return None;
    }
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    Some(rest)
}

/// Authenticate credentials against the backend and produce a [`Principal`].
///
/// Bearer tokens are dispatched to [`AuthBackend::verify_bearer_token`].
/// Backends that do not support OAuth2/Bearer (file, SQL, LDAP, …) return the
/// default implementation which rejects every token, preserving the previous
/// unconditional-reject behaviour for those backends. The OAuth2 backend
/// overrides the method with real JWT introspection.
pub async fn authenticate(
    auth: &dyn AuthBackend,
    creds: &Credentials,
) -> Result<Principal, AuthError> {
    match creds {
        Credentials::Basic { username, password } => {
            let user = Username::new(username.clone()).map_err(|_| AuthError::Unauthorized)?;
            let ok = auth
                .authenticate(&user, password)
                .await
                .map_err(|err| AuthError::Backend(err.to_string()))?;
            if !ok {
                return Err(AuthError::Unauthorized);
            }
            Ok(Principal {
                username: username.clone(),
                account_id: derive_account_id(username),
                scopes: Vec::new(),
            })
        }
        Credentials::Bearer { token } => {
            let username = auth
                .verify_bearer_token(token)
                .await
                .map_err(|_| AuthError::Unauthorized)?;
            let username_str = username.to_string();
            Ok(Principal {
                account_id: derive_account_id(&username_str),
                username: username_str,
                scopes: Vec::new(),
            })
        }
    }
}

/// Authentication failures that surface as HTTP 401.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// No `Authorization` header, or credentials rejected by the backend.
    Unauthorized,
    /// The backend itself errored (network, file I/O, …) — surfaced to the
    /// caller as 401 to avoid leaking implementation details, and logged.
    Backend(String),
}

impl AuthError {
    fn into_response_body(self) -> Response {
        let detail = match self {
            AuthError::Unauthorized => "Authentication required".to_string(),
            AuthError::Backend(err) => {
                tracing::warn!("JMAP auth backend error: {}", err);
                "Authentication backend error".to_string()
            }
        };
        let body = JmapError::new(JmapErrorType::ServerFail)
            .with_status(401)
            .with_detail(detail);
        let mut resp = (StatusCode::UNAUTHORIZED, Json(body)).into_response();
        // RFC 7235 §4.1: indicate the supported schemes on a 401.
        if let Ok(value) = header::HeaderValue::from_str("Basic realm=\"jmap\"") {
            resp.headers_mut().insert(header::WWW_AUTHENTICATE, value);
        }
        resp
    }
}

/// Axum middleware that enforces authentication on every JMAP route it
/// guards. Successful authentication attaches a [`Principal`] to the request
/// extensions; failure short-circuits the chain with a 401.
pub async fn require_auth(
    State(auth): State<SharedAuth>,
    mut request: Request,
    next: Next,
) -> Response {
    let creds = match extract_credentials(request.headers()) {
        Some(c) => c,
        None => return AuthError::Unauthorized.into_response_body(),
    };
    let principal = match authenticate(auth.as_ref(), &creds).await {
        Ok(p) => p,
        Err(err) => return err.into_response_body(),
    };
    request.extensions_mut().insert(principal);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::http::HeaderValue;

    /// Test backend that accepts "alice"/"hunter2" and rejects everything else.
    struct TestBackend;

    #[async_trait]
    impl AuthBackend for TestBackend {
        async fn authenticate(&self, username: &Username, password: &str) -> anyhow::Result<bool> {
            Ok(username.as_str() == "alice" && password == "hunter2")
        }
        async fn verify_identity(&self, _username: &Username) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
            Ok(vec![])
        }
        async fn create_user(&self, _u: &Username, _p: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_user(&self, _u: &Username) -> anyhow::Result<()> {
            Ok(())
        }
        async fn change_password(&self, _u: &Username, _p: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn header_with_auth(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(value) {
            headers.insert(header::AUTHORIZATION, v);
        }
        headers
    }

    #[test]
    fn test_extract_basic_ok() {
        // base64("alice:hunter2") = "YWxpY2U6aHVudGVyMg=="
        let headers = header_with_auth("Basic YWxpY2U6aHVudGVyMg==");
        let creds = extract_credentials(&headers).expect("creds parse");
        assert_eq!(
            creds,
            Credentials::Basic {
                username: "alice".to_string(),
                password: "hunter2".to_string()
            }
        );
    }

    #[test]
    fn test_extract_basic_case_insensitive_scheme() {
        let headers = header_with_auth("basic YWxpY2U6aHVudGVyMg==");
        assert!(extract_credentials(&headers).is_some());
    }

    #[test]
    fn test_extract_bearer_ok() {
        let headers = header_with_auth("Bearer abc.def.ghi");
        let creds = extract_credentials(&headers).expect("creds parse");
        assert_eq!(
            creds,
            Credentials::Bearer {
                token: "abc.def.ghi".to_string()
            }
        );
    }

    #[test]
    fn test_extract_no_header() {
        let headers = HeaderMap::new();
        assert!(extract_credentials(&headers).is_none());
    }

    #[test]
    fn test_extract_unknown_scheme() {
        let headers = header_with_auth("Digest something");
        assert!(extract_credentials(&headers).is_none());
    }

    #[test]
    fn test_extract_basic_empty_username_rejected() {
        // base64(":pwd") = "OnB3ZA=="
        let headers = header_with_auth("Basic OnB3ZA==");
        assert!(extract_credentials(&headers).is_none());
    }

    #[test]
    fn test_extract_basic_no_colon_rejected() {
        // base64("alicehunter2") = "YWxpY2VodW50ZXIy"
        let headers = header_with_auth("Basic YWxpY2VodW50ZXIy");
        assert!(extract_credentials(&headers).is_none());
    }

    #[tokio::test]
    async fn test_authenticate_basic_ok() {
        let backend = TestBackend;
        let creds = Credentials::Basic {
            username: "alice".to_string(),
            password: "hunter2".to_string(),
        };
        let principal = authenticate(&backend, &creds).await.expect("auth ok");
        assert_eq!(principal.username, "alice");
        assert_eq!(principal.account_id, "account-alice");
    }

    #[tokio::test]
    async fn test_authenticate_basic_bad_password() {
        let backend = TestBackend;
        let creds = Credentials::Basic {
            username: "alice".to_string(),
            password: "wrong".to_string(),
        };
        let err = authenticate(&backend, &creds)
            .await
            .expect_err("should fail");
        assert_eq!(err, AuthError::Unauthorized);
    }

    #[tokio::test]
    async fn test_authenticate_bearer_backend_without_override_rejected() {
        // TestBackend does not override verify_bearer_token, so it falls
        // through to the default implementation which rejects every token.
        let backend = TestBackend;
        let creds = Credentials::Bearer {
            token: "anything".to_string(),
        };
        let err = authenticate(&backend, &creds)
            .await
            .expect_err("bearer 401");
        assert_eq!(err, AuthError::Unauthorized);
    }

    #[tokio::test]
    async fn test_authenticate_basic_with_email_username() {
        // Username with @ sign — valid email-style usernames are common.
        let backend = TestBackend;
        let creds = Credentials::Basic {
            username: "bob@example.com".to_string(),
            password: "hunter2".to_string(),
        };
        // Backend rejects (only "alice" is valid), so we get Unauthorized.
        let err = authenticate(&backend, &creds).await.expect_err("rejected");
        assert_eq!(err, AuthError::Unauthorized);
    }
}
