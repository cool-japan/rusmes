//! ACME challenge implementations

use crate::{AcmeError, Result};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Challenge manager
pub struct ChallengeManager {
    http_challenges: Arc<RwLock<std::collections::HashMap<String, String>>>,
}

impl ChallengeManager {
    /// Create a new challenge manager
    pub fn new() -> Self {
        Self {
            http_challenges: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Add HTTP-01 challenge
    pub async fn add_http_challenge(&self, token: String, key_auth: String) {
        let mut challenges = self.http_challenges.write().await;
        challenges.insert(token, key_auth);
        info!("Added HTTP-01 challenge");
    }

    /// Get HTTP-01 challenge response
    pub async fn get_http_challenge(&self, token: &str) -> Option<String> {
        let challenges = self.http_challenges.read().await;
        challenges.get(token).cloned()
    }

    /// Remove HTTP-01 challenge
    pub async fn remove_http_challenge(&self, token: &str) {
        let mut challenges = self.http_challenges.write().await;
        challenges.remove(token);
        debug!("Removed HTTP-01 challenge");
    }

    /// Clear all challenges
    pub async fn clear(&self) {
        let mut challenges = self.http_challenges.write().await;
        challenges.clear();
        info!("Cleared all challenges");
    }
}

impl Default for ChallengeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// HTTP-01 challenge handler
pub struct Http01Handler {
    manager: Arc<ChallengeManager>,
}

impl Http01Handler {
    /// Create a new HTTP-01 handler
    pub fn new(manager: Arc<ChallengeManager>) -> Self {
        Self { manager }
    }

    /// Handle challenge request
    pub async fn handle(&self, token: &str) -> Result<String> {
        self.manager
            .get_http_challenge(token)
            .await
            .ok_or_else(|| AcmeError::ChallengeFailed("Challenge not found".to_string()))
    }

    /// Setup challenge
    pub async fn setup(&self, token: String, key_auth: String) {
        self.manager.add_http_challenge(token, key_auth).await;
    }

    /// Cleanup challenge
    pub async fn cleanup(&self, token: &str) {
        self.manager.remove_http_challenge(token).await;
    }
}

/// DNS-01 challenge handler
pub struct Dns01Handler {
    // DNS provider API credentials would go here
}

impl Dns01Handler {
    /// Create a new DNS-01 handler
    pub fn new() -> Self {
        Self {}
    }

    /// Setup DNS TXT record for challenge
    pub async fn setup(&self, _domain: &str, _txt_record: String) -> Result<()> {
        // In real implementation, would create DNS TXT record via provider API
        info!("DNS-01 challenge setup");
        Ok(())
    }

    /// Cleanup DNS TXT record
    pub async fn cleanup(&self, _domain: &str) -> Result<()> {
        // In real implementation, would remove DNS TXT record
        info!("DNS-01 challenge cleanup");
        Ok(())
    }

    /// Verify DNS record propagation
    pub async fn verify_propagation(&self, _domain: &str, _expected_value: &str) -> Result<bool> {
        // In real implementation, would check DNS for TXT record
        Ok(true)
    }
}

impl Default for Dns01Handler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_challenge_manager() {
        let manager = ChallengeManager::new();

        manager
            .add_http_challenge("token1".to_string(), "key_auth1".to_string())
            .await;

        let response = manager.get_http_challenge("token1").await;
        assert_eq!(response, Some("key_auth1".to_string()));

        manager.remove_http_challenge("token1").await;
        let response = manager.get_http_challenge("token1").await;
        assert_eq!(response, None);
    }

    #[tokio::test]
    async fn test_http01_handler() {
        let manager = Arc::new(ChallengeManager::new());
        let handler = Http01Handler::new(manager);

        handler
            .setup("token1".to_string(), "key_auth1".to_string())
            .await;

        let response = handler.handle("token1").await.unwrap();
        assert_eq!(response, "key_auth1");

        handler.cleanup("token1").await;
        let result = handler.handle("token1").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_dns01_handler() {
        let handler = Dns01Handler::new();

        let result = handler
            .setup("example.com", "txt_record_value".to_string())
            .await;
        assert!(result.is_ok());

        let verified = handler
            .verify_propagation("example.com", "txt_record_value")
            .await
            .unwrap();
        assert!(verified);

        let result = handler.cleanup("example.com").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_challenge_manager_clear() {
        let manager = ChallengeManager::new();

        manager
            .add_http_challenge("token1".to_string(), "key1".to_string())
            .await;
        manager
            .add_http_challenge("token2".to_string(), "key2".to_string())
            .await;

        manager.clear().await;

        assert_eq!(manager.get_http_challenge("token1").await, None);
        assert_eq!(manager.get_http_challenge("token2").await, None);
    }
}
