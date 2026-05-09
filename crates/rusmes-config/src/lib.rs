//! # rusmes-config
//!
//! Configuration management for the RusMES mail server.
//!
//! ## Overview
//!
//! `rusmes-config` provides the [`ServerConfig`] struct and supporting types that model
//! the complete runtime configuration of a RusMES installation.  Configuration is
//! normally loaded from a TOML or YAML file on disk, with optional overrides from
//! environment variables (prefix `RUSMES_`).
//!
//! ## File format auto-detection
//!
//! [`ServerConfig::from_file`] inspects the file extension:
//!
//! | Extension | Format |
//! |-----------|--------|
//! | `.toml`   | TOML   |
//! | `.yaml` / `.yml` | YAML |
//!
//! Both formats expose identical semantics; see the crate tests for concrete examples.
//!
//! ## Environment variable overrides
//!
//! Every significant configuration key has a corresponding `RUSMES_*` environment
//! variable that takes precedence over the file value.  A full list is documented on
//! [`ServerConfig::apply_env_overrides`].  This enables twelve-factor-style deployments
//! where the base config is baked into a container image and secrets are injected at
//! runtime.
//!
//! ## Sections
//!
//! | Struct | Field | Description |
//! |--------|-------|-------------|
//! | [`SmtpServerConfig`] | `smtp` | Listening addresses, TLS ports, rate limits |
//! | [`ImapServerConfig`] | `imap` | IMAP4rev1 listener |
//! | [`JmapServerConfig`] | `jmap` | JMAP HTTP listener |
//! | [`Pop3ServerConfig`] | `pop3` | POP3 listener |
//! | [`StorageConfig`] | `storage` | Filesystem, Postgres, or AmateRS backend |
//! | [`AuthConfig`] | `auth` | File, LDAP, SQL, or OAuth2 auth backend config |
//! | [`QueueConfig`] | `queue` | Retry queue with exponential back-off |
//! | [`SecurityConfig`] | `security` | Relay networks, blocked IPs |
//! | [`DomainsConfig`] | `domains` | Local domains and address aliases |
//! | [`MetricsConfig`] | `metrics` | Prometheus scrape endpoint |
//! | [`TracingConfig`] | `tracing` | OpenTelemetry OTLP exporter |
//! | [`ConnectionLimitsConfig`] | `connection_limits` | Per-IP and global connection caps |
//! | [`LoggingConfig`] | `logging` | Log level / format / output routing |
//!
//! The [`logging`] module provides [`logging::init_logging`] for initialising the global
//! `tracing` subscriber from a [`logging::LogConfig`], including file rotation and optional
//! gzip compression of rotated files.
//!
//! ## Validation
//!
//! [`ServerConfig::validate`] is called automatically during [`ServerConfig::from_file`].
//! It checks domain syntax, email addresses, port numbers, storage path accessibility,
//! and processor uniqueness.
//!
//! ## Example
//!
//! ```rust,no_run
//! use rusmes_config::ServerConfig;
//!
//! let cfg = ServerConfig::from_file("/etc/rusmes/rusmes.toml")?;
//! println!("Serving domain {}", cfg.domain);
//! # Ok::<(), anyhow::Error>(())
//! ```

mod env_overrides;
mod listeners;
pub mod logging;
mod parse;
pub mod performance;
mod runtime;
pub mod tls;
mod unknown_keys;
mod validation;

use rusmes_proto::MailAddress;
use serde::{Deserialize, Serialize};
use std::path::Path;
use unknown_keys::collect_unknown_toml_keys;
use validation::{
    validate_domain, validate_email, validate_port, validate_processors, validate_storage_path,
};

// Re-export all public types from sub-modules so downstream crates see no change.
pub use listeners::{
    ConnectionLimitsConfig, ImapServerConfig, JmapPushConfig, JmapServerConfig, Pop3ServerConfig,
    RateLimitConfig, RelayConfig, SmtpOutboundConfig, SmtpServerConfig,
};
pub use performance::PerformanceConfig;
pub use runtime::{
    AuthConfig, DomainsConfig, FileAuthConfig, LdapAuthConfig, LogFileConfig, LoggingConfig,
    MailetConfig, MetricsBasicAuthConfig, MetricsConfig, OAuth2AuthConfig, OtlpProtocol,
    ProcessorConfig, QueueConfig, SecurityConfig, SqlAuthConfig, StorageConfig, TracingConfig,
};
pub use tls::{ClientAuthMode, ProtocolKind, TlsConfig, TlsEndpointConfig};

/// Main server configuration.
///
/// Loaded from a TOML or YAML file via [`ServerConfig::from_file`].
/// All optional sections default to `None`; required fields (`domain`,
/// `postmaster`, `smtp`, `storage`, `processors`) must be present.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    /// Required. Primary mail domain served by this RusMES installation
    /// (e.g. `"mail.example.com"`). Must be a syntactically valid domain name.
    pub domain: String,

    /// Required. RFC 5321 postmaster email address (e.g. `"postmaster@example.com"`).
    /// Used as the envelope sender for system-generated bounce messages.
    pub postmaster: String,

    /// Required. SMTP listener configuration (host, port, TLS port, size limits).
    pub smtp: SmtpServerConfig,

    /// Default: `None`. IMAP4rev1 listener configuration. When absent the IMAP
    /// service is not started.
    pub imap: Option<ImapServerConfig>,

    /// Default: `None`. JMAP HTTP listener configuration. When absent the JMAP
    /// service is not started.
    pub jmap: Option<JmapServerConfig>,

    /// Default: `None`. POP3 listener configuration. When absent the POP3
    /// service is not started.
    pub pop3: Option<Pop3ServerConfig>,

    /// Required. Mail storage backend (filesystem, PostgreSQL, or AmateRS).
    pub storage: StorageConfig,

    /// Required. Ordered list of processor chains. At least one processor
    /// named `"root"` must be present.
    pub processors: Vec<ProcessorConfig>,

    /// Default: `"/var/run/rusmes"`. Per-process runtime directory used for
    /// the PID file, the rate-limiter snapshot, and any other ephemeral state
    /// files. Must be writable by the user running `rusmes-server`.
    #[serde(default = "default_runtime_dir")]
    pub runtime_dir: String,

    /// Default: `None`. Outbound SMTP relay configuration. When absent,
    /// rusmes delivers directly via DNS MX lookup.
    #[serde(default)]
    pub relay: Option<RelayConfig>,

    /// Default: `None`. Authentication backend configuration (file, LDAP,
    /// SQL, or OAuth2). When absent the server falls back to no-auth mode.
    #[serde(default)]
    pub auth: Option<AuthConfig>,

    /// Default: `None`. Logging configuration (level, format, output, file
    /// rotation). When absent the server logs `info`-level messages to stdout
    /// in text format.
    #[serde(default)]
    pub logging: Option<LoggingConfig>,

    /// Default: `None`. Outbound queue configuration (retry delays, back-off).
    /// When absent, reasonable built-in defaults are used.
    #[serde(default)]
    pub queue: Option<QueueConfig>,

    /// Default: `None`. Security configuration (relay networks, blocked IPs,
    /// recipient validation). When absent all security checks are disabled.
    #[serde(default)]
    pub security: Option<SecurityConfig>,

    /// Default: `None`. Local domain and alias mapping configuration.
    /// When absent only the primary `domain` is considered local.
    #[serde(default)]
    pub domains: Option<DomainsConfig>,

    /// Default: `None`. Prometheus metrics endpoint configuration.
    /// When absent the `/metrics` endpoint is not exposed.
    #[serde(default)]
    pub metrics: Option<MetricsConfig>,

    /// Default: `None`. OpenTelemetry OTLP tracing configuration.
    /// When absent distributed tracing is disabled.
    #[serde(default)]
    pub tracing: Option<TracingConfig>,

    /// Default: `None`. Per-IP and global connection limit configuration.
    /// When absent no connection caps are enforced.
    #[serde(default)]
    pub connection_limits: Option<ConnectionLimitsConfig>,

    /// Default: `PerformanceConfig::default()`. Runtime performance tuning:
    /// Tokio worker threads, connection pool sizes, and per-connection buffer
    /// sizes. Omitting `[performance]` uses conservative built-in defaults.
    #[serde(default)]
    pub performance: PerformanceConfig,

    /// Default: `None`. TLS certificate and key paths. Supports a shared
    /// `[tls.default]` endpoint and optional per-protocol overrides
    /// (`[tls.smtp]`, `[tls.imap]`, `[tls.pop3]`, `[tls.jmap]`).
    #[serde(default)]
    pub tls: Option<TlsConfig>,

    /// Default: `false`. When `true`, call `chroot(runtime_dir)` after binding
    /// all sockets and loading TLS material, before dropping privileges.
    /// Has effect only on Linux; on other platforms a `tracing::warn!` is
    /// emitted and this field is otherwise ignored.
    #[serde(default)]
    pub chroot: bool,

    /// Default: `""` (no-op). System user name to `setuid` to after binding
    /// all sockets.  The empty string means "do not change UID".  Only
    /// effective on Linux; ignored on other platforms (with a warning).
    #[serde(default)]
    pub run_as_user: String,

    /// Default: `""` (no-op). System group name to `setgid` to after binding
    /// all sockets.  The empty string means "do not change GID".  Only
    /// effective on Linux; ignored on other platforms (with a warning).
    #[serde(default)]
    pub run_as_group: String,

    /// Unknown TOML/YAML keys captured for diagnostic warnings.
    ///
    /// Not serialized to output. Populated by [`ServerConfig::from_file`]
    /// via a two-phase parse (raw `toml::Value` → known-key diff) so that
    /// `warn_unknown_keys` can emit [`tracing::warn!`] for each entry.
    /// Exposed as `pub` so tests can assert on which keys were captured
    /// without relying on subscriber interception.
    #[serde(skip)]
    pub extra: Vec<String>,
}

/// Default runtime directory used when `runtime_dir` is omitted from the
/// configuration file. This path is conventionally writable by the user
/// running `rusmes-server`.
fn default_runtime_dir() -> String {
    "/var/run/rusmes".to_string()
}

impl ServerConfig {
    /// Load configuration from a TOML or YAML file.
    ///
    /// The format is auto-detected based on file extension:
    /// - `.toml` files are parsed as TOML
    /// - `.yaml` or `.yml` files are parsed as YAML
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;

        // Auto-detect format based on file extension
        let mut config: ServerConfig = match path.extension().and_then(|ext| ext.to_str()) {
            Some("yaml") | Some("yml") => serde_yaml::from_str(&content)?,
            Some("toml") => {
                // Two-phase: first parse to raw Value so we can detect
                // unknown top-level keys, then deserialize into the struct.
                let raw: toml::Value = toml::from_str(&content)?;
                let unknown = collect_unknown_toml_keys(&raw);
                let mut cfg: ServerConfig = toml::from_str(&content)?;
                cfg.extra = unknown;
                cfg
            }
            Some(ext) => {
                return Err(anyhow::anyhow!(
                    "Unsupported configuration file extension: .{}. Use .toml, .yaml, or .yml",
                    ext
                ));
            }
            None => {
                return Err(anyhow::anyhow!(
                    "Configuration file must have a .toml, .yaml, or .yml extension"
                ));
            }
        };

        // Apply environment variable overrides
        config.apply_env_overrides();

        // Warn about unknown keys before validation so operators get actionable
        // output even when validation subsequently fails.
        config.warn_unknown_keys();

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Validate the entire configuration.
    ///
    /// This method is called automatically when loading configuration from a file.
    /// It validates:
    /// - Domain name format
    /// - Postmaster email address
    /// - Port numbers for SMTP, IMAP, JMAP
    /// - Storage path accessibility
    /// - Processor uniqueness
    /// - Local domain names (if configured)
    pub fn validate(&self) -> anyhow::Result<()> {
        // Validate main domain
        validate_domain(&self.domain)
            .map_err(|e| anyhow::anyhow!("Invalid server domain: {}", e))?;

        // Validate postmaster email
        validate_email(&self.postmaster)
            .map_err(|e| anyhow::anyhow!("Invalid postmaster email: {}", e))?;

        // Validate SMTP configuration
        validate_port(self.smtp.port, "SMTP port")?;
        if let Some(tls_port) = self.smtp.tls_port {
            validate_port(tls_port, "SMTP TLS port")?;
        }

        // Validate IMAP configuration
        if let Some(ref imap) = self.imap {
            validate_port(imap.port, "IMAP port")?;
            if let Some(tls_port) = imap.tls_port {
                validate_port(tls_port, "IMAP TLS port")?;
            }
        }

        // Validate JMAP configuration
        if let Some(ref jmap) = self.jmap {
            validate_port(jmap.port, "JMAP port")?;
        }

        // Validate POP3 configuration
        if let Some(ref pop3) = self.pop3 {
            validate_port(pop3.port, "POP3 port")?;
            if let Some(tls_port) = pop3.tls_port {
                validate_port(tls_port, "POP3 TLS port")?;
            }
        }

        // Validate storage path
        match &self.storage {
            StorageConfig::Filesystem { path } => {
                validate_storage_path(path)?;
            }
            StorageConfig::Postgres { connection_string } => {
                if connection_string.is_empty() {
                    anyhow::bail!("Postgres connection string cannot be empty");
                }
            }
            StorageConfig::AmateRS {
                endpoints,
                replication_factor,
            } => {
                if endpoints.is_empty() {
                    anyhow::bail!("AmateRS endpoints cannot be empty");
                }
                if *replication_factor == 0 {
                    anyhow::bail!("AmateRS replication factor must be greater than 0");
                }
            }
        }

        // Validate processors
        validate_processors(&self.processors)?;

        // Validate local domains if configured
        if let Some(ref domains) = self.domains {
            for domain in &domains.local_domains {
                validate_domain(domain)
                    .map_err(|e| anyhow::anyhow!("Invalid local domain '{}': {}", domain, e))?;
            }

            // Validate aliases
            for (from, to) in &domains.aliases {
                validate_email(from)
                    .map_err(|e| anyhow::anyhow!("Invalid alias source '{}': {}", from, e))?;
                validate_email(to)
                    .map_err(|e| anyhow::anyhow!("Invalid alias destination '{}': {}", to, e))?;
            }
        }

        // Validate logging configuration
        if let Some(ref logging) = self.logging {
            logging.validate_level()?;
            logging.validate_format()?;
        }

        // Validate queue configuration
        if let Some(ref queue) = self.queue {
            queue.validate_backoff_multiplier()?;
            queue.validate_worker_threads()?;
        }

        // Validate security configuration
        if let Some(ref security) = self.security {
            security.validate_relay_networks()?;
            security.validate_blocked_ips()?;
        }

        // Validate metrics configuration
        if let Some(ref metrics) = self.metrics {
            metrics.validate_bind_address()?;
            metrics.validate_path()?;
        }

        // Validate performance configuration
        self.performance.validate()?;

        // Validate TLS configuration (if present)
        if let Some(ref tls) = self.tls {
            tls.validate()?;
        }

        Ok(())
    }

    /// Get postmaster address.
    pub fn postmaster_address(&self) -> anyhow::Result<MailAddress> {
        self.postmaster
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid postmaster address: {}", e))
    }

    /// Return the [`TlsEndpointConfig`] for `proto`, or `None` if no TLS is
    /// configured.
    ///
    /// Delegates to [`TlsConfig::tls_for_protocol`] which returns the
    /// per-protocol override when present and falls back to `tls.default`.
    pub fn tls_for_protocol(&self, proto: ProtocolKind) -> Option<&TlsEndpointConfig> {
        self.tls.as_ref().map(|t| t.tls_for_protocol(proto))
    }

    /// Emit `tracing::warn!` for every unknown top-level configuration key.
    ///
    /// Called automatically by [`ServerConfig::from_file`] after
    /// deserialization. Operators can use the warnings to detect typos or
    /// stale keys without causing a hard failure.
    pub fn warn_unknown_keys(&self) {
        for key in &self.extra {
            tracing::warn!(
                "unknown configuration key '{}' will be ignored; check your config file for typos",
                key
            );
        }
    }
}
