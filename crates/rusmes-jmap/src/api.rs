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

use crate::session::Session;
use crate::types::{JmapError, JmapErrorType, JmapRequest, JmapResponse};
use axum::{
    extract::Json,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose, Engine as _};

/// JMAP server
pub struct JmapServer;

impl JmapServer {
    /// Create JMAP routes
    pub fn routes() -> Router {
        Router::new()
            .route("/.well-known/jmap", get(session_endpoint))
            .route("/jmap", post(api_endpoint))
    }
}

/// Session discovery endpoint (RFC 8620 Section 2)
/// Returns a Session object describing the server's capabilities,
/// accounts, and API endpoints.
async fn session_endpoint(headers: HeaderMap) -> Json<Session> {
    // In a real implementation:
    // 1. Extract authentication token from Authorization header
    // 2. Validate the token and get the authenticated user
    // 3. Query the database for user's accounts
    // 4. Build session object with actual user data

    // For now, return a basic session for demonstration
    let username =
        extract_username_from_headers(&headers).unwrap_or_else(|| "user@example.com".to_string());

    let account_id = format!("account-{}", username.replace('@', "-"));
    let base_url = "https://jmap.example.com".to_string();

    let session = Session::new(username, account_id, base_url);

    Json(session)
}

/// Extract and validate username from Authorization header
///
/// NOTE: This is a simplified implementation for development.
/// In production, this should integrate with rusmes-auth::AuthBackend
/// to properly validate credentials against the configured backend
/// (file, LDAP, SQL, OAuth2, etc.)
fn extract_username_from_headers(headers: &HeaderMap) -> Option<String> {
    // Check for Basic auth
    if let Some(auth) = headers.get("authorization") {
        if let Ok(auth_str) = auth.to_str() {
            if let Some(encoded) = auth_str.strip_prefix("Basic ") {
                // Basic auth format: "Basic base64(username:password)"
                if let Ok(decoded) = general_purpose::STANDARD.decode(encoded) {
                    if let Ok(credentials) = String::from_utf8(decoded) {
                        let parts: Vec<&str> = credentials.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            let username = parts[0];
                            let password = parts[1];

                            // DEVELOPMENT ONLY: Simple validation
                            // In production, replace this with:
                            //   auth_backend.authenticate(username, password).await
                            if validate_basic_auth(username, password) {
                                return Some(username.to_string());
                            } else {
                                tracing::warn!(
                                    "Invalid Basic auth credentials for user: {}",
                                    username
                                );
                                return None;
                            }
                        }
                    }
                }
            } else if auth_str.starts_with("Bearer ") {
                // Bearer token format: "Bearer <token>"
                let token = auth_str.strip_prefix("Bearer ").unwrap_or("");

                // DEVELOPMENT ONLY: Simple token validation
                // In production, replace this with proper JWT validation:
                //   - Verify signature with public key
                //   - Check expiration (exp claim)
                //   - Validate issuer (iss claim)
                //   - Extract username from subject (sub claim)
                if let Some(username) = validate_bearer_token(token) {
                    return Some(username);
                } else {
                    tracing::warn!("Invalid Bearer token");
                    return None;
                }
            }
        }
    }

    None
}

/// Validate Basic authentication credentials
///
/// DEVELOPMENT ONLY: Returns true for any non-empty credentials.
/// In production, integrate with AuthBackend:
///   auth_backend.authenticate(username, password).await
fn validate_basic_auth(username: &str, password: &str) -> bool {
    // Allow any non-empty credentials for development
    // Real implementation would check against AuthBackend
    !username.is_empty() && !password.is_empty()
}

/// Validate Bearer token and extract username
///
/// DEVELOPMENT ONLY: Accepts any token and returns dummy username.
/// In production, implement proper JWT validation:
///   - Decode JWT and verify signature
///   - Check expiration, issuer, audience
///   - Extract username from 'sub' or custom claim
fn validate_bearer_token(_token: &str) -> Option<String> {
    // In production, use jsonwebtoken crate:
    //   let token_data = decode::<Claims>(token, &key, &validation)?;
    //   Some(token_data.claims.sub)

    // For now, return dummy username for development
    Some("user@example.com".to_string())
}

/// Main JMAP API endpoint
async fn api_endpoint(Json(request): Json<JmapRequest>) -> Response {
    tracing::debug!(
        "API_ENDPOINT: Received JMAP request with {} method calls",
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

    // Process each method call
    for method_call in request.method_calls {
        let call_id = method_call.2.clone();

        match crate::methods::dispatch_method(method_call, &request.using).await {
            Ok(method_response) => {
                response.method_responses.push(method_response);
            }
            Err(e) => {
                tracing::error!("JMAP method error: {}", e);
                // Return error response
                response
                    .method_responses
                    .push(crate::types::JmapMethodResponse(
                        "error".to_string(),
                        serde_json::to_value(
                            JmapError::new(JmapErrorType::ServerFail).with_detail(e.to_string()),
                        )
                        .unwrap_or_default(),
                        call_id,
                    ));
            }
        }
    }

    (StatusCode::OK, Json(response)).into_response()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jmap_server_routes() {
        let _router = JmapServer::routes();
        // Router created successfully
    }
}
