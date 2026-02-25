//! Load test configuration

use crate::scenarios::ScenarioType;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Protocol to test
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Protocol {
    /// SMTP protocol
    Smtp,
    /// IMAP protocol
    Imap,
    /// JMAP protocol
    Jmap,
    /// POP3 protocol
    Pop3,
    /// Mixed protocols
    Mixed,
}

/// Message size configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageSize {
    /// Fixed size in bytes
    Fixed(usize),
    /// Random size between min and max
    Random { min: usize, max: usize },
}

impl MessageSize {
    /// Get a size value
    pub fn get(&self) -> (usize, usize) {
        match self {
            MessageSize::Fixed(size) => (*size, *size),
            MessageSize::Random { min, max } => (*min, *max),
        }
    }
}

/// Message content type
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MessageContent {
    /// Random text
    Random,
    /// Template-based
    Template,
    /// Real-world simulated
    RealWorld,
}

/// Load test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadTestConfig {
    /// Target host to test
    pub target_host: String,

    /// Target port
    pub target_port: u16,

    /// Protocol to test
    pub protocol: Protocol,

    /// Test scenario to run
    pub scenario: ScenarioType,

    /// Test duration in seconds
    pub duration_secs: u64,

    /// Number of concurrent workers
    pub concurrency: usize,

    /// Target message rate (messages per second)
    pub message_rate: u64,

    /// Ramp-up duration in seconds (gradual increase to target rate)
    pub ramp_up_secs: u64,

    /// Message size configuration
    pub message_size: MessageSize,

    /// Message content type
    pub message_content: MessageContent,

    /// Minimum message size in bytes (deprecated, use message_size)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_size_min: Option<usize>,

    /// Maximum message size in bytes (deprecated, use message_size)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_size_max: Option<usize>,

    /// Output file for JSON report
    pub output_json: Option<String>,

    /// Output file for HTML report
    pub output_html: Option<String>,

    /// Output file for CSV report
    pub output_csv: Option<String>,

    /// Enable Prometheus metrics export
    pub prometheus_export: bool,

    /// Prometheus export port
    pub prometheus_port: u16,

    /// Mixed protocol weights (SMTP, IMAP, JMAP, POP3)
    pub mixed_weights: Option<(u8, u8, u8, u8)>,
}

impl LoadTestConfig {
    /// Get ramp-up duration
    pub fn ramp_up_duration(&self) -> Duration {
        Duration::from_secs(self.ramp_up_secs)
    }

    /// Get test duration
    pub fn test_duration(&self) -> Duration {
        Duration::from_secs(self.duration_secs)
    }

    /// Get message size range
    pub fn message_size_range(&self) -> (usize, usize) {
        self.message_size.get()
    }
}

impl Default for LoadTestConfig {
    fn default() -> Self {
        Self {
            target_host: "localhost".to_string(),
            target_port: 25,
            protocol: Protocol::Smtp,
            scenario: ScenarioType::SmtpThroughput,
            duration_secs: 60,
            concurrency: 10,
            message_rate: 100,
            ramp_up_secs: 0,
            message_size: MessageSize::Random {
                min: 1024,
                max: 102400,
            },
            message_content: MessageContent::Random,
            message_size_min: None,
            message_size_max: None,
            output_json: None,
            output_html: None,
            output_csv: None,
            prometheus_export: false,
            prometheus_port: 9090,
            mixed_weights: None,
        }
    }
}

impl LoadTestConfig {
    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.target_host.is_empty() {
            return Err("Target host cannot be empty".to_string());
        }

        if self.target_port == 0 {
            return Err("Target port must be greater than 0".to_string());
        }

        if self.duration_secs == 0 {
            return Err("Duration must be greater than 0".to_string());
        }

        if self.concurrency == 0 {
            return Err("Concurrency must be greater than 0".to_string());
        }

        match &self.message_size {
            MessageSize::Fixed(size) if *size == 0 => {
                return Err("Message size must be greater than 0".to_string());
            }
            MessageSize::Random { min, max } if min > max => {
                return Err("Min message size cannot be greater than max".to_string());
            }
            MessageSize::Random { min, .. } if *min == 0 => {
                return Err("Min message size must be greater than 0".to_string());
            }
            _ => {}
        }

        if self.protocol == Protocol::Mixed && self.mixed_weights.is_none() {
            return Err("Mixed protocol requires weights (smtp, imap, jmap, pop3)".to_string());
        }

        if let Some((smtp, imap, jmap, pop3)) = self.mixed_weights {
            if smtp + imap + jmap + pop3 == 0 {
                return Err("At least one protocol weight must be non-zero".to_string());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = LoadTestConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_empty_host_is_invalid() {
        let config = LoadTestConfig {
            target_host: "".to_string(),
            ..LoadTestConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_zero_port_is_invalid() {
        let config = LoadTestConfig {
            target_port: 0,
            ..LoadTestConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_zero_duration_is_invalid() {
        let config = LoadTestConfig {
            duration_secs: 0,
            ..LoadTestConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_zero_concurrency_is_invalid() {
        let config = LoadTestConfig {
            concurrency: 0,
            ..LoadTestConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_message_sizes() {
        let config = LoadTestConfig {
            message_size: MessageSize::Random {
                min: 10000,
                max: 1000,
            },
            ..LoadTestConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
