//! ACME client implementation using instant-acme

use crate::{
    cert::{Certificate, CsrGenerator, KeyType},
    config::{AcmeConfig, ChallengeType},
    dns01::Dns01Handler,
    http01::Http01Handler,
    AcmeError, Result,
};
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType as AcmeChallengeType, Identifier, LetsEncrypt,
    NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// ACME client for managing certificates
pub struct AcmeClient {
    config: Arc<AcmeConfig>,
    http_handler: Option<Arc<Http01Handler>>,
    dns_handler: Option<Arc<Dns01Handler>>,
}

impl AcmeClient {
    /// Create a new ACME client
    pub fn new(config: AcmeConfig) -> Result<Self> {
        config.validate().map_err(AcmeError::Other)?;

        Ok(Self {
            config: Arc::new(config),
            http_handler: None,
            dns_handler: None,
        })
    }

    /// Set HTTP-01 challenge handler
    pub fn with_http01_handler(mut self, handler: Http01Handler) -> Self {
        self.http_handler = Some(Arc::new(handler));
        self
    }

    /// Set DNS-01 challenge handler
    pub fn with_dns01_handler(mut self, handler: Dns01Handler) -> Self {
        self.dns_handler = Some(Arc::new(handler));
        self
    }

    /// Request a new certificate
    pub async fn request_certificate(&self) -> Result<Certificate> {
        info!(
            "Requesting certificate for domains: {:?}",
            self.config.domains
        );

        // Create or load account
        let account = self.create_account().await?;

        // Create order
        let identifiers: Vec<Identifier> = self
            .config
            .domains
            .iter()
            .map(|d| Identifier::Dns(d.clone()))
            .collect();

        let mut order = account
            .new_order(&NewOrder::new(identifiers.as_slice()))
            .await
            .map_err(|e| AcmeError::Protocol(format!("Order creation failed: {}", e)))?;

        // Get authorizations and complete each one
        let mut authorizations = order.authorizations();
        while let Some(authz_result) = authorizations.next().await {
            let mut authz = authz_result
                .map_err(|e| AcmeError::Protocol(format!("Failed to get authorization: {}", e)))?;

            // Check if already valid
            if matches!(authz.status, AuthorizationStatus::Valid) {
                debug!("Authorization already valid for {:?}", authz.identifier());
                continue;
            }

            // Find the appropriate challenge and handle it
            match self.config.challenge_type {
                ChallengeType::Http01 => {
                    let mut challenge =
                        authz.challenge(AcmeChallengeType::Http01).ok_or_else(|| {
                            AcmeError::ChallengeFailed(
                                "HTTP-01 challenge type not available".to_string(),
                            )
                        })?;

                    let token = challenge.token.clone();
                    let key_auth = challenge.key_authorization().as_str().to_owned();

                    self.setup_http01_challenge_data(&token, key_auth).await?;

                    challenge.set_ready().await.map_err(|e| {
                        AcmeError::Protocol(format!("Failed to set challenge ready: {}", e))
                    })?;

                    // Cleanup HTTP challenge after setting ready
                    self.cleanup_http01_challenge(&token).await;
                }
                ChallengeType::Dns01 => {
                    let mut challenge =
                        authz.challenge(AcmeChallengeType::Dns01).ok_or_else(|| {
                            AcmeError::ChallengeFailed(
                                "DNS-01 challenge type not available".to_string(),
                            )
                        })?;

                    let dns_value = challenge.key_authorization().dns_value();
                    let identifier = challenge.identifier();
                    let domain = match identifier.identifier {
                        Identifier::Dns(ref d) => d.clone(),
                        _ => {
                            return Err(AcmeError::ChallengeFailed(
                                "Non-DNS identifier for DNS-01 challenge".to_string(),
                            ))
                        }
                    };

                    self.setup_dns01_challenge_data(&domain, dns_value.clone())
                        .await?;

                    challenge.set_ready().await.map_err(|e| {
                        AcmeError::Protocol(format!("Failed to set challenge ready: {}", e))
                    })?;

                    // Cleanup DNS challenge after setting ready
                    self.cleanup_dns01_challenge(&domain).await;
                }
            }
        }

        // Wait for the order to become ready
        let retry_policy = RetryPolicy::default();
        let order_status = order
            .poll_ready(&retry_policy)
            .await
            .map_err(|e| AcmeError::Protocol(format!("Failed polling order readiness: {}", e)))?;

        if order_status != OrderStatus::Ready {
            return Err(AcmeError::Protocol(format!(
                "Order not ready after polling, status: {:?}",
                order_status
            )));
        }

        info!("Authorization validated successfully");

        // Generate CSR
        let key_type = KeyType::default();
        let (csr_pem, key_pem) = CsrGenerator::generate(self.config.domains.clone(), key_type)?;

        // Parse CSR DER from PEM
        let (_, csr_der) = pem_rfc7468::decode_vec(csr_pem.as_bytes())
            .map_err(|e| AcmeError::Other(format!("Failed to parse CSR PEM: {}", e)))?;

        // Finalize order with the DER-encoded CSR
        order
            .finalize_csr(&csr_der)
            .await
            .map_err(|e| AcmeError::Protocol(format!("Order finalization failed: {}", e)))?;

        // Wait for certificate — poll_certificate handles retries internally
        let cert_chain_pem = order
            .poll_certificate(&retry_policy)
            .await
            .map_err(|e| AcmeError::Protocol(format!("Certificate download failed: {}", e)))?;

        // Parse certificate to extract metadata
        let cert = Certificate::from_pem(cert_chain_pem.clone(), key_pem)?;

        info!("Certificate obtained successfully");

        Ok(Certificate::new(
            cert_chain_pem,
            cert.key_pem,
            String::new(),
            self.config.domains.clone(),
            cert.expires_at,
            cert.not_before,
        ))
    }

    /// Create or load ACME account
    async fn create_account(&self) -> Result<Account> {
        debug!("Creating ACME account for {}", self.config.email);

        let url = if self.config.directory_url.contains("staging") {
            LetsEncrypt::Staging.url()
        } else {
            LetsEncrypt::Production.url()
        };

        // instant-acme 0.8: Account::builder() returns a Result<AccountBuilder>
        let (account, _credentials) = Account::builder()
            .map_err(|e| AcmeError::Protocol(format!("Failed to create account builder: {}", e)))?
            .create(
                &NewAccount {
                    contact: &[&format!("mailto:{}", self.config.email)],
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                url.to_owned(),
                None,
            )
            .await
            .map_err(|e| AcmeError::Protocol(format!("Account creation failed: {}", e)))?;

        info!("ACME account created/loaded");
        Ok(account)
    }

    /// Setup HTTP-01 challenge with token and key authorization data
    async fn setup_http01_challenge_data(
        &self,
        token: &str,
        key_authorization: String,
    ) -> Result<()> {
        let handler = self.http_handler.as_ref().ok_or_else(|| {
            AcmeError::ChallengeFailed("HTTP-01 handler not configured".to_string())
        })?;

        handler
            .add_challenge(token.to_owned(), key_authorization)
            .await;

        info!("HTTP-01 challenge setup for token: {}", token);

        Ok(())
    }

    /// Setup DNS-01 challenge with domain and pre-computed DNS TXT record value
    async fn setup_dns01_challenge_data(&self, domain: &str, dns_value: String) -> Result<()> {
        let handler = self.dns_handler.as_ref().ok_or_else(|| {
            AcmeError::ChallengeFailed("DNS-01 handler not configured".to_string())
        })?;

        handler.setup(domain, dns_value.clone()).await?;

        info!("DNS-01 challenge setup for domain: {}", domain);

        // Wait for DNS propagation
        handler
            .wait_for_propagation(domain, &dns_value, 10, 5)
            .await?;

        Ok(())
    }

    /// Cleanup HTTP-01 challenge
    async fn cleanup_http01_challenge(&self, token: &str) {
        if let Some(handler) = &self.http_handler {
            handler.remove_challenge(token).await;
            debug!("HTTP-01 challenge cleaned up for token: {}", token);
        }
    }

    /// Cleanup DNS-01 challenge
    async fn cleanup_dns01_challenge(&self, domain: &str) {
        if let Some(handler) = &self.dns_handler {
            if let Err(e) = handler.cleanup(domain).await {
                warn!("Failed to cleanup DNS-01 challenge for {}: {}", domain, e);
            } else {
                debug!("DNS-01 challenge cleaned up for domain: {}", domain);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> AcmeConfig {
        AcmeConfig::new(
            "test@example.com".to_string(),
            vec!["example.com".to_string()],
        )
        .staging()
    }

    #[test]
    fn test_client_creation() {
        let config = create_test_config();
        let client = AcmeClient::new(config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_client_with_handlers() {
        let config = create_test_config();
        let http_handler = Http01Handler::new();
        let dns_handler = Dns01Handler::new();

        let client = AcmeClient::new(config)
            .unwrap()
            .with_http01_handler(http_handler)
            .with_dns01_handler(dns_handler);

        assert!(client.http_handler.is_some());
        assert!(client.dns_handler.is_some());
    }

    #[test]
    fn test_client_invalid_config() {
        let mut config = create_test_config();
        config.email = String::new();

        let result = AcmeClient::new(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_client_no_domains() {
        let config = AcmeConfig::new("test@example.com".to_string(), vec![]);

        let result = AcmeClient::new(config);
        assert!(result.is_err());
    }
}
