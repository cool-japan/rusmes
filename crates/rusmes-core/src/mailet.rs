//! Mailet trait and types

use async_trait::async_trait;
use rusmes_proto::{Mail, MailState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

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
}

impl MailetConfig {
    /// Create a new mailet config
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            params: HashMap::new(),
        }
    }

    /// Add a parameter
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
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
    fn test_mailet_action_equality() {
        assert_eq!(MailetAction::Continue, MailetAction::Continue);
        assert_eq!(MailetAction::Drop, MailetAction::Drop);
        assert_ne!(MailetAction::Continue, MailetAction::Drop);
    }
}
