//! JMAP API server
//!
//! This module implements the JMAP (JSON Meta Application Protocol) API server
//! as defined in RFC 8620. It provides comprehensive request validation including:
//!
//! - Request structure validation (using, methodCalls)
//! - Capability validation (ensuring declared capabilities are supported)
//! - Method call validation (structure, limits, arguments)
//! - Error responses per RFC 8620 Section 3.6:
//!   - `unknownCapability`: Capability not recognized
//!   - `notRequest`: Request doesn't match JMAP structure
//!   - `limit`: Server limit exceeded (e.g., maxCallsInRequest)
//!   - `unknownMethod`: Method not recognized
//!   - `invalidArguments`: Invalid method arguments
//!   - Other error types for account/server issues
//!
//! ## Authentication
//!
//! Real authentication is handled by [`crate::auth::require_auth`] which
//! attaches a [`Principal`] to the request extensions. Construct the router
//! via [`JmapServer::routes_with_auth`] in production. The legacy
//! [`JmapServer::routes`] returns a router that rejects every request with
//! 401 — the previous implementation that fabricated a hardcoded principal
//! was a development-only fallback and has been removed.
//!
//! For production deployments that include blob storage and EventSource push,
//! use [`JmapServer::routes_with_auth_and_state`] which additionally mounts:
//! - `POST /upload/:account_id` — blob upload (RFC 8620 §6.2)
//! - `GET /download/:account_id/:blob_id/:name` — blob download (RFC 8620 §6.2)
//! - `GET /eventsource` — Server-Sent Events push channel (RFC 8620 §7.3)

use crate::auth::{require_auth, SharedAuth};
use crate::back_reference;
use crate::blob::{self, BlobStorage};
use crate::eventsource::{self, EventSourceManager};
use crate::session::Session;
use crate::types::{
    derive_account_id, JmapError, JmapErrorType, JmapMethodCall, JmapRequest, JmapResponse,
    Principal,
};
use axum::{
    extract::{Extension, Json, Request},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};

/// JMAP server.
///
/// Construct routes with [`Self::routes_with_auth`] to wire a real
/// authentication backend. The bare [`Self::routes`] constructor returns a
/// router that 401s every request and exists primarily for tests that only
/// exercise the routing surface.
pub struct JmapServer;

impl JmapServer {
    /// Build the JMAP HTTP routes with a real [`AuthBackend`](rusmes_auth::AuthBackend).
    ///
    /// Every route is wrapped by the authentication middleware so handlers
    /// receive a guaranteed-present [`Principal`] in their extensions, and by the
    /// connection-tracking middleware that maintains
    /// `rusmes_active_connections{protocol="jmap"}` plus the TLS counter.
    ///
    /// This mounts only the core JMAP API endpoints (session + method dispatch).
    /// For blob and EventSource routes, use [`Self::routes_with_auth_and_state`].
    pub fn routes_with_auth(auth: SharedAuth) -> Router {
        Router::new()
            .route("/.well-known/jmap", get(session_endpoint))
            .route("/jmap", post(api_endpoint))
            .layer(middleware::from_fn_with_state(auth.clone(), require_auth))
            .layer(middleware::from_fn(metrics_middleware))
            .with_state(auth)
    }

    /// Build the full JMAP HTTP routes including blob storage and EventSource push.
    ///
    /// In addition to the core JMAP routes from [`Self::routes_with_auth`], this
    /// mounts the following endpoints behind the same `require_auth` middleware:
    ///
    /// - `POST /upload/:account_id` — blob upload (RFC 8620 §6.2)
    /// - `GET /download/:account_id/:blob_id/:name` — blob download (RFC 8620 §6.2)
    /// - `GET /eventsource` — Server-Sent Events push channel (RFC 8620 §7.3)
    ///
    /// All routes enforce authentication via `require_auth` before dispatching
    /// to their respective handlers.
    pub fn routes_with_auth_and_state(
        auth: SharedAuth,
        blob_storage: BlobStorage,
        event_manager: EventSourceManager,
    ) -> Router {
        let blob_r = blob::blob_routes().with_state(blob_storage);
        let es_r = eventsource::eventsource_routes().with_state(event_manager);
        Router::new()
            .route("/.well-known/jmap", get(session_endpoint))
            .route("/jmap", post(api_endpoint))
            .merge(blob_r)
            .merge(es_r)
            .layer(middleware::from_fn_with_state(auth.clone(), require_auth))
            .layer(middleware::from_fn(metrics_middleware))
            .with_state(auth)
    }

    /// Build the JMAP HTTP routes WITHOUT an auth backend.
    ///
    /// Every request is rejected with 401. This exists so the public API
    /// remains call-compatible during the transition window — callers that
    /// previously relied on the unauthenticated dev path should migrate to
    /// [`Self::routes_with_auth`].
    pub fn routes() -> Router {
        Router::new()
            .route("/.well-known/jmap", get(reject_unauthenticated))
            .route("/jmap", post(reject_unauthenticated))
            .layer(middleware::from_fn(metrics_middleware))
    }
}

/// Axum middleware that records each JMAP HTTP request as a "session" in the metrics.
///
/// JMAP is request/response over HTTP — there is no long-lived TCP session in the same
/// sense as SMTP/IMAP, so we treat every request as one session for the purposes of the
/// active-connections gauge and TLS counter. The active gauge therefore tracks
/// concurrently-in-flight requests, which is the operationally useful number.
///
/// TLS labelling: at this layer we cannot tell whether the request arrived over TLS
/// (the listener handles termination upstream). We unconditionally record `no` — when a
/// future change wraps the JMAP listener with rustls-axum, this should be flipped to
/// inspect the request's `extensions()` for a `ConnectInfo<TlsConnectionInfo>` marker.
async fn metrics_middleware(request: Request, next: Next) -> Response {
    let metrics = rusmes_metrics::global_metrics();
    let _conn_guard = metrics.connection_guard("jmap");
    metrics.inc_tls_session(rusmes_metrics::tls_label::NO);
    next.run(request).await
}

/// Hard-fail handler used by the auth-less constructor — every JMAP route
/// answers 401 without an `AuthBackend`.
async fn reject_unauthenticated() -> Response {
    let body = JmapError::new(JmapErrorType::ServerFail)
        .with_status(401)
        .with_detail(
            "JMAP server constructed without an authentication backend; \
             use JmapServer::routes_with_auth in production",
        );
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

/// Session discovery endpoint (RFC 8620 Section 2)
///
/// Returns a Session object describing the server's capabilities, accounts,
/// and API endpoints for the authenticated [`Principal`].
async fn session_endpoint(Extension(principal): Extension<Principal>) -> Json<Session> {
    let base_url = "https://jmap.example.com".to_string();
    let session = Session::new(
        principal.username.clone(),
        principal.account_id.clone(),
        base_url,
    );
    Json(session)
}

/// Main JMAP API endpoint
async fn api_endpoint(
    Extension(principal): Extension<Principal>,
    Json(request): Json<JmapRequest>,
) -> Response {
    tracing::debug!(
        "API_ENDPOINT: Received JMAP request from {} with {} method calls",
        principal.username,
        request.method_calls.len()
    );
    // Validate the request structure (RFC 8620 Section 3.3)
    if let Some(error_response) = validate_request(&request) {
        tracing::debug!("API_ENDPOINT: Request validation failed");
        return error_response;
    }
    tracing::debug!("API_ENDPOINT: Request validated successfully");

    let mut response = JmapResponse {
        method_responses: Vec::new(),
        session_state: Some("state1".to_string()),
        created_ids: request.created_ids.clone(),
    };

    // Track completed calls so that later method calls in the same batch can
    // reference their results via RFC 8620 §3.7 ResultReferences.
    // Each entry is (call_id, method_name, response_body).
    let mut completed: Vec<(String, String, serde_json::Value)> = Vec::new();

    // Process each method call
    for method_call in request.method_calls {
        let call_id = method_call.2.clone();
        let method_name = method_call.0.clone();

        // RFC 8620 §3.7 — resolve any ResultReferences in the call's arguments
        // before dispatching.
        let method_call = match resolve_back_refs_in_call(method_call, &completed) {
            Ok(resolved) => resolved,
            Err(e) => {
                // Resolution failure → invalidArguments error for this call.
                tracing::debug!("Back-reference resolution failed for {}: {}", call_id, e);
                let err_value = serde_json::to_value(
                    JmapError::new(JmapErrorType::InvalidArguments).with_detail(e.to_string()),
                )
                .unwrap_or(serde_json::Value::Null);
                response
                    .method_responses
                    .push(crate::types::JmapMethodResponse(
                        "error".to_string(),
                        err_value,
                        call_id,
                    ));
                // Continue to the next call — RFC 8620 §3.7 says failure
                // yields an error for that call and execution continues.
                continue;
            }
        };

        match crate::methods::dispatch_method(method_call, &request.using, &principal).await {
            Ok(method_response) => {
                // Record this completed call so subsequent calls can reference it.
                completed.push((call_id, method_name, method_response.1.clone()));
                response.method_responses.push(method_response);
            }
            Err(e) => {
                tracing::error!("JMAP method error: {}", e);
                let err_value = serde_json::to_value(
                    JmapError::new(JmapErrorType::ServerFail).with_detail(e.to_string()),
                )
                .unwrap_or(serde_json::Value::Null);
                // Record the error response body too, so that a reference to this
                // call's result correctly returns ResultWasError.
                completed.push((call_id.clone(), method_name, err_value.clone()));
                response
                    .method_responses
                    .push(crate::types::JmapMethodResponse(
                        "error".to_string(),
                        err_value,
                        call_id,
                    ));
            }
        }
    }

    (StatusCode::OK, Json(response)).into_response()
}

/// Apply RFC 8620 §3.7 back-reference resolution to a single method call.
///
/// Mutates the call's argument object in place, replacing every `#key`
/// ResultReference with the value it points to in `completed`.  Returns the
/// (possibly mutated) call on success, or a [`back_reference::BackRefError`]
/// on the first resolution failure.
fn resolve_back_refs_in_call(
    mut call: JmapMethodCall,
    completed: &[(String, String, serde_json::Value)],
) -> Result<JmapMethodCall, back_reference::BackRefError> {
    if let Some(obj) = call.1.as_object_mut() {
        back_reference::resolve_back_references(obj, completed)?;
    }
    Ok(call)
}

/// Validate JMAP request structure and capabilities (RFC 8620 Section 3.3)
fn validate_request(request: &JmapRequest) -> Option<Response> {
    // Validate "using" capabilities
    if request.using.is_empty() {
        let error = JmapError::new(JmapErrorType::UnknownCapability)
            .with_status(400)
            .with_detail("The 'using' property must contain at least one capability");
        return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
    }

    // Check for supported capabilities
    let supported_capabilities = get_supported_capabilities();
    for capability in &request.using {
        if !supported_capabilities.contains(&capability.as_str()) {
            let error = JmapError::new(JmapErrorType::UnknownCapability)
                .with_status(400)
                .with_detail(format!("Unsupported capability: {}", capability));
            return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
        }
    }

    // Validate methodCalls structure
    if request.method_calls.is_empty() {
        let error = JmapError::new(JmapErrorType::NotRequest)
            .with_status(400)
            .with_detail("The 'methodCalls' property must contain at least one method call");
        return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
    }

    // Check for method call limit (RFC 8620 Section 3.4)
    const MAX_CALLS_IN_REQUEST: usize = 16;
    if request.method_calls.len() > MAX_CALLS_IN_REQUEST {
        let error = JmapError::new(JmapErrorType::Limit)
            .with_status(400)
            .with_detail(format!(
                "Too many method calls. Maximum allowed: {}",
                MAX_CALLS_IN_REQUEST
            ))
            .with_limit("maxCallsInRequest");
        return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
    }

    // Validate each method call structure
    for (idx, method_call) in request.method_calls.iter().enumerate() {
        let method_name = &method_call.0;
        let call_id = &method_call.2;

        // Validate method name is not empty
        if method_name.is_empty() {
            let error = JmapError::new(JmapErrorType::NotRequest)
                .with_status(400)
                .with_detail(format!("Method call {} has empty method name", idx));
            return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
        }

        // Validate call ID is not empty
        if call_id.is_empty() {
            let error = JmapError::new(JmapErrorType::NotRequest)
                .with_status(400)
                .with_detail(format!("Method call {} has empty call ID", idx));
            return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
        }

        // Validate arguments is an object
        if !method_call.1.is_object() {
            let error = JmapError::new(JmapErrorType::InvalidArguments)
                .with_status(400)
                .with_detail(format!(
                    "Method call {} ('{}') has invalid arguments - must be an object",
                    idx, method_name
                ));
            return Some((StatusCode::BAD_REQUEST, Json(error)).into_response());
        }
    }

    None
}

/// Get the list of supported JMAP capabilities
fn get_supported_capabilities() -> Vec<&'static str> {
    vec![
        "urn:ietf:params:jmap:core",
        "urn:ietf:params:jmap:mail",
        "urn:ietf:params:jmap:submission",
        "urn:ietf:params:jmap:vacationresponse",
    ]
}

/// Helper exposed for callers that need the canonical username → account-id
/// mapping (kept here so external crates have a single entry point).
pub fn account_id_for(username: &str) -> String {
    derive_account_id(username)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::SharedAuth;
    use async_trait::async_trait;
    use rusmes_auth::AuthBackend;
    use rusmes_proto::Username;
    use std::sync::Arc;

    struct DenyAll;

    #[async_trait]
    impl AuthBackend for DenyAll {
        async fn authenticate(&self, _u: &Username, _p: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn verify_identity(&self, _u: &Username) -> anyhow::Result<bool> {
            Ok(false)
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

    #[test]
    fn test_jmap_server_routes() {
        let _router = JmapServer::routes();
        // Router created successfully
    }

    #[test]
    fn test_jmap_server_routes_with_auth() {
        let auth: SharedAuth = Arc::new(DenyAll);
        let _router = JmapServer::routes_with_auth(auth);
    }

    #[test]
    fn test_account_id_helper() {
        assert_eq!(
            account_id_for("alice@example.com"),
            "account-alice-example.com"
        );
    }
}
