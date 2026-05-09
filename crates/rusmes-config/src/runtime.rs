//! Runtime configuration types: storage, auth, processors, logging, queue,
//! security, domains, metrics, tracing, and observability settings.

use crate::parse::{parse_duration, parse_size};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// --------------------------------------------------------------------------
// StorageConfig
// --------------------------------------------------------------------------

/// Storage backend configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "backend")]
pub enum StorageConfig {
    #[serde(rename = "filesystem")]
    Filesystem { path: String },
    #[serde(rename = "postgres")]
    Postgres { connection_string: String },
    #[serde(rename = "amaters")]
    AmateRS {
        endpoints: Vec<String>,
        replication_factor: usize,
    },
}

// --------------------------------------------------------------------------
// ProcessorConfig / MailetConfig
// --------------------------------------------------------------------------

/// A named processor chain containing an ordered list of mailets.
///
/// Processors are the top-level mail-processing pipeline stages. At least one
/// processor with `state = "root"` must be present.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessorConfig {
    /// Required. Unique name for this processor chain (e.g. `"root"`,
    /// `"spam"`, `"virus"`).
    pub name: String,

    /// Required. State label used to route messages into this chain
    /// (e.g. `"root"`, `"transport"`).
    pub state: String,

    /// Required. Ordered list of mailet rules applied to each message
    /// entering this processor.
    pub mailets: Vec<MailetConfig>,
}

/// A single matcher + mailet rule within a processor chain.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MailetConfig {
    /// Required. Name of the matcher that selects messages for this mailet
    /// (e.g. `"All"`, `"RecipientIsLocal"`, `"HasHeader=X-Spam-Flag,YES"`).
    pub matcher: String,

    /// Required. Name of the mailet to execute on matching messages
    /// (e.g. `"LocalDelivery"`, `"RemoteDelivery"`, `"Null"`).
    pub mailet: String,

    /// Default: `{}`. Arbitrary key-value parameters passed to the mailet
    /// at initialization time.
    #[serde(default)]
    pub params: HashMap<String, String>,
}

// --------------------------------------------------------------------------
// AuthConfig and variants
// --------------------------------------------------------------------------

/// Authentication backend configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "backend")]
pub enum AuthConfig {
    #[serde(rename = "file")]
    File {
        #[serde(flatten)]
        config: FileAuthConfig,
    },
    #[serde(rename = "ldap")]
    Ldap {
        #[serde(flatten)]
        config: LdapAuthConfig,
    },
    #[serde(rename = "sql")]
    Sql {
        #[serde(flatten)]
        config: SqlAuthConfig,
    },
    #[serde(rename = "oauth2")]
    OAuth2 {
        #[serde(flatten)]
        config: OAuth2AuthConfig,
    },
}

/// File-based authentication configuration.
///
/// `hash_algorithm` selects the password-hashing algorithm used for **new**
/// password writes (`create_user`, `change_password`). Existing hashes
/// continue to verify regardless of this setting (auto-detected by their
/// PHC prefix). Accepted values: `"bcrypt"` (default) or `"argon2"` /
/// `"argon2id"`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileAuthConfig {
    pub path: String,
    /// Optional algorithm name; defaults to `"bcrypt"` when omitted for
    /// backwards compatibility with existing on-disk configurations.
    #[serde(default = "default_hash_algorithm")]
    pub hash_algorithm: String,
}

fn default_hash_algorithm() -> String {
    "bcrypt".to_string()
}

/// LDAP authentication configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LdapAuthConfig {
    pub url: String,
    pub base_dn: String,
    pub bind_dn: String,
    pub bind_password: String,
    pub user_filter: String,
}

/// SQL authentication configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SqlAuthConfig {
    pub connection_string: String,
    pub query: String,
}

/// OAuth2 authentication configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuth2AuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub token_url: String,
    pub authorization_url: String,
}

// --------------------------------------------------------------------------
// LoggingConfig / LogFileConfig
// --------------------------------------------------------------------------

/// Structured logging configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    /// Required. Minimum log level to emit. Valid values: `"trace"`,
    /// `"debug"`, `"info"`, `"warn"`, `"error"`.
    pub level: String,

    /// Required. Log output format. Valid values: `"text"` (human-readable),
    /// `"json"` (structured JSON for log aggregators).
    pub format: String,

    /// Required. Log output destination. Valid values: `"stdout"`, `"stderr"`,
    /// or an absolute file path for file-based output.
    pub output: String,

    /// Default: `None`. Log file rotation settings. Only meaningful when
    /// `output` is a file path; ignored for `"stdout"` / `"stderr"`.
    #[serde(default)]
    pub file: Option<LogFileConfig>,
}

impl LoggingConfig {
    /// Validate log level.
    pub fn validate_level(&self) -> anyhow::Result<()> {
        match self.level.as_str() {
            "trace" | "debug" | "info" | "warn" | "error" => Ok(()),
            _ => Err(anyhow::anyhow!("Invalid log level: {}", self.level)),
        }
    }

    /// Validate log format.
    pub fn validate_format(&self) -> anyhow::Result<()> {
        match self.format.as_str() {
            "json" | "text" => Ok(()),
            _ => Err(anyhow::anyhow!("Invalid log format: {}", self.format)),
        }
    }
}

/// Log file rotation configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogFileConfig {
    /// Required. Absolute path to the log file (e.g. `"/var/log/rusmes/server.log"`).
    pub path: String,

    /// Required. Maximum log file size before rotation, expressed as a
    /// human-readable string (e.g. `"100MB"`, `"1GB"`).
    pub max_size: String,

    /// Required. Number of rotated log backup files to retain. Older files
    /// beyond this limit are deleted automatically.
    pub max_backups: u32,

    /// Required. When `true`, rotated log files are compressed using deflate
    /// to reduce disk usage.
    pub compress: bool,
}

impl LogFileConfig {
    /// Parse max file size to bytes.
    pub fn max_size_bytes(&self) -> anyhow::Result<usize> {
        parse_size(&self.max_size)
    }
}

// --------------------------------------------------------------------------
// QueueConfig
// --------------------------------------------------------------------------

/// Outbound mail queue and retry configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QueueConfig {
    /// Required. Initial retry delay after the first failed delivery attempt,
    /// expressed as a human-readable duration (e.g. `"60s"`, `"1m"`).
    pub initial_delay: String,

    /// Required. Maximum retry delay after many consecutive failures,
    /// expressed as a human-readable duration (e.g. `"3600s"`, `"1h"`).
    pub max_delay: String,

    /// Required. Exponential back-off multiplier applied between retries.
    /// Must be positive. A value of `2.0` doubles the delay after each
    /// failure up to `max_delay`.
    pub backoff_multiplier: f64,

    /// Required. Maximum number of delivery attempts before the message is
    /// bounced back to the sender.
    pub max_attempts: u32,

    /// Required. Number of threads in the queue worker pool. Must be `> 0`.
    pub worker_threads: usize,

    /// Required. Maximum number of messages to dequeue and attempt in a
    /// single batch. Larger values increase throughput at the cost of latency.
    pub batch_size: usize,
}

impl QueueConfig {
    /// Parse initial delay to seconds.
    pub fn initial_delay_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.initial_delay)
    }

    /// Parse max delay to seconds.
    pub fn max_delay_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.max_delay)
    }

    /// Validate backoff multiplier.
    pub fn validate_backoff_multiplier(&self) -> anyhow::Result<()> {
        if self.backoff_multiplier <= 0.0 {
            return Err(anyhow::anyhow!("backoff_multiplier must be positive"));
        }
        Ok(())
    }

    /// Validate worker threads.
    pub fn validate_worker_threads(&self) -> anyhow::Result<()> {
        if self.worker_threads == 0 {
            return Err(anyhow::anyhow!("worker_threads must be greater than 0"));
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------
// SecurityConfig
// --------------------------------------------------------------------------

/// Inbound relay and IP-filtering security configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    /// Required. List of CIDR network ranges whose senders are allowed to
    /// relay mail through this server without authentication
    /// (e.g. `["127.0.0.0/8", "10.0.0.0/8"]`).
    pub relay_networks: Vec<String>,

    /// Required. List of IP addresses that are unconditionally blocked from
    /// connecting. Both IPv4 and IPv6 addresses are accepted.
    pub blocked_ips: Vec<String>,

    /// Required. When `true`, incoming mail is checked to verify the recipient
    /// mailbox exists before accepting the message.
    pub check_recipient_exists: bool,

    /// Required. When `true`, connections from senders whose reverse DNS
    /// lookup fails or does not match are rejected.
    pub reject_unknown_recipients: bool,
}

impl SecurityConfig {
    /// Validate CIDR notation for relay networks.
    pub fn validate_relay_networks(&self) -> anyhow::Result<()> {
        for network in &self.relay_networks {
            // Basic validation - should contain a slash for CIDR notation
            if !network.contains('/') {
                return Err(anyhow::anyhow!("Invalid CIDR notation: {}", network));
            }
        }
        Ok(())
    }

    /// Validate IP addresses in blocked list.
    pub fn validate_blocked_ips(&self) -> anyhow::Result<()> {
        for ip in &self.blocked_ips {
            // Basic validation - should contain dots (IPv4) or colons (IPv6)
            if !ip.contains('.') && !ip.contains(':') {
                return Err(anyhow::anyhow!("Invalid IP address: {}", ip));
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------
// DomainsConfig
// --------------------------------------------------------------------------

/// Local domain and address alias configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DomainsConfig {
    /// Required. List of domain names for which this server accepts mail as
    /// the final destination (e.g. `["example.com", "mail.example.com"]`).
    pub local_domains: Vec<String>,

    /// Default: `{}`. Mapping of source email address to destination email
    /// address for simple address rewriting (e.g.
    /// `"abuse@example.com" = "postmaster@example.com"`).
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

impl DomainsConfig {
    /// Validate domain names.
    pub fn validate_local_domains(&self) -> anyhow::Result<()> {
        for domain in &self.local_domains {
            if domain.is_empty() {
                return Err(anyhow::anyhow!("Domain name cannot be empty"));
            }
            // Basic validation - should contain at least one dot
            if !domain.contains('.') {
                return Err(anyhow::anyhow!("Invalid domain name: {}", domain));
            }
        }
        Ok(())
    }

    /// Validate alias email addresses.
    pub fn validate_aliases(&self) -> anyhow::Result<()> {
        for (from, to) in &self.aliases {
            if !from.contains('@') {
                return Err(anyhow::anyhow!("Invalid alias source: {}", from));
            }
            if !to.contains('@') {
                return Err(anyhow::anyhow!("Invalid alias destination: {}", to));
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------
// MetricsConfig / MetricsBasicAuthConfig
// --------------------------------------------------------------------------

/// Prometheus metrics scrape endpoint configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetricsConfig {
    /// Required. When `true`, the metrics HTTP endpoint is started on
    /// `bind_address`.
    pub enabled: bool,

    /// Required. Socket address on which the metrics HTTP server listens
    /// (e.g. `"0.0.0.0:9090"`). Must contain a colon separating host and port.
    pub bind_address: String,

    /// Required. URL path at which Prometheus can scrape metrics
    /// (e.g. `"/metrics"`). Must start with `'/'`.
    pub path: String,

    /// Optional HTTP Basic auth on the scrape endpoint.
    ///
    /// When present, the metrics handler verifies the `Authorization: Basic`
    /// header against the configured bcrypt hash (RFC 7617). Returns 401 on
    /// missing/invalid credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_auth: Option<MetricsBasicAuthConfig>,
}

/// Optional HTTP Basic authentication for the metrics scrape endpoint.
///
/// The password is stored as a bcrypt hash (RFC 7617 + bcrypt §3) so the
/// plaintext password never lives at rest. Use `bcrypt::hash(password,
/// bcrypt::DEFAULT_COST)` to generate or `htpasswd -B -n username` from a
/// shell.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetricsBasicAuthConfig {
    /// Required username.
    pub username: String,
    /// bcrypt-hashed password.
    pub password_hash: String,
}

impl MetricsConfig {
    /// Validate bind address format.
    pub fn validate_bind_address(&self) -> anyhow::Result<()> {
        if !self.bind_address.contains(':') {
            return Err(anyhow::anyhow!(
                "Invalid bind address format: {}",
                self.bind_address
            ));
        }
        Ok(())
    }

    /// Validate path format.
    pub fn validate_path(&self) -> anyhow::Result<()> {
        if !self.path.starts_with('/') {
            return Err(anyhow::anyhow!(
                "Metrics path must start with '/': {}",
                self.path
            ));
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------
// TracingConfig / OtlpProtocol
// --------------------------------------------------------------------------

/// OpenTelemetry OTLP distributed tracing configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TracingConfig {
    /// Required. When `true`, span data is exported to the configured
    /// OTLP `endpoint`.
    pub enabled: bool,

    /// Required. OTLP exporter endpoint URL (e.g. `"http://localhost:4317"`
    /// for gRPC or `"http://localhost:4318"` for HTTP). Must start with
    /// `http://` or `https://`.
    pub endpoint: String,

    /// Required. OTLP transport protocol. Valid values: `"grpc"`, `"http"`.
    pub protocol: OtlpProtocol,

    /// Required. Service name recorded on every emitted span.
    pub service_name: String,

    /// Default: `1.0`. Fraction of traces to sample, in the range `0.0`
    /// (no traces) to `1.0` (all traces).
    #[serde(default)]
    pub sample_ratio: f64,
}

/// OTLP protocol type.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OtlpProtocol {
    Grpc,
    Http,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:4317".to_string(),
            protocol: OtlpProtocol::Grpc,
            service_name: "rusmes".to_string(),
            sample_ratio: 1.0,
        }
    }
}

impl TracingConfig {
    /// Validate endpoint URL format.
    pub fn validate_endpoint(&self) -> anyhow::Result<()> {
        if !self.endpoint.starts_with("http://") && !self.endpoint.starts_with("https://") {
            return Err(anyhow::anyhow!(
                "Endpoint must start with http:// or https://: {}",
                self.endpoint
            ));
        }
        Ok(())
    }

    /// Validate sample ratio.
    pub fn validate_sample_ratio(&self) -> anyhow::Result<()> {
        if !(0.0..=1.0).contains(&self.sample_ratio) {
            return Err(anyhow::anyhow!(
                "Sample ratio must be between 0.0 and 1.0: {}",
                self.sample_ratio
            ));
        }
        Ok(())
    }

    /// Validate service name.
    pub fn validate_service_name(&self) -> anyhow::Result<()> {
        if self.service_name.trim().is_empty() {
            return Err(anyhow::anyhow!("Service name cannot be empty"));
        }
        Ok(())
    }
}
