//! Protocol listener configuration types.
//!
//! This module contains the per-protocol server listener configuration structs:
//! [`SmtpServerConfig`], [`SmtpOutboundConfig`], [`RateLimitConfig`],
//! [`ImapServerConfig`], [`JmapServerConfig`], [`JmapPushConfig`],
//! [`Pop3ServerConfig`], and [`RelayConfig`].

use crate::parse::{
    default_idle_timeout, default_max_connections_per_ip, default_max_total_connections,
    default_reaper_interval, parse_duration, parse_size,
};
use serde::{Deserialize, Serialize};

// --------------------------------------------------------------------------
// SmtpOutboundConfig
// --------------------------------------------------------------------------

/// Configuration for outbound SMTP connection pooling.
///
/// Controls the pool of reusable outbound SMTP connections maintained by
/// `OutboundPool`.  Defaults are intentionally conservative; tune for your
/// deployment's message volume.
///
/// ## TOML example
///
/// ```toml
/// [smtp.outbound]
/// idle_timeout_secs = 30
/// per_remote_cap    = 8
/// global_cap        = 256
/// ```
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmtpOutboundConfig {
    /// Default: `30`. Seconds a connection may sit idle before the background
    /// reaper closes it.
    #[serde(default = "default_outbound_idle_timeout")]
    pub idle_timeout_secs: u64,

    /// Default: `8`. Maximum pooled connections to a single remote address.
    #[serde(default = "default_outbound_per_remote_cap")]
    pub per_remote_cap: usize,

    /// Default: `256`. Total pooled connections across all remote addresses.
    #[serde(default = "default_outbound_global_cap")]
    pub global_cap: usize,
}

fn default_outbound_idle_timeout() -> u64 {
    30
}

fn default_outbound_per_remote_cap() -> usize {
    8
}

fn default_outbound_global_cap() -> usize {
    256
}

impl Default for SmtpOutboundConfig {
    fn default() -> Self {
        Self {
            idle_timeout_secs: default_outbound_idle_timeout(),
            per_remote_cap: default_outbound_per_remote_cap(),
            global_cap: default_outbound_global_cap(),
        }
    }
}

// --------------------------------------------------------------------------
// SmtpServerConfig
// --------------------------------------------------------------------------

/// SMTP server listener configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmtpServerConfig {
    /// Required. IP address or hostname on which the SMTP listener binds
    /// (e.g. `"0.0.0.0"` for all interfaces).
    pub host: String,

    /// Required. SMTP port number (typically `25` for server-to-server or
    /// `587` for mail submission). Must be in the range `1–65535`.
    pub port: u16,

    /// Default: `None`. SMTPS (implicit TLS) port number, typically `465`.
    /// Requires a `[tls]` section to be configured.
    #[serde(default)]
    pub tls_port: Option<u16>,

    /// Required. Maximum accepted message size, expressed as a human-readable
    /// string (e.g. `"50MB"`, `"1GB"`). Parsed by [`SmtpServerConfig::max_message_size_bytes`].
    pub max_message_size: String,

    /// Default: `false`. When `true`, the server requires SMTP AUTH before
    /// accepting mail for delivery. Recommended for submission ports.
    #[serde(default)]
    pub require_auth: bool,

    /// Default: `false`. When `true`, advertises the STARTTLS extension and
    /// upgrades connections on demand. Requires a `[tls]` section.
    #[serde(default)]
    pub enable_starttls: bool,

    /// Default: `None`. Per-IP and per-hour rate limiting for SMTP connections.
    /// When absent no rate limiting is applied.
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,

    /// Default: `SmtpOutboundConfig::default()`. Outbound connection pool
    /// parameters.  Omitting the `[smtp.outbound]` subsection uses built-in
    /// defaults (30 s idle, 8 per-remote, 256 global cap).
    #[serde(default)]
    pub outbound: SmtpOutboundConfig,
}

impl SmtpServerConfig {
    /// Parse max message size to bytes.
    pub fn max_message_size_bytes(&self) -> anyhow::Result<usize> {
        parse_size(&self.max_message_size)
    }
}

// --------------------------------------------------------------------------
// RateLimitConfig
// --------------------------------------------------------------------------

/// Per-IP SMTP rate limiting parameters.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RateLimitConfig {
    /// Default: `10`. Maximum number of simultaneous SMTP connections allowed
    /// from a single remote IP address.
    #[serde(default = "default_max_connections_per_ip")]
    pub max_connections_per_ip: usize,

    /// Required. Maximum number of accepted messages per `window_duration`
    /// from a single IP address.
    pub max_messages_per_hour: u32,

    /// Required. Length of the rate-limiting window as a human-readable
    /// duration string (e.g. `"1h"`, `"30m"`, `"3600s"`).
    pub window_duration: String,
}

impl RateLimitConfig {
    /// Parse window duration to seconds.
    pub fn window_duration_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.window_duration)
    }
}

// --------------------------------------------------------------------------
// ImapServerConfig
// --------------------------------------------------------------------------

/// IMAP4rev1 server listener configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImapServerConfig {
    /// Required. IP address or hostname on which the IMAP listener binds
    /// (e.g. `"0.0.0.0"` for all interfaces).
    pub host: String,

    /// Required. IMAP port number (typically `143`). Must be in `1–65535`.
    pub port: u16,

    /// Default: `None`. IMAPS (implicit TLS) port number, typically `993`.
    /// Requires a `[tls]` section to be configured.
    #[serde(default)]
    pub tls_port: Option<u16>,
}

// --------------------------------------------------------------------------
// JmapServerConfig / JmapPushConfig
// --------------------------------------------------------------------------

/// JMAP over HTTP server listener configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JmapServerConfig {
    /// Required. IP address or hostname on which the JMAP HTTP listener binds
    /// (e.g. `"0.0.0.0"` for all interfaces).
    pub host: String,

    /// Required. HTTP port number for the JMAP API (typically `8080`).
    /// Must be in the range `1–65535`.
    pub port: u16,

    /// Required. Externally reachable base URL returned in JMAP Session
    /// resources (e.g. `"https://jmap.example.com"`).
    pub base_url: String,

    /// Optional WebPush / VAPID push delivery configuration.
    /// When absent, WebPush delivery is disabled.
    #[serde(default)]
    pub push: Option<JmapPushConfig>,
}

/// JMAP WebPush delivery configuration (RFC 8030 + RFC 8444 VAPID).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JmapPushConfig {
    /// Path to the VAPID ES256 private key PEM file.
    ///
    /// If the file does not exist when the server starts it will be
    /// generated and written to this path automatically.  Omit this field
    /// to use an ephemeral in-memory key (the key changes on every restart,
    /// which forces all push subscriptions to re-verify).
    #[serde(default)]
    pub vapid_key_path: Option<std::path::PathBuf>,

    /// `mailto:` URI or email address used as the `sub` claim in VAPID JWTs.
    ///
    /// RFC 8444 requires this to identify a point of contact for push
    /// endpoint operators.  Defaults to `"mailto:admin@localhost"`.
    #[serde(default = "default_vapid_admin_email")]
    pub admin_email: String,
}

fn default_vapid_admin_email() -> String {
    "admin@localhost".to_string()
}

impl Default for JmapPushConfig {
    fn default() -> Self {
        Self {
            vapid_key_path: None,
            admin_email: default_vapid_admin_email(),
        }
    }
}

// --------------------------------------------------------------------------
// Pop3ServerConfig
// --------------------------------------------------------------------------

/// POP3 server listener configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Pop3ServerConfig {
    /// Required. IP address or hostname on which the POP3 listener binds
    /// (e.g. `"0.0.0.0"` for all interfaces).
    pub host: String,

    /// Required. POP3 port number (typically `110`). Must be in `1–65535`.
    pub port: u16,

    /// Default: `None`. POP3S (implicit TLS) port number, typically `995`.
    /// Requires a `[tls]` section to be configured.
    #[serde(default)]
    pub tls_port: Option<u16>,

    /// Default: `600`. Session inactivity timeout in seconds after which an
    /// idle POP3 connection is dropped.
    #[serde(default = "default_pop3_timeout")]
    pub timeout_seconds: u64,

    /// Default: `false`. When `true`, advertises the STLS capability
    /// (RFC 2595) to upgrade plain POP3 connections to TLS.
    #[serde(default)]
    pub enable_stls: bool,
}

fn default_pop3_timeout() -> u64 {
    600
}

// --------------------------------------------------------------------------
// RelayConfig
// --------------------------------------------------------------------------

/// Outbound SMTP relay ("smart-host") configuration.
///
/// When present, rusmes forwards all outbound mail through the specified relay
/// instead of delivering directly via DNS MX lookup.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayConfig {
    /// Required. Hostname or IP address of the upstream SMTP relay.
    pub host: String,

    /// Required. TCP port of the upstream SMTP relay (e.g. `587` or `25`).
    pub port: u16,

    /// Default: `None`. Optional SMTP AUTH username for the relay.
    #[serde(default)]
    pub username: Option<String>,

    /// Default: `None`. Optional SMTP AUTH password for the relay.
    #[serde(default)]
    pub password: Option<String>,

    /// Default: `true`. When `true`, the relay connection is secured with
    /// TLS (STARTTLS or implicit TLS depending on the relay port).
    #[serde(default = "default_use_tls")]
    pub use_tls: bool,
}

fn default_use_tls() -> bool {
    true
}

// --------------------------------------------------------------------------
// ConnectionLimitsConfig
// --------------------------------------------------------------------------

/// Connection limits configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionLimitsConfig {
    /// Maximum connections per IP address (0 = unlimited).
    #[serde(default = "default_max_connections_per_ip")]
    pub max_connections_per_ip: usize,
    /// Maximum total connections (0 = unlimited).
    #[serde(default = "default_max_total_connections")]
    pub max_total_connections: usize,
    /// Idle timeout for connections (e.g., `"300s"`, `"5m"`).
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: String,
    /// Reaper interval for cleaning up idle connections (e.g., `"60s"`, `"1m"`).
    #[serde(default = "default_reaper_interval")]
    pub reaper_interval: String,
}

impl ConnectionLimitsConfig {
    /// Parse idle timeout to seconds.
    pub fn idle_timeout_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.idle_timeout)
    }

    /// Parse reaper interval to seconds.
    pub fn reaper_interval_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.reaper_interval)
    }
}
