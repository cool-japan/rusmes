//! HTTP-01 challenge handler for ACME

use crate::{AcmeError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// HTTP-01 challenge handler
///
/// Handles HTTP-01 challenges by serving challenge responses on:
/// `http://<domain>/.well-known/acme-challenge/<token>`
#[derive(Clone)]
pub struct Http01Handler {
    /// Map of token -> key_authorization
    challenges: Arc<RwLock<HashMap<String, String>>>,
}

impl Http01Handler {
    /// Create a new HTTP-01 challenge handler
    pub fn new() -> Self {
        Self {
            challenges: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a challenge response
    pub async fn add_challenge(&self, token: String, key_authorization: String) {
        let mut challenges = self.challenges.write().await;
        challenges.insert(token.clone(), key_authorization);
        info!("Added HTTP-01 challenge for token: {}", token);
    }

    /// Get challenge response for a token
    pub async fn get_challenge(&self, token: &str) -> Option<String> {
        let challenges = self.challenges.read().await;
        let response = challenges.get(token).cloned();
        debug!(
            "HTTP-01 challenge lookup for token {}: {:?}",
            token,
            response.is_some()
        );
        response
    }

    /// Remove a challenge
    pub async fn remove_challenge(&self, token: &str) {
        let mut challenges = self.challenges.write().await;
        challenges.remove(token);
        info!("Removed HTTP-01 challenge for token: {}", token);
    }

    /// Clear all challenges
    pub async fn clear(&self) {
        let mut challenges = self.challenges.write().await;
        challenges.clear();
        info!("Cleared all HTTP-01 challenges");
    }

    /// Handle HTTP-01 challenge request
    ///
    /// This should be integrated with the HTTP server to serve challenges at:
    /// GET /.well-known/acme-challenge/{token}
    pub async fn handle_request(&self, token: &str) -> Result<String> {
        self.get_challenge(token).await.ok_or_else(|| {
            AcmeError::ChallengeFailed(format!("Challenge token not found: {}", token))
        })
    }

    /// Get the well-known path for a token
    pub fn well_known_path(token: &str) -> String {
        format!("/.well-known/acme-challenge/{}", token)
    }

    /// Verify challenge can be accessed
    pub async fn verify_accessibility(&self, domain: &str, token: &str) -> Result<bool> {
        let url = format!("http://{}/.well-known/acme-challenge/{}", domain, token);

        match reqwest::get(&url).await {
            Ok(response) => {
                if response.status().is_success() {
                    let body = response.text().await?;
                    let expected = self.get_challenge(token).await;

                    if let Some(expected_value) = expected {
                        Ok(body == expected_value)
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            }
            Err(_) => Ok(false),
        }
    }
}

impl Default for Http01Handler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_http01_handler_creation() {
        let handler = Http01Handler::new();
        let result = handler.get_challenge("test").await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_add_and_get_challenge() {
        let handler = Http01Handler::new();

        handler
            .add_challenge("test-token".to_string(), "test-key-auth".to_string())
            .await;

        let result = handler.get_challenge("test-token").await;
        assert_eq!(result, Some("test-key-auth".to_string()));
    }

    #[tokio::test]
    async fn test_remove_challenge() {
        let handler = Http01Handler::new();

        handler
            .add_challenge("test-token".to_string(), "test-key-auth".to_string())
            .await;

        let result = handler.get_challenge("test-token").await;
        assert!(result.is_some());

        handler.remove_challenge("test-token").await;

        let result = handler.get_challenge("test-token").await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_clear_all_challenges() {
        let handler = Http01Handler::new();

        handler
            .add_challenge("token1".to_string(), "auth1".to_string())
            .await;
        handler
            .add_challenge("token2".to_string(), "auth2".to_string())
            .await;

        handler.clear().await;

        assert_eq!(handler.get_challenge("token1").await, None);
        assert_eq!(handler.get_challenge("token2").await, None);
    }

    #[tokio::test]
    async fn test_handle_request_success() {
        let handler = Http01Handler::new();

        handler
            .add_challenge("test-token".to_string(), "test-key-auth".to_string())
            .await;

        let result = handler.handle_request("test-token").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test-key-auth");
    }

    #[tokio::test]
    async fn test_handle_request_not_found() {
        let handler = Http01Handler::new();

        let result = handler.handle_request("nonexistent").await;
        assert!(result.is_err());
    }

    #[test]
    fn test_well_known_path() {
        let path = Http01Handler::well_known_path("test-token");
        assert_eq!(path, "/.well-known/acme-challenge/test-token");
    }

    #[tokio::test]
    async fn test_multiple_challenges() {
        let handler = Http01Handler::new();

        for i in 0..10 {
            handler
                .add_challenge(format!("token-{}", i), format!("auth-{}", i))
                .await;
        }

        for i in 0..10 {
            let result = handler.get_challenge(&format!("token-{}", i)).await;
            assert_eq!(result, Some(format!("auth-{}", i)));
        }
    }

    #[tokio::test]
    async fn test_overwrite_challenge() {
        let handler = Http01Handler::new();

        handler
            .add_challenge("token".to_string(), "auth1".to_string())
            .await;
        handler
            .add_challenge("token".to_string(), "auth2".to_string())
            .await;

        let result = handler.get_challenge("token").await;
        assert_eq!(result, Some("auth2".to_string()));
    }
}
