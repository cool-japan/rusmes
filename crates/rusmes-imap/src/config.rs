//! IMAP server configuration

use std::time::Duration;

/// IMAP server configuration
#[derive(Debug, Clone)]
pub struct ImapConfig {
    /// Bind address (e.g., "0.0.0.0:143")
    pub host: String,
    /// IMAP port (default 143)
    pub port: u16,
    /// Optional TLS port (default 993)
    pub tls_port: Option<u16>,
    /// Maximum concurrent connections
    pub max_connections: usize,
    /// Idle timeout - time between commands before auto-logout (default 30 minutes)
    pub idle_timeout: Duration,
    /// Optional TLS certificate path
    pub tls_cert: Option<String>,
    /// Optional TLS key path
    pub tls_key: Option<String>,
}

impl ImapConfig {
    /// Create a new IMAP configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Create configuration with custom idle timeout
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Create configuration with custom port
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Create configuration with custom host
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Get bind address (host:port)
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl Default for ImapConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 143,
            tls_port: Some(993),
            max_connections: 1000,
            idle_timeout: Duration::from_secs(1800), // 30 minutes
            tls_cert: None,
            tls_key: None,
        }
    }
}
