//! Per-protocol TLS certificate configuration for RusMES.
//!
//! The `[tls]` TOML section allows a shared default TLS certificate/key pair
//! with optional per-protocol overrides for SMTP, IMAP, POP3, and JMAP.
//!
//! ## Backward compatibility
//!
//! Existing configurations that specify a top-level `[tls]` section with
//! `cert_path` / `key_path` directly are supported via `#[serde(alias)]`.
//!
//! ## Example
//!
//! ```toml
//! # Shared default
//! [tls.default]
//! cert_path = "/etc/rusmes/tls/cert.pem"
//! key_path  = "/etc/rusmes/tls/key.pem"
//!
//! # IMAP uses its own certificate
//! [tls.imap]
//! cert_path = "/etc/rusmes/tls/imap-cert.pem"
//! key_path  = "/etc/rusmes/tls/imap-key.pem"
//!
//! # SMTP with mutual TLS (require client certificate)
//! [tls.smtp]
//! cert_path      = "/etc/rusmes/tls/smtp-cert.pem"
//! key_path       = "/etc/rusmes/tls/smtp-key.pem"
//! client_auth    = "required"
//! client_ca_path = "/etc/rusmes/tls/client-ca.pem"
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Client certificate authentication mode for mutual TLS.
///
/// Controls whether the server requests and/or requires a client certificate
/// during the TLS handshake.  Default is [`ClientAuthMode::Disabled`] (no
/// client certificate requested).
///
/// ## TOML spelling
///
/// ```toml
/// client_auth = "disabled"   # default — no client certificate requested
/// client_auth = "optional"   # certificate is requested but not required
/// client_auth = "required"   # handshake fails if no valid certificate is presented
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClientAuthMode {
    /// No client certificate is requested.  This is the default and matches
    /// the previous behaviour (i.e. backward-compatible).
    #[default]
    Disabled,
    /// Client certificate is requested but the handshake succeeds even when
    /// the client does not present one.  The certificate chain, when present,
    /// is still verified against `client_ca_path`.
    Optional,
    /// Client certificate is mandatory.  The TLS handshake is aborted if the
    /// client does not present a certificate signed by the configured CA.
    Required,
}

/// Which protocol is requesting a TLS endpoint configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolKind {
    /// SMTP (port 25 / 587 / 465)
    Smtp,
    /// IMAP4rev1 (port 143 / 993)
    Imap,
    /// POP3 (port 110 / 995)
    Pop3,
    /// JMAP over HTTP/TLS
    Jmap,
}

/// A single TLS endpoint: certificate chain and private key paths.
///
/// Both fields must be present for the endpoint to be valid.
///
/// ## Mutual TLS
///
/// Set `client_auth` to `"optional"` or `"required"` and supply a
/// `client_ca_path` pointing to a PEM file that contains the CA certificate
/// (or chain) used to sign client certificates.  When `client_auth` is
/// `"disabled"` (the default) both `client_auth` and `client_ca_path` are
/// ignored.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TlsEndpointConfig {
    /// Default: none. Path to the PEM-encoded certificate chain file.
    /// Use `#[serde(alias = "cert_path")]` to accept the legacy spelling.
    #[serde(alias = "cert_path")]
    pub cert_path: PathBuf,

    /// Default: none. Path to the PEM-encoded private key file.
    /// Use `#[serde(alias = "key_path")]` to accept the legacy spelling.
    #[serde(alias = "key_path")]
    pub key_path: PathBuf,

    /// Default: [`ClientAuthMode::Disabled`].  Controls whether a client
    /// certificate is requested / required during the TLS handshake.
    #[serde(default)]
    pub client_auth: ClientAuthMode,

    /// Default: `None`.  Path to a PEM-encoded CA certificate file used to
    /// verify the client certificate chain.  Required when `client_auth` is
    /// `"optional"` or `"required"`.
    #[serde(default)]
    pub client_ca_path: Option<PathBuf>,
}

impl TlsEndpointConfig {
    /// Validate that both paths are non-empty strings.
    ///
    /// Existence on disk is not checked here — that is deferred to server
    /// startup so that config validation can succeed in CI without real certs.
    ///
    /// Also validates that `client_ca_path` is set when `client_auth` is not
    /// `Disabled`.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.cert_path.as_os_str().is_empty() {
            anyhow::bail!("TLS cert_path cannot be empty");
        }
        if self.key_path.as_os_str().is_empty() {
            anyhow::bail!("TLS key_path cannot be empty");
        }
        if self.client_auth != ClientAuthMode::Disabled && self.client_ca_path.is_none() {
            anyhow::bail!(
                "TLS client_ca_path must be set when client_auth is '{}' (not 'disabled')",
                match self.client_auth {
                    ClientAuthMode::Optional => "optional",
                    ClientAuthMode::Required => "required",
                    ClientAuthMode::Disabled => unreachable!(),
                }
            );
        }
        Ok(())
    }
}

/// Top-level TLS configuration block (`[tls]` in TOML).
///
/// `default` is the fallback used by any protocol that does not have a
/// dedicated override. Per-protocol overrides take precedence when present.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TlsConfig {
    /// Default certificate/key pair used as a fallback for all protocols.
    pub default: TlsEndpointConfig,

    /// Default: `None`. SMTP-specific TLS certificate override.
    #[serde(default)]
    pub smtp: Option<TlsEndpointConfig>,

    /// Default: `None`. IMAP-specific TLS certificate override.
    #[serde(default)]
    pub imap: Option<TlsEndpointConfig>,

    /// Default: `None`. POP3-specific TLS certificate override.
    #[serde(default)]
    pub pop3: Option<TlsEndpointConfig>,

    /// Default: `None`. JMAP-specific TLS certificate override.
    #[serde(default)]
    pub jmap: Option<TlsEndpointConfig>,
}

impl TlsConfig {
    /// Return the [`TlsEndpointConfig`] that should be used for `proto`.
    ///
    /// Returns the per-protocol override when present, otherwise the
    /// `default` endpoint.
    pub fn tls_for_protocol(&self, proto: ProtocolKind) -> &TlsEndpointConfig {
        let override_cfg = match proto {
            ProtocolKind::Smtp => self.smtp.as_ref(),
            ProtocolKind::Imap => self.imap.as_ref(),
            ProtocolKind::Pop3 => self.pop3.as_ref(),
            ProtocolKind::Jmap => self.jmap.as_ref(),
        };
        override_cfg.unwrap_or(&self.default)
    }

    /// Validate all configured TLS endpoints.
    pub fn validate(&self) -> anyhow::Result<()> {
        self.default
            .validate()
            .map_err(|e| anyhow::anyhow!("tls.default: {}", e))?;
        if let Some(ref s) = self.smtp {
            s.validate()
                .map_err(|e| anyhow::anyhow!("tls.smtp: {}", e))?;
        }
        if let Some(ref i) = self.imap {
            i.validate()
                .map_err(|e| anyhow::anyhow!("tls.imap: {}", e))?;
        }
        if let Some(ref p) = self.pop3 {
            p.validate()
                .map_err(|e| anyhow::anyhow!("tls.pop3: {}", e))?;
        }
        if let Some(ref j) = self.jmap {
            j.validate()
                .map_err(|e| anyhow::anyhow!("tls.jmap: {}", e))?;
        }
        Ok(())
    }
}
