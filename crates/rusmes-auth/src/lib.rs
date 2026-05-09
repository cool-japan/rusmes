//! # rusmes-auth
//!
//! Pluggable authentication backends for the RusMES mail server.
//!
//! ## Overview
//!
//! `rusmes-auth` provides a unified [`AuthBackend`] trait that abstracts over multiple
//! authentication strategies. All backends implement the same async interface, allowing
//! them to be composed, swapped, or wrapped by middleware such as the brute-force
//! protector found in the [`security`] module.
//!
//! ## Backends
//!
//! | Backend | Module | Notes |
//! |---------|--------|-------|
//! | File (htpasswd-style) | [`mod@file`] | bcrypt hashes, atomic writes |
//! | LDAP / Active Directory | [`backends::ldap`] | connection pooling, group filtering |
//! | SQL (SQLite / Postgres / MySQL) | [`backends::sql`] | bcrypt + Argon2 + SCRAM-SHA-256 |
//! | OAuth2 / OIDC | [`backends::oauth2`] | JWT validation, XOAUTH2 SASL |
//! | System (Unix) | [`backends::system`] | Pure Rust `/etc/shadow` auth |
//!
//! ## SASL Mechanisms
//!
//! The [`sasl`] module implements RFC-compliant SASL mechanisms on top of any `AuthBackend`:
//!
//! - `PLAIN` (RFC 4616)
//! - `LOGIN` (obsolete but widely supported)
//! - `CRAM-MD5` (RFC 2195)
//! - `SCRAM-SHA-256` (RFC 5802 / RFC 7677)
//! - `XOAUTH2` (RFC 7628)
//!
//! ## Security
//!
//! The [`security`] module provides:
//!
//! - Brute-force / account-lockout protection (progressive lockout)
//! - Per-IP rate limiting
//! - Password strength validation (entropy, character class, banned list)
//! - In-memory audit logging
//!
//! ## Example
//!
//! ```rust,no_run
//! use rusmes_auth::file::FileAuthBackend;
//! use rusmes_auth::AuthBackend;
//! use rusmes_proto::Username;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let backend = FileAuthBackend::new("/etc/rusmes/passwd").await?;
//! let user = Username::new("alice".to_string())?;
//! let ok = backend.authenticate(&user, "s3cr3t").await?;
//! println!("authenticated: {ok}");
//! # Ok(())
//! # }
//! ```

use async_trait::async_trait;
use rusmes_proto::Username;
use std::sync::Arc;

pub mod backends;
pub mod file;
pub mod sasl;
pub mod security;

// ============================================================================
// ScramCredentials — RFC 5802 credential bundle
// ============================================================================

/// SCRAM-SHA-256 credential bundle as defined by RFC 5802.
///
/// These values are derived from a user's password via PBKDF2-SHA-256 and stored
/// so that the server never needs to see the raw password for SCRAM authentication.
///
/// Derivation (per RFC 5802 §3):
/// ```text
/// SaltedPassword = PBKDF2-SHA-256(password, salt, i)
/// ClientKey      = HMAC-SHA-256(SaltedPassword, "Client Key")
/// StoredKey      = SHA-256(ClientKey)        // stored here
/// ServerKey      = HMAC-SHA-256(SaltedPassword, "Server Key")  // stored here
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScramCredentials {
    /// Random salt bytes used in the PBKDF2 derivation.
    pub salt: Vec<u8>,
    /// Number of PBKDF2 iterations (≥ 4096 recommended by RFC 7677).
    pub iteration_count: u32,
    /// `SHA-256(ClientKey)` — used to verify the client proof without storing the key.
    pub stored_key: Vec<u8>,
    /// `HMAC-SHA-256(SaltedPassword, "Server Key")` — used to produce the server signature.
    pub server_key: Vec<u8>,
}

// ============================================================================
// AuthBackendKind — config-driven factory enum
// ============================================================================

/// Discriminated-union config type for all supported authentication backends.
///
/// Call [`AuthBackendKind::build`] to construct an `Arc<dyn AuthBackend>` from
/// whichever variant was selected in the server configuration.
pub enum AuthBackendKind {
    /// File-based backend (bcrypt htpasswd format with optional SCRAM extension).
    File(FileBackendConfig),
    /// SQL database backend (PostgreSQL / MySQL / SQLite via sqlx).
    Sql(backends::sql::SqlConfig),
    /// LDAP / Active Directory backend.
    Ldap(backends::ldap::LdapConfig),
    /// OAuth2 / OIDC backend (JWT introspection, XOAUTH2 SASL).
    OAuth2(backends::oauth2::OAuth2Config),
}

/// Configuration for the file-based authentication backend.
#[derive(Debug, Clone, Default)]
pub struct FileBackendConfig {
    /// Path to the passwd file.
    pub path: String,
    /// Password-hashing algorithm used for *new* password writes.
    ///
    /// Existing hashes (whichever algorithm produced them) continue to verify
    /// regardless of this setting. Defaults to bcrypt for backwards compatibility
    /// with deployments that pre-date argon2 support.
    pub hash_algorithm: file::HashAlgorithm,
}

impl AuthBackendKind {
    /// Construct an `Arc<dyn AuthBackend>` from this configuration variant.
    ///
    /// The returned backend is immediately ready for use; SQL backends have their
    /// connection pool opened, LDAP backends hold a lazily-allocated pool, and the
    /// file backend has already read its passwd file into memory.
    pub async fn build(self) -> anyhow::Result<Arc<dyn AuthBackend>> {
        match self {
            AuthBackendKind::File(cfg) => {
                let backend =
                    file::FileAuthBackend::with_algorithm(&cfg.path, cfg.hash_algorithm).await?;
                Ok(Arc::new(backend))
            }
            AuthBackendKind::Sql(cfg) => {
                let backend = backends::sql::SqlBackend::new(cfg).await?;
                Ok(Arc::new(backend))
            }
            AuthBackendKind::Ldap(cfg) => {
                let backend = backends::ldap::LdapBackend::new(cfg);
                Ok(Arc::new(backend))
            }
            AuthBackendKind::OAuth2(cfg) => {
                let backend = backends::oauth2::OAuth2Backend::new(cfg);
                Ok(Arc::new(backend))
            }
        }
    }
}

#[async_trait]
pub trait AuthBackend: Send + Sync {
    /// Authenticate a user with username and password
    async fn authenticate(&self, username: &Username, password: &str) -> anyhow::Result<bool>;

    /// Verify if a username maps to a valid identity
    async fn verify_identity(&self, username: &Username) -> anyhow::Result<bool>;

    /// List all users (for admin CLI)
    async fn list_users(&self) -> anyhow::Result<Vec<Username>>;

    /// Create a new user with the given password
    async fn create_user(&self, username: &Username, password: &str) -> anyhow::Result<()>;

    /// Delete a user
    async fn delete_user(&self, username: &Username) -> anyhow::Result<()>;

    /// Change a user's password
    async fn change_password(&self, username: &Username, new_password: &str) -> anyhow::Result<()>;

    // ========================================================================
    // SCRAM-SHA-256 Support (Optional)
    // ========================================================================
    // IMPORTANT: SCRAM-SHA-256 requires different credential storage than
    // bcrypt. The following methods provide SCRAM-specific credential access.
    // Default implementations return errors to maintain backward compatibility.

    /// Fetch the full RFC 5802 SCRAM-SHA-256 credential bundle for a user.
    ///
    /// Returns `Ok(Some(creds))` when pre-computed SCRAM credentials exist,
    /// `Ok(None)` when this backend does not store SCRAM credentials for the
    /// user (SCRAM-SHA-256 should then be declined; PLAIN / LOGIN remain available),
    /// and `Err(...)` only on I/O or parse failures.
    ///
    /// The default implementation returns `Ok(None)`, meaning SQL / LDAP / OAuth2
    /// backends gracefully degrade without requiring any code changes in those backends.
    /// Only the file backend overrides this with a real implementation.
    async fn fetch_scram_credentials(
        &self,
        _user: &str,
    ) -> anyhow::Result<Option<ScramCredentials>> {
        Ok(None)
    }

    /// Get SCRAM-SHA-256 parameters (salt, iteration count) for a user
    ///
    /// Returns (salt, iterations) if SCRAM credentials are stored.
    /// Default implementation returns an error indicating SCRAM is not supported.
    async fn get_scram_params(&self, _username: &str) -> anyhow::Result<(Vec<u8>, u32)> {
        Err(anyhow::anyhow!(
            "SCRAM-SHA-256 credential storage not implemented in this AuthBackend"
        ))
    }

    /// Get SCRAM-SHA-256 StoredKey for a user
    ///
    /// StoredKey = SHA256(ClientKey) where ClientKey = HMAC(SaltedPassword, "Client Key")
    /// Default implementation returns an error indicating SCRAM is not supported.
    async fn get_scram_stored_key(&self, _username: &str) -> anyhow::Result<Vec<u8>> {
        Err(anyhow::anyhow!(
            "SCRAM-SHA-256 credential storage not implemented in this AuthBackend"
        ))
    }

    /// Get SCRAM-SHA-256 ServerKey for a user
    ///
    /// ServerKey = HMAC(SaltedPassword, "Server Key")
    /// Default implementation returns an error indicating SCRAM is not supported.
    async fn get_scram_server_key(&self, _username: &str) -> anyhow::Result<Vec<u8>> {
        Err(anyhow::anyhow!(
            "SCRAM-SHA-256 credential storage not implemented in this AuthBackend"
        ))
    }

    /// Store SCRAM-SHA-256 credentials for a user
    ///
    /// This should store: salt, iterations, StoredKey, and ServerKey
    /// Default implementation returns an error indicating SCRAM is not supported.
    async fn store_scram_credentials(
        &self,
        _username: &Username,
        _salt: Vec<u8>,
        _iterations: u32,
        _stored_key: Vec<u8>,
        _server_key: Vec<u8>,
    ) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "SCRAM-SHA-256 credential storage not implemented in this AuthBackend"
        ))
    }

    // ========================================================================
    // APOP (MD5 Digest) Support (Optional)
    // ========================================================================
    // APOP requires access to plaintext password to compute MD5 digest.
    // This is incompatible with bcrypt and most secure password storage.
    // Backends can optionally support APOP by storing plaintext passwords
    // separately (not recommended for production).

    /// Get plaintext password for APOP authentication
    ///
    /// Returns the plaintext password if available.
    /// Default implementation returns an error indicating APOP is not supported.
    ///
    /// WARNING: This method exposes plaintext passwords and should only be used
    /// for APOP authentication. Consider disabling APOP in production environments.
    async fn get_apop_secret(&self, _username: &Username) -> anyhow::Result<String> {
        Err(anyhow::anyhow!(
            "APOP authentication not supported by this AuthBackend"
        ))
    }

    // ========================================================================
    // Bearer Token / OAuth2 Support (Optional)
    // ========================================================================

    /// Verify a Bearer token and return the authenticated username.
    ///
    /// This is the entry point for HTTP Bearer authentication (e.g. in JMAP).
    /// The default implementation rejects all tokens unconditionally so that
    /// backends without OAuth2 support never silently accept Bearer credentials.
    ///
    /// Backends that support Bearer / JWT verification (e.g. [`backends::oauth2`])
    /// override this method to perform real token introspection or JWT validation.
    ///
    /// # Errors
    /// Returns an `anyhow::Error` wrapping a rejected-token message if the
    /// token is invalid, expired, or this backend does not support Bearer auth.
    /// Callers that want a typed rejection should map the error to their own
    /// error type (e.g. `AuthError::Unauthorized`).
    async fn verify_bearer_token(&self, token: &str) -> anyhow::Result<Username> {
        // Suppress unused-variable lint without doing anything with the token.
        let _ = token;
        Err(anyhow::anyhow!(
            "Bearer token authentication is not supported by this AuthBackend"
        ))
    }
}
