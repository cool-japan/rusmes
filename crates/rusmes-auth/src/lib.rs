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

pub mod backends;
pub mod file;
pub mod sasl;
pub mod security;

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
}
