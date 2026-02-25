//! ACME v2 protocol client for automatic TLS certificate management
//!
//! This crate implements the [ACME v2](https://www.rfc-editor.org/rfc/rfc8555) protocol
//! (Automatic Certificate Management Environment) to obtain and renew TLS certificates
//! from [Let's Encrypt](https://letsencrypt.org/) or any other ACME-compatible CA.
//!
//! # Key Features
//!
//! - **HTTP-01 challenge** — serves the challenge token over HTTP on
//!   `/.well-known/acme-challenge/<token>` via an in-memory map that can be mounted
//!   on any HTTP server (see [`Http01Handler`]).
//! - **DNS-01 challenge** — creates `_acme-challenge.<domain>` TXT records through a
//!   pluggable [`DnsProvider`] trait; includes a [`MockDnsProvider`] for testing
//!   (see [`Dns01Handler`]).
//! - **Automatic renewal** — [`RenewalManager`] runs a background Tokio task that
//!   checks the certificate expiry at a configurable interval and renews proactively
//!   (default: 30 days before expiry).
//! - **CSR generation** — [`CsrGenerator`] uses `rcgen` to produce ECDSA P-256 or RSA
//!   CSRs without any C/Fortran dependencies.
//! - **Certificate storage** — `CertificateStorage` manages per-domain `*.crt`,
//!   `*.key`, and `*.chain` files with correct Unix permissions (0o600 for keys,
//!   0o644 for certs).
//! - **Staging / Production** — the [`AcmeConfig`] builder exposes a `.staging()`
//!   method that switches to the Let's Encrypt staging environment.
//!
//! # Usage
//!
//! ```rust,no_run
//! use rusmes_acme::{AcmeClient, AcmeConfig, ChallengeType, Http01Handler, RenewalManager};
//!
//! # async fn example() -> rusmes_acme::Result<()> {
//! // Build configuration
//! let config = AcmeConfig::new(
//!     "admin@example.com".to_string(),
//!     vec!["example.com".to_string(), "www.example.com".to_string()],
//! )
//! .challenge_type(ChallengeType::Http01)
//! .renewal(30, 3600);
//!
//! // Create ACME client and attach an HTTP-01 handler
//! let http_handler = Http01Handler::new();
//! let client = AcmeClient::new(config.clone())?
//!     .with_http01_handler(http_handler);
//!
//! // Request a certificate (blocks until ACME challenge completes)
//! let cert = client.request_certificate().await?;
//! cert.save(&config.cert_path, &config.key_path).await?;
//!
//! // Start automatic renewal in the background
//! let manager = RenewalManager::new(client, config);
//! manager.start().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Error Handling
//!
//! All fallible operations return [`Result<T>`][crate::Result], which aliases
//! `std::result::Result<T, AcmeError>`. The [`AcmeError`] enum covers ACME protocol
//! failures, challenge failures, validation errors, I/O errors, HTTP client errors,
//! and JSON serialisation errors.
//!
//! # Relevant Standards
//!
//! - ACME v2: [RFC 8555](https://www.rfc-editor.org/rfc/rfc8555)
//! - HTTP-01 challenge: [RFC 8555 §8.3](https://www.rfc-editor.org/rfc/rfc8555#section-8.3)
//! - DNS-01 challenge: [RFC 8555 §8.4](https://www.rfc-editor.org/rfc/rfc8555#section-8.4)
//! - TLS certificate format: [RFC 5280](https://www.rfc-editor.org/rfc/rfc5280) (X.509 v3)
//! - CSR format: [RFC 2986](https://www.rfc-editor.org/rfc/rfc2986) (PKCS #10)

pub mod cert;
pub mod challenge;
pub mod client;
pub mod config;
pub mod dns01;
pub mod http01;
pub mod renewal;
pub mod storage;

pub use cert::{Certificate, CsrGenerator, KeyType};
pub use client::AcmeClient;
pub use config::{AcmeConfig, ChallengeType};
pub use dns01::{Dns01Handler, DnsProvider, MockDnsProvider};
pub use http01::Http01Handler;
pub use renewal::RenewalManager;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AcmeError {
    #[error("ACME protocol error: {0}")]
    Protocol(String),

    #[error("Challenge failed: {0}")]
    ChallengeFailed(String),

    #[error("Certificate validation failed: {0}")]
    ValidationFailed(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, AcmeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = AcmeError::Protocol("test".to_string());
        assert_eq!(err.to_string(), "ACME protocol error: test");
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: AcmeError = io_err.into();
        assert!(matches!(err, AcmeError::Io(_)));
    }
}
