//! IMAP AUTHENTICATE command implementation with SASL integration
//!
//! This module implements the AUTHENTICATE command as specified in RFC 3501 Section 6.2.2,
//! integrating the SASL framework from rusmes-auth.
//!
//! Supported SASL mechanisms:
//! - PLAIN (RFC 4616)
//! - LOGIN (obsolete but widely used)
//! - CRAM-MD5 (RFC 2195)
//! - SCRAM-SHA-256 (RFC 5802, RFC 7677)
//! - XOAUTH2 (RFC 7628)
//!
//! # Authentication Flow
//!
//! ## Basic Flow (PLAIN, single-step)
//! ```text
//! C: A001 AUTHENTICATE PLAIN
//! S: +
//! C: <base64-encoded credentials>
//! S: A001 OK AUTHENTICATE completed
//! ```
//!
//! ## Challenge-Response Flow (CRAM-MD5, SCRAM-SHA-256)
//! ```text
//! C: A001 AUTHENTICATE CRAM-MD5
//! S: + <base64-encoded challenge>
//! C: <base64-encoded response>
//! S: A001 OK AUTHENTICATE completed
//! ```
//!
//! ## Initial Response Optimization (RFC 4959)
//! ```text
//! C: A001 AUTHENTICATE PLAIN <base64-encoded credentials>
//! S: A001 OK AUTHENTICATE completed
//! ```

use crate::response::ImapResponse;
use crate::session::{ImapSession, ImapState};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusmes_auth::{sasl::SaslServer, AuthBackend};

/// Authentication state for multi-step SASL authentication
#[derive(Debug)]
pub enum AuthenticateState {
    /// Initial state - mechanism selected, waiting for client data
    Initial,
    /// Challenge sent, waiting for response
    Challenge,
    /// Completed (success or failure)
    Completed,
}

/// AUTHENTICATE command context for tracking multi-step authentication
pub struct AuthenticateContext {
    /// SASL mechanism instance
    mechanism: Box<dyn rusmes_auth::sasl::SaslMechanism>,
    /// Current authentication state
    #[allow(dead_code)]
    state: AuthenticateState,
    /// Tag from original AUTHENTICATE command
    tag: String,
}

impl AuthenticateContext {
    /// Create a new authentication context
    pub fn new(mechanism: Box<dyn rusmes_auth::sasl::SaslMechanism>, tag: String) -> Self {
        Self {
            mechanism,
            state: AuthenticateState::Initial,
            tag,
        }
    }

    /// Get the tag
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Get the mechanism name
    pub fn mechanism_name(&self) -> &str {
        self.mechanism.name()
    }
}

/// Handle AUTHENTICATE command
///
/// # Arguments
/// * `session` - Current IMAP session
/// * `tag` - Command tag
/// * `mechanism_name` - SASL mechanism name (e.g., "PLAIN", "CRAM-MD5")
/// * `initial_response` - Optional initial response (RFC 4959 SASL-IR)
/// * `sasl_server` - SASL server for mechanism creation
/// * `auth_backend` - Authentication backend
///
/// # Returns
/// Returns an IMAP response and optionally an authentication context for multi-step auth
pub async fn handle_authenticate(
    session: &mut ImapSession,
    tag: &str,
    mechanism_name: &str,
    initial_response: Option<&str>,
    sasl_server: &SaslServer,
    auth_backend: &dyn AuthBackend,
) -> anyhow::Result<(ImapResponse, Option<AuthenticateContext>)> {
    // Must be in NotAuthenticated state
    if !matches!(session.state(), ImapState::NotAuthenticated) {
        return Ok((ImapResponse::bad(tag, "Already authenticated"), None));
    }

    // Check if mechanism is supported
    if !sasl_server.is_mechanism_enabled(mechanism_name) {
        return Ok((
            ImapResponse::no(
                tag,
                format!(
                    "[AUTHENTICATIONFAILED] Mechanism {} not supported",
                    mechanism_name
                ),
            ),
            None,
        ));
    }

    // Create mechanism instance
    let mut mechanism = match sasl_server.create_mechanism(mechanism_name) {
        Ok(m) => m,
        Err(e) => {
            return Ok((
                ImapResponse::no(tag, format!("[AUTHENTICATIONFAILED] {}", e)),
                None,
            ));
        }
    };

    // Handle initial response if provided (SASL-IR, RFC 4959)
    if let Some(initial_resp) = initial_response {
        // Decode the base64-encoded initial response
        let decoded = match BASE64.decode(initial_resp.trim()) {
            Ok(d) => d,
            Err(e) => {
                return Ok((
                    ImapResponse::bad(tag, format!("Invalid Base64 in initial response: {}", e)),
                    None,
                ));
            }
        };

        let decoded_str = std::str::from_utf8(&decoded).unwrap_or("");

        return handle_authenticate_step(session, tag, mechanism, decoded_str, auth_backend).await;
    }

    // No initial response - send continuation or challenge based on mechanism
    let auth_backend_ref: &dyn AuthBackend = auth_backend;

    match mechanism.step(b"", auth_backend_ref).await {
        Ok(rusmes_auth::sasl::SaslStep::Challenge { data }) => {
            // Mechanism needs to send a challenge
            let encoded = BASE64.encode(&data);
            let ctx = AuthenticateContext {
                mechanism,
                state: AuthenticateState::Challenge,
                tag: tag.to_string(),
            };
            Ok((ImapResponse::new(None, "+", encoded), Some(ctx)))
        }
        Ok(rusmes_auth::sasl::SaslStep::Continue) => {
            // Mechanism needs more data from client (no challenge)
            let ctx = AuthenticateContext {
                mechanism,
                state: AuthenticateState::Initial,
                tag: tag.to_string(),
            };
            Ok((ImapResponse::new(None, "+", ""), Some(ctx)))
        }
        Ok(rusmes_auth::sasl::SaslStep::Done { success, username }) => {
            // Authentication completed in first step (shouldn't happen without initial response)
            if success && username.is_some() {
                session.state = ImapState::Authenticated;
                session.username = username;
                Ok((ImapResponse::ok(tag, "AUTHENTICATE completed"), None))
            } else {
                Ok((
                    ImapResponse::no(tag, "[AUTHENTICATIONFAILED] Authentication failed"),
                    None,
                ))
            }
        }
        Err(e) => Ok((
            ImapResponse::no(tag, format!("[AUTHENTICATIONFAILED] {}", e)),
            None,
        )),
    }
}

/// Continue multi-step authentication with client response
///
/// # Arguments
/// * `session` - Current IMAP session
/// * `ctx` - Authentication context from previous step
/// * `client_data` - Base64-encoded client response
/// * `auth_backend` - Authentication backend
///
/// # Returns
/// Returns an IMAP response and optionally an updated authentication context
pub async fn handle_authenticate_continue(
    session: &mut ImapSession,
    ctx: AuthenticateContext,
    client_data: &str,
    auth_backend: &dyn AuthBackend,
) -> anyhow::Result<(ImapResponse, Option<AuthenticateContext>)> {
    // Check for cancellation (client sends "*")
    if client_data.trim() == "*" {
        return Ok((ImapResponse::bad(&ctx.tag, "AUTHENTICATE cancelled"), None));
    }

    // Decode client response
    let decoded = match BASE64.decode(client_data.trim()) {
        Ok(d) => d,
        Err(e) => {
            return Ok((
                ImapResponse::bad(&ctx.tag, format!("Invalid Base64: {}", e)),
                None,
            ));
        }
    };

    // Process the step
    handle_authenticate_step(
        session,
        &ctx.tag,
        ctx.mechanism,
        std::str::from_utf8(&decoded).unwrap_or(""),
        auth_backend,
    )
    .await
}

/// Handle a single authentication step
async fn handle_authenticate_step(
    session: &mut ImapSession,
    tag: &str,
    mut mechanism: Box<dyn rusmes_auth::sasl::SaslMechanism>,
    client_data: &str,
    auth_backend: &dyn AuthBackend,
) -> anyhow::Result<(ImapResponse, Option<AuthenticateContext>)> {
    let auth_backend_ref: &dyn AuthBackend = auth_backend;

    match mechanism
        .step(client_data.as_bytes(), auth_backend_ref)
        .await
    {
        Ok(rusmes_auth::sasl::SaslStep::Challenge { data }) => {
            // Send another challenge
            let encoded = BASE64.encode(&data);
            let ctx = AuthenticateContext {
                mechanism,
                state: AuthenticateState::Challenge,
                tag: tag.to_string(),
            };
            Ok((ImapResponse::new(None, "+", encoded), Some(ctx)))
        }
        Ok(rusmes_auth::sasl::SaslStep::Continue) => {
            // Need more data from client
            let ctx = AuthenticateContext {
                mechanism,
                state: AuthenticateState::Challenge,
                tag: tag.to_string(),
            };
            Ok((ImapResponse::new(None, "+", ""), Some(ctx)))
        }
        Ok(rusmes_auth::sasl::SaslStep::Done { success, username }) => {
            // Authentication completed
            if success && username.is_some() {
                session.state = ImapState::Authenticated;
                session.username = username.clone();
                let user_str = username
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| "user".to_string());
                Ok((
                    ImapResponse::ok(tag, format!("{} authenticated", user_str)),
                    None,
                ))
            } else {
                Ok((
                    ImapResponse::no(tag, "[AUTHENTICATIONFAILED] Authentication failed"),
                    None,
                ))
            }
        }
        Err(e) => Ok((
            ImapResponse::no(tag, format!("[AUTHENTICATIONFAILED] {}", e)),
            None,
        )),
    }
}

/// Parse AUTHENTICATE command
///
/// Syntax: AUTHENTICATE `<mechanism>` \[`<initial-response>`\]
///
/// Returns (mechanism_name, optional_initial_response)
pub fn parse_authenticate_args(args: &str) -> anyhow::Result<(String, Option<String>)> {
    let parts: Vec<&str> = args.split_whitespace().collect();

    if parts.is_empty() {
        return Err(anyhow::anyhow!("Missing mechanism name"));
    }

    let mechanism = parts[0].to_uppercase();
    let initial_response = if parts.len() > 1 {
        // Handle "=" as empty initial response (RFC 4959)
        if parts[1] == "=" {
            Some(String::new())
        } else {
            Some(parts[1].to_string())
        }
    } else {
        None
    };

    Ok((mechanism, initial_response))
}

/// Helper to create a SASL server with default configuration
pub fn create_default_sasl_server(hostname: String) -> SaslServer {
    use rusmes_auth::sasl::SaslConfig;
    let config = SaslConfig {
        enabled_mechanisms: vec![
            "PLAIN".to_string(),
            "LOGIN".to_string(),
            "CRAM-MD5".to_string(),
            "SCRAM-SHA-256".to_string(),
            "XOAUTH2".to_string(),
        ],
        hostname,
    };
    SaslServer::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rusmes_auth::sasl::SaslConfig;
    use rusmes_proto::Username;

    // Mock auth backend for testing
    struct MockAuthBackend {
        valid_users: Vec<(String, String)>,
    }

    #[async_trait]
    impl AuthBackend for MockAuthBackend {
        async fn authenticate(&self, username: &Username, password: &str) -> anyhow::Result<bool> {
            Ok(self
                .valid_users
                .iter()
                .any(|(u, p)| u == username.as_str() && p == password))
        }

        async fn verify_identity(&self, username: &Username) -> anyhow::Result<bool> {
            Ok(self.valid_users.iter().any(|(u, _)| u == username.as_str()))
        }

        async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
            Ok(vec![])
        }

        async fn create_user(&self, _username: &Username, _password: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn delete_user(&self, _username: &Username) -> anyhow::Result<()> {
            Ok(())
        }

        async fn change_password(
            &self,
            _username: &Username,
            _new_password: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_parse_authenticate_args_basic() {
        let (mechanism, initial_resp) =
            parse_authenticate_args("PLAIN").expect("PLAIN mechanism parse should succeed");
        assert_eq!(mechanism, "PLAIN");
        assert!(initial_resp.is_none());
    }

    #[test]
    fn test_parse_authenticate_args_with_initial_response() {
        let (mechanism, initial_resp) = parse_authenticate_args("PLAIN AHRlc3R1c2VyAHRlc3RwYXNz")
            .expect("PLAIN with initial response parse should succeed");
        assert_eq!(mechanism, "PLAIN");
        assert_eq!(initial_resp, Some("AHRlc3R1c2VyAHRlc3RwYXNz".to_string()));
    }

    #[test]
    fn test_parse_authenticate_args_empty_initial_response() {
        let (mechanism, initial_resp) = parse_authenticate_args("PLAIN =")
            .expect("PLAIN with empty initial response (=) parse should succeed");
        assert_eq!(mechanism, "PLAIN");
        assert_eq!(initial_resp, Some(String::new()));
    }

    #[test]
    fn test_parse_authenticate_args_case_insensitive() {
        let (mechanism, _) =
            parse_authenticate_args("plain").expect("lowercase plain parse should succeed");
        assert_eq!(mechanism, "PLAIN");

        let (mechanism, _) =
            parse_authenticate_args("Cram-Md5").expect("mixed-case Cram-Md5 parse should succeed");
        assert_eq!(mechanism, "CRAM-MD5");
    }

    #[test]
    fn test_parse_authenticate_args_no_mechanism() {
        let result = parse_authenticate_args("");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_authenticate_plain_with_initial_response() {
        let backend = MockAuthBackend {
            valid_users: vec![("testuser".to_string(), "testpass".to_string())],
        };

        let config = SaslConfig {
            enabled_mechanisms: vec!["PLAIN".to_string()],
            hostname: "localhost".to_string(),
        };
        let sasl_server = SaslServer::new(config);

        let mut session = ImapSession::new();

        // PLAIN credentials: \0testuser\0testpass encoded in base64
        let initial_response = BASE64.encode(b"\0testuser\0testpass");

        let (response, ctx) = handle_authenticate(
            &mut session,
            "A001",
            "PLAIN",
            Some(&initial_response),
            &sasl_server,
            &backend,
        )
        .await
        .expect("PLAIN auth with valid credentials should succeed");

        assert!(ctx.is_none()); // Should complete in one step
        assert!(response.format().contains("OK"));
        assert!(matches!(session.state(), ImapState::Authenticated));
    }

    #[tokio::test]
    async fn test_handle_authenticate_plain_wrong_credentials() {
        let backend = MockAuthBackend {
            valid_users: vec![("testuser".to_string(), "testpass".to_string())],
        };

        let config = SaslConfig {
            enabled_mechanisms: vec!["PLAIN".to_string()],
            hostname: "localhost".to_string(),
        };
        let sasl_server = SaslServer::new(config);

        let mut session = ImapSession::new();

        // Wrong password
        let initial_response = BASE64.encode(b"\0testuser\0wrongpass");

        let (response, ctx) = handle_authenticate(
            &mut session,
            "A001",
            "PLAIN",
            Some(&initial_response),
            &sasl_server,
            &backend,
        )
        .await
        .expect("PLAIN auth handler should not error even with wrong credentials");

        assert!(ctx.is_none());
        assert!(response.format().contains("NO"));
        assert!(response.format().contains("AUTHENTICATIONFAILED"));
        assert!(matches!(session.state(), ImapState::NotAuthenticated));
    }

    #[tokio::test]
    async fn test_handle_authenticate_unsupported_mechanism() {
        let backend = MockAuthBackend {
            valid_users: vec![],
        };

        let config = SaslConfig {
            enabled_mechanisms: vec!["PLAIN".to_string()],
            hostname: "localhost".to_string(),
        };
        let sasl_server = SaslServer::new(config);

        let mut session = ImapSession::new();

        let (response, ctx) = handle_authenticate(
            &mut session,
            "A001",
            "UNKNOWN",
            None,
            &sasl_server,
            &backend,
        )
        .await
        .expect("auth handler should not error for unsupported mechanism");

        assert!(ctx.is_none());
        assert!(response.format().contains("NO"));
        assert!(response.format().contains("not supported"));
    }

    #[tokio::test]
    async fn test_handle_authenticate_already_authenticated() {
        let backend = MockAuthBackend {
            valid_users: vec![],
        };

        let config = SaslConfig {
            enabled_mechanisms: vec!["PLAIN".to_string()],
            hostname: "localhost".to_string(),
        };
        let sasl_server = SaslServer::new(config);

        let mut session = ImapSession::new();
        session.state = ImapState::Authenticated; // Already authenticated

        let (response, ctx) =
            handle_authenticate(&mut session, "A001", "PLAIN", None, &sasl_server, &backend)
                .await
                .expect("auth handler should not error for already-authenticated session");

        assert!(ctx.is_none());
        assert!(response.format().contains("BAD"));
        assert!(response.format().contains("Already authenticated"));
    }

    #[tokio::test]
    async fn test_handle_authenticate_login_multi_step() {
        let backend = MockAuthBackend {
            valid_users: vec![("testuser".to_string(), "testpass".to_string())],
        };

        let config = SaslConfig {
            enabled_mechanisms: vec!["LOGIN".to_string()],
            hostname: "localhost".to_string(),
        };
        let sasl_server = SaslServer::new(config);

        let mut session = ImapSession::new();

        // Step 1: Start authentication
        let (response, ctx) =
            handle_authenticate(&mut session, "A001", "LOGIN", None, &sasl_server, &backend)
                .await
                .expect("LOGIN auth initiation should succeed");

        assert!(ctx.is_some());
        assert!(response.format().contains("+"));

        let ctx = ctx.expect("LOGIN step 1 should return a continuation context");

        // Step 2: Send username
        let username_b64 = BASE64.encode(b"testuser");
        let (response, ctx) =
            handle_authenticate_continue(&mut session, ctx, &username_b64, &backend)
                .await
                .expect("LOGIN step 2 (username) should succeed");

        assert!(ctx.is_some());
        assert!(response.format().contains("+"));

        let ctx = ctx.expect("LOGIN step 2 should return a continuation context for password");

        // Step 3: Send password
        let password_b64 = BASE64.encode(b"testpass");
        let (response, ctx) =
            handle_authenticate_continue(&mut session, ctx, &password_b64, &backend)
                .await
                .expect("LOGIN step 3 (password) should succeed");

        assert!(ctx.is_none());
        assert!(response.format().contains("OK"));
        assert!(matches!(session.state(), ImapState::Authenticated));
    }

    #[tokio::test]
    async fn test_handle_authenticate_cancel() {
        let backend = MockAuthBackend {
            valid_users: vec![],
        };

        let config = SaslConfig {
            enabled_mechanisms: vec!["LOGIN".to_string()],
            hostname: "localhost".to_string(),
        };
        let sasl_server = SaslServer::new(config);

        let mut session = ImapSession::new();

        // Start authentication
        let (_, ctx) =
            handle_authenticate(&mut session, "A001", "LOGIN", None, &sasl_server, &backend)
                .await
                .expect("LOGIN auth initiation should succeed");

        let ctx = ctx.expect("LOGIN auth initiation should return a continuation context");

        // Cancel with "*"
        let (response, ctx) = handle_authenticate_continue(&mut session, ctx, "*", &backend)
            .await
            .expect("auth cancellation via * should not error");

        assert!(ctx.is_none());
        assert!(response.format().contains("BAD"));
        assert!(response.format().contains("cancelled"));
    }

    #[test]
    fn test_create_default_sasl_server() {
        let server = create_default_sasl_server("localhost".to_string());

        assert!(server.is_mechanism_enabled("PLAIN"));
        assert!(server.is_mechanism_enabled("LOGIN"));
        assert!(server.is_mechanism_enabled("CRAM-MD5"));
        assert!(server.is_mechanism_enabled("SCRAM-SHA-256"));
        assert!(server.is_mechanism_enabled("XOAUTH2"));
    }
}
