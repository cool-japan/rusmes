//! DNS-01 challenge handler for ACME

use crate::{AcmeError, Result};
use async_trait::async_trait;
use tracing::{debug, info, warn};

/// DNS provider trait for DNS-01 challenges
#[async_trait]
pub trait DnsProvider: Send + Sync {
    /// Create a TXT record for ACME challenge
    async fn create_txt_record(&self, domain: &str, name: &str, value: &str) -> Result<()>;

    /// Delete a TXT record
    async fn delete_txt_record(&self, domain: &str, name: &str) -> Result<()>;

    /// Verify TXT record propagation
    async fn verify_txt_record(
        &self,
        domain: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<bool>;
}

/// DNS-01 challenge handler
pub struct Dns01Handler {
    provider: Option<Box<dyn DnsProvider>>,
}

impl Dns01Handler {
    /// Create a new DNS-01 handler without provider
    pub fn new() -> Self {
        Self { provider: None }
    }

    /// Create a new DNS-01 handler with provider
    pub fn with_provider(provider: Box<dyn DnsProvider>) -> Self {
        Self {
            provider: Some(provider),
        }
    }

    /// Set DNS provider
    pub fn set_provider(&mut self, provider: Box<dyn DnsProvider>) {
        self.provider = Some(provider);
    }

    /// Setup DNS-01 challenge
    pub async fn setup(&self, domain: &str, txt_value: String) -> Result<()> {
        let record_name = format!("_acme-challenge.{}", domain);

        if let Some(ref provider) = self.provider {
            info!("Setting up DNS-01 challenge for {}", domain);
            provider
                .create_txt_record(domain, &record_name, &txt_value)
                .await?;
            Ok(())
        } else {
            Err(AcmeError::Other("No DNS provider configured".to_string()))
        }
    }

    /// Cleanup DNS-01 challenge
    pub async fn cleanup(&self, domain: &str) -> Result<()> {
        let record_name = format!("_acme-challenge.{}", domain);

        if let Some(ref provider) = self.provider {
            info!("Cleaning up DNS-01 challenge for {}", domain);
            provider.delete_txt_record(domain, &record_name).await?;
            Ok(())
        } else {
            Err(AcmeError::Other("No DNS provider configured".to_string()))
        }
    }

    /// Verify DNS propagation
    pub async fn verify_propagation(&self, domain: &str, expected_value: &str) -> Result<bool> {
        let record_name = format!("_acme-challenge.{}", domain);

        if let Some(ref provider) = self.provider {
            debug!("Verifying DNS-01 propagation for {}", domain);
            provider
                .verify_txt_record(domain, &record_name, expected_value)
                .await
        } else {
            Err(AcmeError::Other("No DNS provider configured".to_string()))
        }
    }

    /// Wait for DNS propagation with retries
    pub async fn wait_for_propagation(
        &self,
        domain: &str,
        expected_value: &str,
        max_attempts: u32,
        delay_secs: u64,
    ) -> Result<()> {
        for attempt in 1..=max_attempts {
            match self.verify_propagation(domain, expected_value).await {
                Ok(true) => {
                    info!(
                        "DNS propagation verified for {} (attempt {})",
                        domain, attempt
                    );
                    return Ok(());
                }
                Ok(false) => {
                    warn!(
                        "DNS propagation not yet complete for {} (attempt {}/{})",
                        domain, attempt, max_attempts
                    );
                }
                Err(e) => {
                    warn!("DNS verification failed for {}: {}", domain, e);
                }
            }

            if attempt < max_attempts {
                tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
            }
        }

        Err(AcmeError::ChallengeFailed(format!(
            "DNS propagation timeout for {} after {} attempts",
            domain, max_attempts
        )))
    }

    /// Get the ACME challenge record name
    pub fn challenge_record_name(domain: &str) -> String {
        format!("_acme-challenge.{}", domain)
    }
}

impl Default for Dns01Handler {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock DNS provider for testing
#[derive(Default)]
pub struct MockDnsProvider {
    records: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
}

impl MockDnsProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DnsProvider for MockDnsProvider {
    async fn create_txt_record(&self, _domain: &str, name: &str, value: &str) -> Result<()> {
        let mut records = self.records.write().await;
        records.insert(name.to_string(), value.to_string());
        Ok(())
    }

    async fn delete_txt_record(&self, _domain: &str, name: &str) -> Result<()> {
        let mut records = self.records.write().await;
        records.remove(name);
        Ok(())
    }

    async fn verify_txt_record(
        &self,
        _domain: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<bool> {
        let records = self.records.read().await;
        Ok(records
            .get(name)
            .map(|v| v == expected_value)
            .unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dns01_handler_no_provider() {
        let handler = Dns01Handler::new();
        let result = handler.setup("example.com", "test-value".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_dns01_handler_with_provider() {
        let provider = Box::new(MockDnsProvider::new());
        let handler = Dns01Handler::with_provider(provider);

        let result = handler.setup("example.com", "test-value".to_string()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_setup_and_cleanup() {
        let provider = Box::new(MockDnsProvider::new());
        let handler = Dns01Handler::with_provider(provider);

        handler
            .setup("example.com", "test-value".to_string())
            .await
            .unwrap();

        let verified = handler
            .verify_propagation("example.com", "test-value")
            .await
            .unwrap();
        assert!(verified);

        handler.cleanup("example.com").await.unwrap();

        let verified = handler
            .verify_propagation("example.com", "test-value")
            .await
            .unwrap();
        assert!(!verified);
    }

    #[tokio::test]
    async fn test_verify_propagation_success() {
        let provider = Box::new(MockDnsProvider::new());
        let handler = Dns01Handler::with_provider(provider);

        handler
            .setup("example.com", "test-value".to_string())
            .await
            .unwrap();

        let result = handler
            .verify_propagation("example.com", "test-value")
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_verify_propagation_wrong_value() {
        let provider = Box::new(MockDnsProvider::new());
        let handler = Dns01Handler::with_provider(provider);

        handler
            .setup("example.com", "test-value".to_string())
            .await
            .unwrap();

        let result = handler
            .verify_propagation("example.com", "wrong-value")
            .await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_wait_for_propagation_success() {
        let provider = Box::new(MockDnsProvider::new());
        let handler = Dns01Handler::with_provider(provider);

        handler
            .setup("example.com", "test-value".to_string())
            .await
            .unwrap();

        let result = handler
            .wait_for_propagation("example.com", "test-value", 3, 1)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_wait_for_propagation_timeout() {
        let provider = Box::new(MockDnsProvider::new());
        let handler = Dns01Handler::with_provider(provider);

        let result = handler
            .wait_for_propagation("example.com", "test-value", 2, 1)
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_challenge_record_name() {
        let name = Dns01Handler::challenge_record_name("example.com");
        assert_eq!(name, "_acme-challenge.example.com");
    }

    #[tokio::test]
    async fn test_multiple_domains() {
        let provider = Box::new(MockDnsProvider::new());
        let handler = Dns01Handler::with_provider(provider);

        handler
            .setup("example1.com", "value1".to_string())
            .await
            .unwrap();
        handler
            .setup("example2.com", "value2".to_string())
            .await
            .unwrap();

        let verified1 = handler
            .verify_propagation("example1.com", "value1")
            .await
            .unwrap();
        let verified2 = handler
            .verify_propagation("example2.com", "value2")
            .await
            .unwrap();

        assert!(verified1);
        assert!(verified2);
    }

    #[tokio::test]
    async fn test_set_provider() {
        let mut handler = Dns01Handler::new();

        let result = handler.setup("example.com", "test".to_string()).await;
        assert!(result.is_err());

        let provider = Box::new(MockDnsProvider::new());
        handler.set_provider(provider);

        let result = handler.setup("example.com", "test".to_string()).await;
        assert!(result.is_ok());
    }
}
