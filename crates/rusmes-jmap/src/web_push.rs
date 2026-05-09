//! WebPush client for RFC 8030 push + RFC 8444 VAPID.
//!
//! # Implementation decision
//!
//! Dependencies chosen: `p256` (ECDSA key generation/loading), `jsonwebtoken`
//! (ES256 JWT signing — already a workspace dep with `rust_crypto` feature
//! which pulls in `p256`), `reqwest` (HTTP POST), `base64` (URL-safe encoding).
//!
//! The `web-push` crate was NOT used: it pulls in `openssl-sys` via `openssl`
//! as a transitive dependency on some targets, violating the COOLJAPAN Pure
//! Rust policy.  All chosen crates are 100% Pure Rust.
//!
//! # RFC 8291 encryption
//!
//! Message-Encryption (RFC 8291 — AES-128-GCM + ECDH + HKDF) is **deferred**.
//! When a subscription includes `keys`, the server still sends an unencrypted
//! "tickle" (zero-byte body) so the client is woken up.  RFC 8291 encryption
//! will be implemented in a follow-up slice once the key-agreement primitives
//! are fully stabilised in the workspace.  The `PushSubscription.keys` field
//! is preserved in storage so the feature can be enabled without breaking
//! existing subscriptions.

use crate::types::PushSubscription;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use p256::ecdsa::SigningKey;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Errors that can occur during WebPush operations.
#[derive(Debug, Error)]
pub enum WebPushError {
    /// The push endpoint returned HTTP 410 Gone — subscription should be removed.
    #[error("push endpoint returned 410 Gone")]
    Gone,

    /// HTTP transport error (connection refused, timeout, etc.)
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    /// JWT signing failed.
    #[error("VAPID JWT signing error: {0}")]
    JwtSigning(#[from] jsonwebtoken::errors::Error),

    /// Private key PEM could not be loaded or generated.
    #[error("VAPID key error: {0}")]
    KeyError(String),

    /// The push endpoint returned an unexpected non-2xx status (not 410).
    #[error("push endpoint returned unexpected status {0}")]
    UnexpectedStatus(u16),
}

/// VAPID JWT claims (RFC 8444).
#[derive(Debug, Serialize, Deserialize)]
struct VapidClaims {
    /// Audience: origin of the push endpoint URL.
    aud: String,
    /// Subject: `mailto:` URI or URL for the push provider to contact.
    sub: String,
    /// Expiry: UNIX timestamp.
    exp: u64,
}

/// WebPush client.
///
/// Cheap to clone (all fields are `Arc`-backed).
#[derive(Clone)]
pub struct WebPushClient {
    http: reqwest::Client,
    /// ECDSA signing key (P-256) used for VAPID JWTs.
    vapid_key: std::sync::Arc<SigningKey>,
    /// Base64url-encoded uncompressed public key point (used in `Crypto-Key` header).
    vapid_pubkey_base64url: String,
    /// `mailto:` or HTTPS subject for VAPID `sub` claim.
    admin_sub: String,
}

impl std::fmt::Debug for WebPushClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebPushClient")
            .field("vapid_pubkey_base64url", &self.vapid_pubkey_base64url)
            .field("admin_sub", &self.admin_sub)
            .finish_non_exhaustive()
    }
}

impl WebPushClient {
    /// Construct from an existing PEM-encoded P-256 EC private key.
    ///
    /// If `vapid_pem` is `None`, a fresh key is generated in memory.
    pub fn new(vapid_pem: Option<&[u8]>, admin_email: &str) -> Result<Self, WebPushError> {
        let signing_key = match vapid_pem {
            Some(pem_bytes) => {
                let pem_str = std::str::from_utf8(pem_bytes)
                    .map_err(|e| WebPushError::KeyError(format!("PEM is not valid UTF-8: {e}")))?;
                SigningKey::from_pkcs8_pem(pem_str)
                    .map_err(|e| WebPushError::KeyError(format!("Failed to load VAPID key: {e}")))?
            }
            None => {
                // Generate an ephemeral key.
                let mut rng_buf = [0u8; 32];
                getrandom::fill(&mut rng_buf)
                    .map_err(|e| WebPushError::KeyError(format!("RNG failure: {e}")))?;
                // Use the raw bytes as a scalar (this is deterministic from the entropy).
                SigningKey::from_slice(&rng_buf)
                    .map_err(|e| WebPushError::KeyError(format!("Key generation failed: {e}")))?
            }
        };

        let pubkey_bytes = p256::ecdsa::VerifyingKey::from(&signing_key)
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        let vapid_pubkey_base64url = URL_SAFE_NO_PAD.encode(&pubkey_bytes);

        let admin_sub = if admin_email.contains('@') {
            format!("mailto:{admin_email}")
        } else {
            admin_email.to_string()
        };

        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(WebPushError::Http)?,
            vapid_key: std::sync::Arc::new(signing_key),
            vapid_pubkey_base64url,
            admin_sub,
        })
    }

    /// Load or generate the VAPID key, optionally persisting it.
    ///
    /// - If `key_path` is `Some` and the file exists: loads the PEM.
    /// - If `key_path` is `Some` and the file does NOT exist: generates a new
    ///   key and writes it to the path so subsequent restarts use the same key.
    /// - If `key_path` is `None`: generates an ephemeral in-memory key.
    pub fn new_with_persistence(
        key_path: Option<&std::path::Path>,
        admin_email: &str,
    ) -> Result<Self, WebPushError> {
        match key_path {
            None => Self::new(None, admin_email),
            Some(path) if path.exists() => {
                let pem = std::fs::read(path).map_err(|e| {
                    WebPushError::KeyError(format!("Cannot read VAPID key file: {e}"))
                })?;
                Self::new(Some(&pem), admin_email)
            }
            Some(path) => {
                // Generate and persist.
                let client = Self::new(None, admin_email)?;
                let pem = client
                    .vapid_key
                    .to_pkcs8_pem(Default::default())
                    .map_err(|e| {
                        WebPushError::KeyError(format!("PEM serialization failed: {e}"))
                    })?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        WebPushError::KeyError(format!("Cannot create VAPID key dir: {e}"))
                    })?;
                }
                std::fs::write(path, pem.as_bytes())
                    .map_err(|e| WebPushError::KeyError(format!("Cannot write VAPID key: {e}")))?;
                Ok(client)
            }
        }
    }

    /// Return the base64url-encoded uncompressed VAPID public key.
    ///
    /// This is the value that must be shared with push endpoints so they can
    /// verify VAPID JWTs.  Exposed for tests and for generating the VAPID public
    /// key header in `Crypto-Key`.
    pub fn vapid_pubkey_base64url(&self) -> &str {
        &self.vapid_pubkey_base64url
    }

    /// Build a VAPID JWT for the given push endpoint origin.
    ///
    /// The JWT is valid for 24 hours from now (RFC 8444 §3).
    pub(crate) fn build_vapid_jwt(&self, endpoint_origin: &str) -> Result<String, WebPushError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| WebPushError::KeyError(format!("System clock error: {e}")))?;
        let exp = now.as_secs() + 86_400; // 24 hours

        let claims = VapidClaims {
            aud: endpoint_origin.to_string(),
            sub: self.admin_sub.clone(),
            exp,
        };

        let header = Header::new(Algorithm::ES256);

        // Convert the p256 SigningKey to DER for jsonwebtoken.
        let der = self
            .vapid_key
            .to_pkcs8_der()
            .map_err(|e| WebPushError::KeyError(format!("Key DER export failed: {e}")))?;
        let encoding_key = EncodingKey::from_ec_der(der.as_bytes());

        let token = jsonwebtoken::encode(&header, &claims, &encoding_key)?;
        Ok(token)
    }

    /// Send a WebPush message to `subscription`.
    ///
    /// When `subscription.keys` is `None` (or RFC 8291 encryption is not yet
    /// implemented) the body is left empty (tickle semantics).  The VAPID JWT
    /// is always included in the `Authorization` header per RFC 8444.
    pub async fn send(
        &self,
        subscription: &PushSubscription,
        payload: &[u8],
    ) -> Result<(), WebPushError> {
        // Derive the origin (scheme + host [+ port]) from the subscription URL
        // for the VAPID `aud` claim.
        let endpoint_origin = extract_origin(&subscription.url).ok_or_else(|| {
            WebPushError::KeyError(format!(
                "Cannot determine origin from URL: {}",
                subscription.url
            ))
        })?;

        let jwt = self.build_vapid_jwt(&endpoint_origin)?;

        // VAPID authorization: `vapid t=<token>,k=<pubkey>` (RFC 8292 §2).
        let authorization = format!("vapid t={},k={}", jwt, self.vapid_pubkey_base64url);

        // RFC 8030 §5.2: TTL header is required.
        const TTL_SECONDS: u32 = 86_400;

        // RFC 8291: when keys are absent we send a tickle (empty body).
        // The payload argument is accepted but ignored for now; see module-level
        // note on RFC 8291 encryption deferral.
        let body = if subscription.keys.is_none() || payload.is_empty() {
            bytes::Bytes::new()
        } else {
            // Encryption is deferred — fall back to tickle.
            bytes::Bytes::new()
        };

        let response = self
            .http
            .post(&subscription.url)
            .header("Authorization", authorization)
            .header("TTL", TTL_SECONDS.to_string())
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .send()
            .await?;

        let status = response.status().as_u16();
        match status {
            200..=299 => Ok(()),
            410 => Err(WebPushError::Gone),
            other => Err(WebPushError::UnexpectedStatus(other)),
        }
    }
}

/// Extract the `scheme://host[:port]` origin from a URL string.
///
/// Returns `None` if the URL cannot be parsed or has no host.
fn extract_origin(url: &str) -> Option<String> {
    // Simple parser: find scheme, then "://", then extract up to the first
    // "/" or end.  Avoids pulling in the `url` crate just for this.
    let after_scheme = url.split_once("://")?.1;
    let host_and_rest = after_scheme.split('/').next()?;
    let scheme = url.split("://").next()?;
    Some(format!("{scheme}://{host_and_rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_origin_https() {
        let url = "https://push.example.com/v1/subscriptions/abc123";
        assert_eq!(
            extract_origin(url),
            Some("https://push.example.com".to_string())
        );
    }

    #[test]
    fn test_extract_origin_with_port() {
        let url = "https://push.example.com:8443/endpoint";
        assert_eq!(
            extract_origin(url),
            Some("https://push.example.com:8443".to_string())
        );
    }

    #[test]
    fn test_vapid_client_ephemeral_key() {
        let client = WebPushClient::new(None, "admin@example.com").unwrap();
        assert!(!client.vapid_pubkey_base64url().is_empty());
        assert!(client.admin_sub.starts_with("mailto:"));
    }

    #[test]
    fn test_build_vapid_jwt() {
        let client = WebPushClient::new(None, "admin@example.com").unwrap();
        let jwt = client.build_vapid_jwt("https://push.example.com").unwrap();
        // JWT has three base64url parts separated by dots.
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT must have header.payload.signature");
    }
}
