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

pub mod logging;
mod validation;

use rusmes_proto::MailAddress;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use validation::{
    validate_domain, validate_email, validate_port, validate_processors, validate_storage_path,
};

/// Main server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub domain: String,
    pub postmaster: String,
    pub smtp: SmtpServerConfig,
    pub imap: Option<ImapServerConfig>,
    pub jmap: Option<JmapServerConfig>,
    pub pop3: Option<Pop3ServerConfig>,
    pub storage: StorageConfig,
    pub processors: Vec<ProcessorConfig>,
    #[serde(default)]
    pub relay: Option<RelayConfig>,
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub logging: Option<LoggingConfig>,
    #[serde(default)]
    pub queue: Option<QueueConfig>,
    #[serde(default)]
    pub security: Option<SecurityConfig>,
    #[serde(default)]
    pub domains: Option<DomainsConfig>,
    #[serde(default)]
    pub metrics: Option<MetricsConfig>,
    #[serde(default)]
    pub tracing: Option<TracingConfig>,
    #[serde(default)]
    pub connection_limits: Option<ConnectionLimitsConfig>,
}

impl ServerConfig {
    /// Load configuration from a TOML or YAML file
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
            Some("toml") => toml::from_str(&content)?,
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

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Apply environment variable overrides to configuration
    ///
    /// Environment variables follow the convention RUSMES_SECTION_KEY.
    /// Priority: env vars > config file > defaults
    ///
    /// Supported environment variables:
    /// - RUSMES_DOMAIN
    /// - RUSMES_POSTMASTER
    /// - RUSMES_SMTP_HOST
    /// - RUSMES_SMTP_PORT
    /// - RUSMES_SMTP_TLS_PORT
    /// - RUSMES_SMTP_MAX_MESSAGE_SIZE
    /// - RUSMES_SMTP_REQUIRE_AUTH
    /// - RUSMES_SMTP_ENABLE_STARTTLS
    /// - RUSMES_SMTP_RATE_LIMIT_MAX_CONNECTIONS_PER_IP
    /// - RUSMES_SMTP_RATE_LIMIT_MAX_MESSAGES_PER_HOUR
    /// - RUSMES_SMTP_RATE_LIMIT_WINDOW_DURATION
    /// - RUSMES_IMAP_HOST
    /// - RUSMES_IMAP_PORT
    /// - RUSMES_IMAP_TLS_PORT
    /// - RUSMES_JMAP_HOST
    /// - RUSMES_JMAP_PORT
    /// - RUSMES_JMAP_BASE_URL
    /// - RUSMES_STORAGE_PATH (for filesystem backend)
    /// - RUSMES_LOG_LEVEL
    /// - RUSMES_LOG_FORMAT
    /// - RUSMES_LOG_OUTPUT
    /// - RUSMES_QUEUE_INITIAL_DELAY
    /// - RUSMES_QUEUE_MAX_DELAY
    /// - RUSMES_QUEUE_BACKOFF_MULTIPLIER
    /// - RUSMES_QUEUE_MAX_ATTEMPTS
    /// - RUSMES_QUEUE_WORKER_THREADS
    /// - RUSMES_QUEUE_BATCH_SIZE
    /// - RUSMES_METRICS_ENABLED
    /// - RUSMES_METRICS_BIND_ADDRESS
    /// - RUSMES_METRICS_PATH
    /// - RUSMES_TRACING_ENABLED
    /// - RUSMES_TRACING_ENDPOINT
    /// - RUSMES_TRACING_PROTOCOL (grpc or http)
    /// - RUSMES_TRACING_SERVICE_NAME
    /// - RUSMES_TRACING_SAMPLE_RATIO
    /// - RUSMES_CONNECTION_LIMITS_MAX_CONNECTIONS_PER_IP
    /// - RUSMES_CONNECTION_LIMITS_MAX_TOTAL_CONNECTIONS
    /// - RUSMES_CONNECTION_LIMITS_IDLE_TIMEOUT
    /// - RUSMES_CONNECTION_LIMITS_REAPER_INTERVAL
    pub fn apply_env_overrides(&mut self) {
        // Top-level fields
        if let Ok(val) = std::env::var("RUSMES_DOMAIN") {
            self.domain = val;
        }
        if let Ok(val) = std::env::var("RUSMES_POSTMASTER") {
            self.postmaster = val;
        }

        // SMTP configuration
        if let Ok(val) = std::env::var("RUSMES_SMTP_HOST") {
            self.smtp.host = val;
        }
        if let Ok(val) = std::env::var("RUSMES_SMTP_PORT") {
            if let Ok(port) = val.parse::<u16>() {
                self.smtp.port = port;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_SMTP_TLS_PORT") {
            if let Ok(port) = val.parse::<u16>() {
                self.smtp.tls_port = Some(port);
            }
        }
        if let Ok(val) = std::env::var("RUSMES_SMTP_MAX_MESSAGE_SIZE") {
            self.smtp.max_message_size = val;
        }
        if let Ok(val) = std::env::var("RUSMES_SMTP_REQUIRE_AUTH") {
            if let Ok(b) = val.parse::<bool>() {
                self.smtp.require_auth = b;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_SMTP_ENABLE_STARTTLS") {
            if let Ok(b) = val.parse::<bool>() {
                self.smtp.enable_starttls = b;
            }
        }

        // SMTP rate limit configuration
        let has_rate_limit_max =
            std::env::var("RUSMES_SMTP_RATE_LIMIT_MAX_MESSAGES_PER_HOUR").is_ok();
        let has_rate_limit_window = std::env::var("RUSMES_SMTP_RATE_LIMIT_WINDOW_DURATION").is_ok();
        let has_rate_limit_max_conn =
            std::env::var("RUSMES_SMTP_RATE_LIMIT_MAX_CONNECTIONS_PER_IP").is_ok();

        if has_rate_limit_max || has_rate_limit_window || has_rate_limit_max_conn {
            // Create rate limit config if it doesn't exist
            if self.smtp.rate_limit.is_none() {
                self.smtp.rate_limit = Some(RateLimitConfig {
                    max_connections_per_ip: 10,
                    max_messages_per_hour: 100,
                    window_duration: "1h".to_string(),
                });
            }

            if let Some(ref mut rate_limit) = self.smtp.rate_limit {
                if let Ok(val) = std::env::var("RUSMES_SMTP_RATE_LIMIT_MAX_CONNECTIONS_PER_IP") {
                    if let Ok(n) = val.parse::<usize>() {
                        rate_limit.max_connections_per_ip = n;
                    }
                }
                if let Ok(val) = std::env::var("RUSMES_SMTP_RATE_LIMIT_MAX_MESSAGES_PER_HOUR") {
                    if let Ok(n) = val.parse::<u32>() {
                        rate_limit.max_messages_per_hour = n;
                    }
                }
                if let Ok(val) = std::env::var("RUSMES_SMTP_RATE_LIMIT_WINDOW_DURATION") {
                    rate_limit.window_duration = val;
                }
            }
        }

        // IMAP configuration
        if let Ok(val) = std::env::var("RUSMES_IMAP_HOST") {
            if self.imap.is_none() {
                self.imap = Some(ImapServerConfig {
                    host: "0.0.0.0".to_string(),
                    port: 143,
                    tls_port: None,
                });
            }
            if let Some(ref mut imap) = self.imap {
                imap.host = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_IMAP_PORT") {
            if let Ok(port) = val.parse::<u16>() {
                if self.imap.is_none() {
                    self.imap = Some(ImapServerConfig {
                        host: "0.0.0.0".to_string(),
                        port,
                        tls_port: None,
                    });
                } else if let Some(ref mut imap) = self.imap {
                    imap.port = port;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_IMAP_TLS_PORT") {
            if let Ok(port) = val.parse::<u16>() {
                if self.imap.is_none() {
                    self.imap = Some(ImapServerConfig {
                        host: "0.0.0.0".to_string(),
                        port: 143,
                        tls_port: Some(port),
                    });
                } else if let Some(ref mut imap) = self.imap {
                    imap.tls_port = Some(port);
                }
            }
        }

        // JMAP configuration
        if let Ok(val) = std::env::var("RUSMES_JMAP_HOST") {
            if self.jmap.is_none() {
                self.jmap = Some(JmapServerConfig {
                    host: "0.0.0.0".to_string(),
                    port: 8080,
                    base_url: "http://localhost:8080".to_string(),
                });
            }
            if let Some(ref mut jmap) = self.jmap {
                jmap.host = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_JMAP_PORT") {
            if let Ok(port) = val.parse::<u16>() {
                if self.jmap.is_none() {
                    self.jmap = Some(JmapServerConfig {
                        host: "0.0.0.0".to_string(),
                        port,
                        base_url: "http://localhost:8080".to_string(),
                    });
                } else if let Some(ref mut jmap) = self.jmap {
                    jmap.port = port;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_JMAP_BASE_URL") {
            if self.jmap.is_none() {
                self.jmap = Some(JmapServerConfig {
                    host: "0.0.0.0".to_string(),
                    port: 8080,
                    base_url: val,
                });
            } else if let Some(ref mut jmap) = self.jmap {
                jmap.base_url = val;
            }
        }

        // Storage configuration (only filesystem backend path)
        if let Ok(val) = std::env::var("RUSMES_STORAGE_PATH") {
            if let StorageConfig::Filesystem { ref mut path } = self.storage {
                *path = val;
            }
        }

        // Logging configuration
        if let Ok(val) = std::env::var("RUSMES_LOG_LEVEL") {
            if self.logging.is_none() {
                self.logging = Some(LoggingConfig {
                    level: val,
                    format: "text".to_string(),
                    output: "stdout".to_string(),
                    file: None,
                });
            } else if let Some(ref mut logging) = self.logging {
                logging.level = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_LOG_FORMAT") {
            if self.logging.is_none() {
                self.logging = Some(LoggingConfig {
                    level: "info".to_string(),
                    format: val,
                    output: "stdout".to_string(),
                    file: None,
                });
            } else if let Some(ref mut logging) = self.logging {
                logging.format = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_LOG_OUTPUT") {
            if self.logging.is_none() {
                self.logging = Some(LoggingConfig {
                    level: "info".to_string(),
                    format: "text".to_string(),
                    output: val,
                    file: None,
                });
            } else if let Some(ref mut logging) = self.logging {
                logging.output = val;
            }
        }

        // Queue configuration
        if let Ok(val) = std::env::var("RUSMES_QUEUE_INITIAL_DELAY") {
            if self.queue.is_none() {
                self.queue = Some(QueueConfig {
                    initial_delay: val,
                    max_delay: "3600s".to_string(),
                    backoff_multiplier: 2.0,
                    max_attempts: 5,
                    worker_threads: 4,
                    batch_size: 100,
                });
            } else if let Some(ref mut queue) = self.queue {
                queue.initial_delay = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_QUEUE_MAX_DELAY") {
            if self.queue.is_none() {
                self.queue = Some(QueueConfig {
                    initial_delay: "60s".to_string(),
                    max_delay: val,
                    backoff_multiplier: 2.0,
                    max_attempts: 5,
                    worker_threads: 4,
                    batch_size: 100,
                });
            } else if let Some(ref mut queue) = self.queue {
                queue.max_delay = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_QUEUE_BACKOFF_MULTIPLIER") {
            if let Ok(multiplier) = val.parse::<f64>() {
                if self.queue.is_none() {
                    self.queue = Some(QueueConfig {
                        initial_delay: "60s".to_string(),
                        max_delay: "3600s".to_string(),
                        backoff_multiplier: multiplier,
                        max_attempts: 5,
                        worker_threads: 4,
                        batch_size: 100,
                    });
                } else if let Some(ref mut queue) = self.queue {
                    queue.backoff_multiplier = multiplier;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_QUEUE_MAX_ATTEMPTS") {
            if let Ok(attempts) = val.parse::<u32>() {
                if self.queue.is_none() {
                    self.queue = Some(QueueConfig {
                        initial_delay: "60s".to_string(),
                        max_delay: "3600s".to_string(),
                        backoff_multiplier: 2.0,
                        max_attempts: attempts,
                        worker_threads: 4,
                        batch_size: 100,
                    });
                } else if let Some(ref mut queue) = self.queue {
                    queue.max_attempts = attempts;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_QUEUE_WORKER_THREADS") {
            if let Ok(threads) = val.parse::<usize>() {
                if self.queue.is_none() {
                    self.queue = Some(QueueConfig {
                        initial_delay: "60s".to_string(),
                        max_delay: "3600s".to_string(),
                        backoff_multiplier: 2.0,
                        max_attempts: 5,
                        worker_threads: threads,
                        batch_size: 100,
                    });
                } else if let Some(ref mut queue) = self.queue {
                    queue.worker_threads = threads;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_QUEUE_BATCH_SIZE") {
            if let Ok(batch_size) = val.parse::<usize>() {
                if self.queue.is_none() {
                    self.queue = Some(QueueConfig {
                        initial_delay: "60s".to_string(),
                        max_delay: "3600s".to_string(),
                        backoff_multiplier: 2.0,
                        max_attempts: 5,
                        worker_threads: 4,
                        batch_size,
                    });
                } else if let Some(ref mut queue) = self.queue {
                    queue.batch_size = batch_size;
                }
            }
        }

        // Metrics configuration
        if let Ok(val) = std::env::var("RUSMES_METRICS_ENABLED") {
            if let Ok(enabled) = val.parse::<bool>() {
                if self.metrics.is_none() {
                    self.metrics = Some(MetricsConfig {
                        enabled,
                        bind_address: "0.0.0.0:9090".to_string(),
                        path: "/metrics".to_string(),
                    });
                } else if let Some(ref mut metrics) = self.metrics {
                    metrics.enabled = enabled;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_METRICS_BIND_ADDRESS") {
            if self.metrics.is_none() {
                self.metrics = Some(MetricsConfig {
                    enabled: true,
                    bind_address: val,
                    path: "/metrics".to_string(),
                });
            } else if let Some(ref mut metrics) = self.metrics {
                metrics.bind_address = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_METRICS_PATH") {
            if self.metrics.is_none() {
                self.metrics = Some(MetricsConfig {
                    enabled: true,
                    bind_address: "0.0.0.0:9090".to_string(),
                    path: val,
                });
            } else if let Some(ref mut metrics) = self.metrics {
                metrics.path = val;
            }
        }

        // Tracing configuration
        if let Ok(val) = std::env::var("RUSMES_TRACING_ENABLED") {
            if let Ok(enabled) = val.parse::<bool>() {
                if self.tracing.is_none() {
                    self.tracing = Some(TracingConfig {
                        enabled,
                        ..Default::default()
                    });
                } else if let Some(ref mut tracing) = self.tracing {
                    tracing.enabled = enabled;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_TRACING_ENDPOINT") {
            if self.tracing.is_none() {
                self.tracing = Some(TracingConfig {
                    enabled: true,
                    endpoint: val,
                    ..Default::default()
                });
            } else if let Some(ref mut tracing) = self.tracing {
                tracing.endpoint = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_TRACING_PROTOCOL") {
            let protocol = match val.to_lowercase().as_str() {
                "grpc" => OtlpProtocol::Grpc,
                "http" => OtlpProtocol::Http,
                _ => OtlpProtocol::Grpc,
            };
            if self.tracing.is_none() {
                self.tracing = Some(TracingConfig {
                    enabled: true,
                    protocol,
                    ..Default::default()
                });
            } else if let Some(ref mut tracing) = self.tracing {
                tracing.protocol = protocol;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_TRACING_SERVICE_NAME") {
            if self.tracing.is_none() {
                self.tracing = Some(TracingConfig {
                    enabled: true,
                    service_name: val,
                    ..Default::default()
                });
            } else if let Some(ref mut tracing) = self.tracing {
                tracing.service_name = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_TRACING_SAMPLE_RATIO") {
            if let Ok(ratio) = val.parse::<f64>() {
                if self.tracing.is_none() {
                    self.tracing = Some(TracingConfig {
                        enabled: true,
                        sample_ratio: ratio,
                        ..Default::default()
                    });
                } else if let Some(ref mut tracing) = self.tracing {
                    tracing.sample_ratio = ratio;
                }
            }
        }

        // Connection limits configuration
        if let Ok(val) = std::env::var("RUSMES_CONNECTION_LIMITS_MAX_CONNECTIONS_PER_IP") {
            if let Ok(max) = val.parse::<usize>() {
                if self.connection_limits.is_none() {
                    self.connection_limits = Some(ConnectionLimitsConfig {
                        max_connections_per_ip: max,
                        max_total_connections: default_max_total_connections(),
                        idle_timeout: default_idle_timeout(),
                        reaper_interval: default_reaper_interval(),
                    });
                } else if let Some(ref mut limits) = self.connection_limits {
                    limits.max_connections_per_ip = max;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_CONNECTION_LIMITS_MAX_TOTAL_CONNECTIONS") {
            if let Ok(max) = val.parse::<usize>() {
                if self.connection_limits.is_none() {
                    self.connection_limits = Some(ConnectionLimitsConfig {
                        max_connections_per_ip: default_max_connections_per_ip(),
                        max_total_connections: max,
                        idle_timeout: default_idle_timeout(),
                        reaper_interval: default_reaper_interval(),
                    });
                } else if let Some(ref mut limits) = self.connection_limits {
                    limits.max_total_connections = max;
                }
            }
        }
        if let Ok(val) = std::env::var("RUSMES_CONNECTION_LIMITS_IDLE_TIMEOUT") {
            if self.connection_limits.is_none() {
                self.connection_limits = Some(ConnectionLimitsConfig {
                    max_connections_per_ip: default_max_connections_per_ip(),
                    max_total_connections: default_max_total_connections(),
                    idle_timeout: val,
                    reaper_interval: default_reaper_interval(),
                });
            } else if let Some(ref mut limits) = self.connection_limits {
                limits.idle_timeout = val;
            }
        }
        if let Ok(val) = std::env::var("RUSMES_CONNECTION_LIMITS_REAPER_INTERVAL") {
            if self.connection_limits.is_none() {
                self.connection_limits = Some(ConnectionLimitsConfig {
                    max_connections_per_ip: default_max_connections_per_ip(),
                    max_total_connections: default_max_total_connections(),
                    idle_timeout: default_idle_timeout(),
                    reaper_interval: val,
                });
            } else if let Some(ref mut limits) = self.connection_limits {
                limits.reaper_interval = val;
            }
        }
    }

    /// Validate the entire configuration
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

        Ok(())
    }

    /// Get postmaster address
    pub fn postmaster_address(&self) -> anyhow::Result<MailAddress> {
        self.postmaster
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid postmaster address: {}", e))
    }
}

/// SMTP server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmtpServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub tls_port: Option<u16>,
    pub max_message_size: String, // e.g., "50MB"
    #[serde(default)]
    pub require_auth: bool,
    #[serde(default)]
    pub enable_starttls: bool,
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
}

impl SmtpServerConfig {
    /// Parse max message size to bytes
    pub fn max_message_size_bytes(&self) -> anyhow::Result<usize> {
        parse_size(&self.max_message_size)
    }
}

/// IMAP server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImapServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub tls_port: Option<u16>,
}

/// JMAP server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JmapServerConfig {
    pub host: String,
    pub port: u16,
    pub base_url: String,
}

/// POP3 server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Pop3ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub tls_port: Option<u16>,
    #[serde(default = "default_pop3_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub enable_stls: bool,
}

fn default_pop3_timeout() -> u64 {
    600
}

/// SMTP Relay configuration for outbound mail
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_use_tls")]
    pub use_tls: bool,
}

fn default_use_tls() -> bool {
    true
}

/// Storage backend configuration
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

/// Processor chain configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessorConfig {
    pub name: String,
    pub state: String,
    pub mailets: Vec<MailetConfig>,
}

/// Mailet configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MailetConfig {
    pub matcher: String,
    pub mailet: String,
    #[serde(default)]
    pub params: HashMap<String, String>,
}

/// Rate limiting configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_max_connections_per_ip")]
    pub max_connections_per_ip: usize,
    pub max_messages_per_hour: u32,
    pub window_duration: String, // e.g., "1h"
}

fn default_max_connections_per_ip() -> usize {
    10
}

impl RateLimitConfig {
    /// Parse window duration to seconds
    pub fn window_duration_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.window_duration)
    }
}

/// Authentication backend configuration
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

/// File-based authentication configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileAuthConfig {
    pub path: String,
}

/// LDAP authentication configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LdapAuthConfig {
    pub url: String,
    pub base_dn: String,
    pub bind_dn: String,
    pub bind_password: String,
    pub user_filter: String,
}

/// SQL authentication configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SqlAuthConfig {
    pub connection_string: String,
    pub query: String,
}

/// OAuth2 authentication configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuth2AuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub token_url: String,
    pub authorization_url: String,
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    pub level: String,  // trace, debug, info, warn, error
    pub format: String, // json or text
    pub output: String, // stdout, stderr, or file path
    #[serde(default)]
    pub file: Option<LogFileConfig>,
}

impl LoggingConfig {
    /// Validate log level
    pub fn validate_level(&self) -> anyhow::Result<()> {
        match self.level.as_str() {
            "trace" | "debug" | "info" | "warn" | "error" => Ok(()),
            _ => Err(anyhow::anyhow!("Invalid log level: {}", self.level)),
        }
    }

    /// Validate log format
    pub fn validate_format(&self) -> anyhow::Result<()> {
        match self.format.as_str() {
            "json" | "text" => Ok(()),
            _ => Err(anyhow::anyhow!("Invalid log format: {}", self.format)),
        }
    }
}

/// Log file configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogFileConfig {
    pub path: String,
    pub max_size: String, // e.g., "100MB"
    pub max_backups: u32,
    pub compress: bool,
}

impl LogFileConfig {
    /// Parse max file size to bytes
    pub fn max_size_bytes(&self) -> anyhow::Result<usize> {
        parse_size(&self.max_size)
    }
}

/// Queue configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QueueConfig {
    pub initial_delay: String, // e.g., "60s"
    pub max_delay: String,     // e.g., "3600s"
    pub backoff_multiplier: f64,
    pub max_attempts: u32,
    pub worker_threads: usize,
    pub batch_size: usize,
}

impl QueueConfig {
    /// Parse initial delay to seconds
    pub fn initial_delay_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.initial_delay)
    }

    /// Parse max delay to seconds
    pub fn max_delay_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.max_delay)
    }

    /// Validate backoff multiplier
    pub fn validate_backoff_multiplier(&self) -> anyhow::Result<()> {
        if self.backoff_multiplier <= 0.0 {
            return Err(anyhow::anyhow!("backoff_multiplier must be positive"));
        }
        Ok(())
    }

    /// Validate worker threads
    pub fn validate_worker_threads(&self) -> anyhow::Result<()> {
        if self.worker_threads == 0 {
            return Err(anyhow::anyhow!("worker_threads must be greater than 0"));
        }
        Ok(())
    }
}

/// Security configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    pub relay_networks: Vec<String>, // CIDR notation
    pub blocked_ips: Vec<String>,
    pub check_recipient_exists: bool,
    pub reject_unknown_recipients: bool,
}

impl SecurityConfig {
    /// Validate CIDR notation for relay networks
    pub fn validate_relay_networks(&self) -> anyhow::Result<()> {
        for network in &self.relay_networks {
            // Basic validation - should contain a slash for CIDR notation
            if !network.contains('/') {
                return Err(anyhow::anyhow!("Invalid CIDR notation: {}", network));
            }
        }
        Ok(())
    }

    /// Validate IP addresses in blocked list
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

/// Domains configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DomainsConfig {
    pub local_domains: Vec<String>,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

impl DomainsConfig {
    /// Validate domain names
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

    /// Validate alias email addresses
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

/// Metrics configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub bind_address: String, // e.g., "0.0.0.0:9090"
    pub path: String,         // e.g., "/metrics"
}

impl MetricsConfig {
    /// Validate bind address format
    pub fn validate_bind_address(&self) -> anyhow::Result<()> {
        if !self.bind_address.contains(':') {
            return Err(anyhow::anyhow!(
                "Invalid bind address format: {}",
                self.bind_address
            ));
        }
        Ok(())
    }

    /// Validate path format
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

/// OpenTelemetry tracing configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TracingConfig {
    pub enabled: bool,
    pub endpoint: String, // e.g., "http://localhost:4317" for gRPC or "http://localhost:4318" for HTTP
    pub protocol: OtlpProtocol,
    pub service_name: String,
    #[serde(default)]
    pub sample_ratio: f64, // 0.0 to 1.0, default 1.0 (trace everything)
}

/// OTLP protocol type
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
    /// Validate endpoint URL format
    pub fn validate_endpoint(&self) -> anyhow::Result<()> {
        if !self.endpoint.starts_with("http://") && !self.endpoint.starts_with("https://") {
            return Err(anyhow::anyhow!(
                "Endpoint must start with http:// or https://: {}",
                self.endpoint
            ));
        }
        Ok(())
    }

    /// Validate sample ratio
    pub fn validate_sample_ratio(&self) -> anyhow::Result<()> {
        if !(0.0..=1.0).contains(&self.sample_ratio) {
            return Err(anyhow::anyhow!(
                "Sample ratio must be between 0.0 and 1.0: {}",
                self.sample_ratio
            ));
        }
        Ok(())
    }

    /// Validate service name
    pub fn validate_service_name(&self) -> anyhow::Result<()> {
        if self.service_name.trim().is_empty() {
            return Err(anyhow::anyhow!("Service name cannot be empty"));
        }
        Ok(())
    }
}

/// Connection limits configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionLimitsConfig {
    /// Maximum connections per IP address (0 = unlimited)
    #[serde(default = "default_max_connections_per_ip")]
    pub max_connections_per_ip: usize,
    /// Maximum total connections (0 = unlimited)
    #[serde(default = "default_max_total_connections")]
    pub max_total_connections: usize,
    /// Idle timeout for connections (e.g., "300s", "5m")
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: String,
    /// Reaper interval for cleaning up idle connections (e.g., "60s", "1m")
    #[serde(default = "default_reaper_interval")]
    pub reaper_interval: String,
}

fn default_max_total_connections() -> usize {
    1000
}

fn default_idle_timeout() -> String {
    "300s".to_string()
}

fn default_reaper_interval() -> String {
    "60s".to_string()
}

impl ConnectionLimitsConfig {
    /// Parse idle timeout to seconds
    pub fn idle_timeout_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.idle_timeout)
    }

    /// Parse reaper interval to seconds
    pub fn reaper_interval_seconds(&self) -> anyhow::Result<u64> {
        parse_duration(&self.reaper_interval)
    }
}

/// Parse size string like "50MB", "1GB", "1024KB"
fn parse_size(s: &str) -> anyhow::Result<usize> {
    let s = s.trim().to_uppercase();

    if let Some(rest) = s.strip_suffix("GB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0 * 1024.0 * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix("MB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0 * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix("KB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix("B") {
        let num: usize = rest.trim().parse()?;
        Ok(num)
    } else {
        // Assume bytes
        let num: usize = s.parse()?;
        Ok(num)
    }
}

/// Parse duration string like "60s", "30m", "1h"
fn parse_duration(s: &str) -> anyhow::Result<u64> {
    let s = s.trim().to_lowercase();

    if let Some(rest) = s.strip_suffix('h') {
        let num: u64 = rest.trim().parse()?;
        Ok(num * 3600)
    } else if let Some(rest) = s.strip_suffix('m') {
        let num: u64 = rest.trim().parse()?;
        Ok(num * 60)
    } else if let Some(rest) = s.strip_suffix('s') {
        let num: u64 = rest.trim().parse()?;
        Ok(num)
    } else {
        // Assume seconds
        let num: u64 = s.parse()?;
        Ok(num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("50MB").unwrap(), 50 * 1024 * 1024);
    }

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("60").unwrap(), 60);
        assert_eq!(parse_duration("60s").unwrap(), 60);
        assert_eq!(parse_duration("5m").unwrap(), 300);
        assert_eq!(parse_duration("1h").unwrap(), 3600);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
    }

    #[test]
    fn test_parse_toml_config() {
        let toml_str = r#"
            domain = "example.com"
            postmaster = "postmaster@example.com"

            [smtp]
            host = "0.0.0.0"
            port = 25
            tls_port = 587
            max_message_size = "50MB"

            [storage]
            backend = "filesystem"
            path = "/var/mail"

            [[processors]]
            name = "root"
            state = "root"

            [[processors.mailets]]
            matcher = "All"
            mailet = "LocalDelivery"
        "#;

        let config: ServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.domain, "example.com");
        assert_eq!(config.smtp.port, 25);
        assert_eq!(config.processors.len(), 1);
        assert_eq!(config.processors[0].mailets.len(), 1);
    }

    #[test]
    fn test_parse_auth_config() {
        let toml_str = r#"
            backend = "file"
            path = "/etc/rusmes/users.db"
        "#;

        let config: AuthConfig = toml::from_str(toml_str).unwrap();
        match config {
            AuthConfig::File { config } => {
                assert_eq!(config.path, "/etc/rusmes/users.db");
            }
            _ => panic!("Expected File auth backend"),
        }
    }

    #[test]
    fn test_parse_logging_config() {
        let toml_str = r#"
            level = "info"
            format = "json"
            output = "stdout"
        "#;

        let config: LoggingConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.level, "info");
        assert_eq!(config.format, "json");
        assert_eq!(config.output, "stdout");
        config.validate_level().unwrap();
        config.validate_format().unwrap();
    }

    #[test]
    fn test_parse_queue_config() {
        let toml_str = r#"
            initial_delay = "60s"
            max_delay = "3600s"
            backoff_multiplier = 2.0
            max_attempts = 5
            worker_threads = 5
            batch_size = 100
        "#;

        let config: QueueConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.initial_delay_seconds().unwrap(), 60);
        assert_eq!(config.max_delay_seconds().unwrap(), 3600);
        assert_eq!(config.backoff_multiplier, 2.0);
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.worker_threads, 5);
        assert_eq!(config.batch_size, 100);
        config.validate_backoff_multiplier().unwrap();
        config.validate_worker_threads().unwrap();
    }

    #[test]
    fn test_parse_security_config() {
        let toml_str = r#"
            relay_networks = ["127.0.0.0/8", "10.0.0.0/8"]
            blocked_ips = ["192.0.2.1", "2001:db8::1"]
            check_recipient_exists = true
            reject_unknown_recipients = true
        "#;

        let config: SecurityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.relay_networks.len(), 2);
        assert_eq!(config.blocked_ips.len(), 2);
        assert!(config.check_recipient_exists);
        assert!(config.reject_unknown_recipients);
        config.validate_relay_networks().unwrap();
        config.validate_blocked_ips().unwrap();
    }

    #[test]
    fn test_parse_domains_config() {
        let toml_str = r#"
            local_domains = ["example.com", "mail.example.com"]

            [aliases]
            "abuse@example.com" = "postmaster@example.com"
            "webmaster@example.com" = "admin@example.com"
        "#;

        let config: DomainsConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.local_domains.len(), 2);
        assert_eq!(config.aliases.len(), 2);
        assert_eq!(
            config.aliases.get("abuse@example.com"),
            Some(&"postmaster@example.com".to_string())
        );
        config.validate_local_domains().unwrap();
        config.validate_aliases().unwrap();
    }

    #[test]
    fn test_parse_metrics_config() {
        let toml_str = r#"
            enabled = true
            bind_address = "0.0.0.0:9090"
            path = "/metrics"
        "#;

        let config: MetricsConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
        assert_eq!(config.bind_address, "0.0.0.0:9090");
        assert_eq!(config.path, "/metrics");
        config.validate_bind_address().unwrap();
        config.validate_path().unwrap();
    }

    #[test]
    fn test_parse_rate_limit_config() {
        let toml_str = r#"
            max_messages_per_hour = 100
            window_duration = "1h"
        "#;

        let config: RateLimitConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.max_messages_per_hour, 100);
        assert_eq!(config.window_duration_seconds().unwrap(), 3600);
    }

    #[test]
    fn test_parse_full_config_with_all_sections() {
        let toml_str = r#"
            domain = "mail.example.com"
            postmaster = "postmaster@example.com"

            [smtp]
            host = "0.0.0.0"
            port = 25
            tls_port = 587
            max_message_size = "50MB"

            [smtp.rate_limit]
            max_messages_per_hour = 100
            window_duration = "1h"

            [storage]
            backend = "filesystem"
            path = "/var/mail"

            [auth]
            backend = "file"
            path = "/etc/rusmes/users.db"

            [logging]
            level = "info"
            format = "json"
            output = "stdout"

            [queue]
            initial_delay = "60s"
            max_delay = "3600s"
            backoff_multiplier = 2.0
            max_attempts = 5
            worker_threads = 5
            batch_size = 100

            [security]
            relay_networks = ["127.0.0.0/8"]
            blocked_ips = []
            check_recipient_exists = true
            reject_unknown_recipients = true

            [domains]
            local_domains = ["example.com"]

            [domains.aliases]
            "abuse@example.com" = "postmaster@example.com"

            [metrics]
            enabled = true
            bind_address = "0.0.0.0:9090"
            path = "/metrics"

            [[processors]]
            name = "root"
            state = "root"

            [[processors.mailets]]
            matcher = "All"
            mailet = "LocalDelivery"
        "#;

        let config: ServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.domain, "mail.example.com");
        assert!(config.auth.is_some());
        assert!(config.logging.is_some());
        assert!(config.queue.is_some());
        assert!(config.security.is_some());
        assert!(config.domains.is_some());
        assert!(config.metrics.is_some());
        assert!(config.smtp.rate_limit.is_some());

        // Validate sections
        if let Some(logging) = &config.logging {
            logging.validate_level().unwrap();
            logging.validate_format().unwrap();
        }

        if let Some(queue) = &config.queue {
            queue.validate_backoff_multiplier().unwrap();
            queue.validate_worker_threads().unwrap();
        }

        if let Some(security) = &config.security {
            security.validate_relay_networks().unwrap();
            security.validate_blocked_ips().unwrap();
        }

        if let Some(domains) = &config.domains {
            domains.validate_local_domains().unwrap();
            domains.validate_aliases().unwrap();
        }

        if let Some(metrics) = &config.metrics {
            metrics.validate_bind_address().unwrap();
            metrics.validate_path().unwrap();
        }
    }

    #[test]
    fn test_parse_yaml_config() {
        let yaml_str = r#"
domain: example.com
postmaster: postmaster@example.com

smtp:
  host: 0.0.0.0
  port: 25
  tls_port: 587
  max_message_size: 50MB

storage:
  backend: filesystem
  path: /var/mail

processors:
  - name: root
    state: root
    mailets:
      - matcher: All
        mailet: LocalDelivery
        params: {}
        "#;

        let config: ServerConfig = serde_yaml::from_str(yaml_str).unwrap();
        assert_eq!(config.domain, "example.com");
        assert_eq!(config.smtp.port, 25);
        assert_eq!(config.processors.len(), 1);
        assert_eq!(config.processors[0].mailets.len(), 1);
    }

    #[test]
    fn test_yaml_equivalence_to_toml() {
        // YAML version
        let yaml_str = r#"
domain: mail.example.com
postmaster: postmaster@example.com

smtp:
  host: 0.0.0.0
  port: 25
  tls_port: 587
  max_message_size: 50MB
  require_auth: true
  enable_starttls: true

storage:
  backend: filesystem
  path: /var/mail

processors:
  - name: root
    state: root
    mailets:
      - matcher: All
        mailet: LocalDelivery

auth:
  backend: file
  path: /etc/rusmes/users.db

logging:
  level: info
  format: json
  output: stdout

domains:
  local_domains:
    - example.com
    - mail.example.com
        "#;

        // TOML version
        let toml_str = r#"
domain = "mail.example.com"
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 25
tls_port = 587
max_message_size = "50MB"
require_auth = true
enable_starttls = true

[storage]
backend = "filesystem"
path = "/var/mail"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"

[auth]
backend = "file"
path = "/etc/rusmes/users.db"

[logging]
level = "info"
format = "json"
output = "stdout"

[domains]
local_domains = ["example.com", "mail.example.com"]
        "#;

        let yaml_config: ServerConfig = serde_yaml::from_str(yaml_str).unwrap();
        let toml_config: ServerConfig = toml::from_str(toml_str).unwrap();

        // Verify both configs are equivalent
        assert_eq!(yaml_config.domain, toml_config.domain);
        assert_eq!(yaml_config.postmaster, toml_config.postmaster);
        assert_eq!(yaml_config.smtp.host, toml_config.smtp.host);
        assert_eq!(yaml_config.smtp.port, toml_config.smtp.port);
        assert_eq!(yaml_config.smtp.tls_port, toml_config.smtp.tls_port);
        assert_eq!(
            yaml_config.smtp.max_message_size,
            toml_config.smtp.max_message_size
        );
        assert_eq!(yaml_config.smtp.require_auth, toml_config.smtp.require_auth);
        assert_eq!(
            yaml_config.smtp.enable_starttls,
            toml_config.smtp.enable_starttls
        );
        assert_eq!(yaml_config.processors.len(), toml_config.processors.len());

        // Check auth config
        assert!(yaml_config.auth.is_some());
        assert!(toml_config.auth.is_some());

        // Check logging config
        if let (Some(yaml_log), Some(toml_log)) = (&yaml_config.logging, &toml_config.logging) {
            assert_eq!(yaml_log.level, toml_log.level);
            assert_eq!(yaml_log.format, toml_log.format);
            assert_eq!(yaml_log.output, toml_log.output);
        }

        // Check domains config
        if let (Some(yaml_domains), Some(toml_domains)) =
            (&yaml_config.domains, &toml_config.domains)
        {
            assert_eq!(
                yaml_domains.local_domains.len(),
                toml_domains.local_domains.len()
            );
            assert_eq!(yaml_domains.local_domains, toml_domains.local_domains);
        }
    }
}
