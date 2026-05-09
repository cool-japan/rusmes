//! Environment variable override logic for [`crate::ServerConfig`].
//!
//! All `RUSMES_*` variable handling lives here, isolated from the struct
//! definitions in `listeners.rs` and `runtime.rs`.

use crate::listeners::{
    ConnectionLimitsConfig, ImapServerConfig, JmapServerConfig, RateLimitConfig,
};
use crate::parse::{
    default_idle_timeout, default_max_connections_per_ip, default_max_total_connections,
    default_reaper_interval,
};
use crate::runtime::{LoggingConfig, MetricsConfig, OtlpProtocol, QueueConfig, TracingConfig};
use crate::ServerConfig;

impl ServerConfig {
    /// Apply environment variable overrides to configuration.
    ///
    /// Environment variables follow the convention `RUSMES_SECTION_KEY`.
    /// Priority: env vars > config file > defaults.
    ///
    /// Supported environment variables:
    /// - `RUSMES_DOMAIN`
    /// - `RUSMES_POSTMASTER`
    /// - `RUSMES_SMTP_HOST`
    /// - `RUSMES_SMTP_PORT`
    /// - `RUSMES_SMTP_TLS_PORT`
    /// - `RUSMES_SMTP_MAX_MESSAGE_SIZE`
    /// - `RUSMES_SMTP_REQUIRE_AUTH`
    /// - `RUSMES_SMTP_ENABLE_STARTTLS`
    /// - `RUSMES_SMTP_RATE_LIMIT_MAX_CONNECTIONS_PER_IP`
    /// - `RUSMES_SMTP_RATE_LIMIT_MAX_MESSAGES_PER_HOUR`
    /// - `RUSMES_SMTP_RATE_LIMIT_WINDOW_DURATION`
    /// - `RUSMES_IMAP_HOST`
    /// - `RUSMES_IMAP_PORT`
    /// - `RUSMES_IMAP_TLS_PORT`
    /// - `RUSMES_JMAP_HOST`
    /// - `RUSMES_JMAP_PORT`
    /// - `RUSMES_JMAP_BASE_URL`
    /// - `RUSMES_STORAGE_PATH` (for filesystem backend)
    /// - `RUSMES_LOG_LEVEL`
    /// - `RUSMES_LOG_FORMAT`
    /// - `RUSMES_LOG_OUTPUT`
    /// - `RUSMES_QUEUE_INITIAL_DELAY`
    /// - `RUSMES_QUEUE_MAX_DELAY`
    /// - `RUSMES_QUEUE_BACKOFF_MULTIPLIER`
    /// - `RUSMES_QUEUE_MAX_ATTEMPTS`
    /// - `RUSMES_QUEUE_WORKER_THREADS`
    /// - `RUSMES_QUEUE_BATCH_SIZE`
    /// - `RUSMES_METRICS_ENABLED`
    /// - `RUSMES_METRICS_BIND_ADDRESS`
    /// - `RUSMES_METRICS_PATH`
    /// - `RUSMES_TRACING_ENABLED`
    /// - `RUSMES_TRACING_ENDPOINT`
    /// - `RUSMES_TRACING_PROTOCOL` (grpc or http)
    /// - `RUSMES_TRACING_SERVICE_NAME`
    /// - `RUSMES_TRACING_SAMPLE_RATIO`
    /// - `RUSMES_CONNECTION_LIMITS_MAX_CONNECTIONS_PER_IP`
    /// - `RUSMES_CONNECTION_LIMITS_MAX_TOTAL_CONNECTIONS`
    /// - `RUSMES_CONNECTION_LIMITS_IDLE_TIMEOUT`
    /// - `RUSMES_CONNECTION_LIMITS_REAPER_INTERVAL`
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
                    push: None,
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
                        push: None,
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
                    push: None,
                });
            } else if let Some(ref mut jmap) = self.jmap {
                jmap.base_url = val;
            }
        }

        // Storage configuration (only filesystem backend path)
        if let Ok(val) = std::env::var("RUSMES_STORAGE_PATH") {
            if let crate::StorageConfig::Filesystem { ref mut path } = self.storage {
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
                        basic_auth: None,
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
                    basic_auth: None,
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
                    basic_auth: None,
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
}
