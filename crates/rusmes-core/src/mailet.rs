//! Mailet trait and types

use async_trait::async_trait;
use rusmes_proto::{Mail, MailState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during mailet processing
#[derive(Debug, Error)]
pub enum MailetError {
    /// Mailet exceeded its configured execution timeout
    #[error("Mailet execution timed out after {0:?}")]
    Timeout(Duration),
    /// Mailet returned an error
    #[error("Mailet error: {0}")]
    ServiceError(#[from] anyhow::Error),
}

/// Policy controlling what happens when a mailet errors or times out
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum MailetErrorPolicy {
    /// Skip this mailet and continue the pipeline to the next step
    Skip,
    /// Abort the pipeline and propagate the error upstream (4xx/5xx response)
    #[default]
    Abort,
    /// Re-enqueue up to `max` times with `backoff` delay between retries,
    /// then Abort if still failing
    Retry {
        /// Maximum number of retry attempts
        max: u32,
        /// Delay between retry attempts
        #[serde(with = "duration_serde")]
        backoff: Duration,
    },
}

/// Serde helpers for Duration
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u128::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis as u64))
    }
}

/// Actions a mailet can take after processing a mail
#[derive(Debug, Clone, PartialEq)]
pub enum MailetAction {
    /// Continue to next mailet in the chain
    Continue,
    /// Change mail state and move to different processor
    ChangeState(MailState),
    /// Drop the mail (set state to Ghost)
    Drop,
    /// Defer processing (requeue with delay)
    Defer(Duration),
}

/// Mailet configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailetConfig {
    /// Mailet name
    pub name: String,
    /// Configuration parameters
    pub params: HashMap<String, String>,
    /// Optional execution timeout in milliseconds. None means no timeout.
    pub timeout_ms: Option<u64>,
    /// Error handling policy when the mailet returns an error or times out
    #[serde(default)]
    pub error_policy: MailetErrorPolicy,
}

impl MailetConfig {
    /// Create a new mailet config
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            params: HashMap::new(),
            timeout_ms: None,
            error_policy: MailetErrorPolicy::default(),
        }
    }

    /// Add a parameter
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    /// Set execution timeout in milliseconds
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Set the error policy
    pub fn with_error_policy(mut self, policy: MailetErrorPolicy) -> Self {
        self.error_policy = policy;
        self
    }

    /// Get a parameter value
    pub fn get_param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    /// Get a required parameter
    pub fn require_param(&self, key: &str) -> anyhow::Result<&str> {
        self.get_param(key).ok_or_else(|| {
            anyhow::anyhow!("Required parameter '{}' not found in mailet config", key)
        })
    }
}

/// Core mailet trait - message processing unit
#[async_trait]
pub trait Mailet: Send + Sync {
    /// Initialize mailet with configuration
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()>;

    /// Process a mail message
    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction>;

    /// Cleanup on shutdown
    async fn destroy(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Mailet name for logging/metrics
    fn name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mailet_config() {
        let config = MailetConfig::new("TestMailet")
            .with_param("key1", "value1")
            .with_param("key2", "value2");

        assert_eq!(config.name, "TestMailet");
        assert_eq!(config.get_param("key1"), Some("value1"));
        assert_eq!(config.get_param("key2"), Some("value2"));
        assert_eq!(config.get_param("nonexistent"), None);
    }

    #[test]
    fn test_mailet_config_timeout() {
        let config = MailetConfig::new("TimedMailet").with_timeout_ms(500);
        assert_eq!(config.timeout_ms, Some(500));
    }

    #[test]
    fn test_mailet_config_error_policy() {
        let skip_config =
            MailetConfig::new("SkipMailet").with_error_policy(MailetErrorPolicy::Skip);
        matches!(skip_config.error_policy, MailetErrorPolicy::Skip);

        let retry_config =
            MailetConfig::new("RetryMailet").with_error_policy(MailetErrorPolicy::Retry {
                max: 3,
                backoff: Duration::from_millis(100),
            });
        matches!(
            retry_config.error_policy,
            MailetErrorPolicy::Retry { max: 3, .. }
        );
    }

    #[test]
    fn test_mailet_action_equality() {
        assert_eq!(MailetAction::Continue, MailetAction::Continue);
        assert_eq!(MailetAction::Drop, MailetAction::Drop);
        assert_ne!(MailetAction::Continue, MailetAction::Drop);
    }
}
