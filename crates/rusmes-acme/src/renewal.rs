//! Automatic certificate renewal

use crate::{AcmeClient, AcmeConfig, AcmeError, Result};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

/// Renewal manager for automatic certificate renewal
pub struct RenewalManager {
    client: Arc<AcmeClient>,
    config: Arc<AcmeConfig>,
    running: Arc<RwLock<bool>>,
}

impl RenewalManager {
    /// Create a new renewal manager
    pub fn new(client: AcmeClient, config: AcmeConfig) -> Self {
        Self {
            client: Arc::new(client),
            config: Arc::new(config),
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start automatic renewal
    pub async fn start(&self) -> Result<()> {
        let mut running = self.running.write().await;
        if *running {
            return Err(AcmeError::Other(
                "Renewal manager already running".to_string(),
            ));
        }
        *running = true;
        drop(running);

        info!("Starting automatic certificate renewal");

        let client = self.client.clone();
        let config = self.config.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            Self::renewal_loop(client, config, running).await;
        });

        Ok(())
    }

    /// Stop automatic renewal
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("Stopping automatic certificate renewal");
    }

    /// Check if renewal is needed
    pub async fn check_renewal(&self) -> Result<bool> {
        info!("Checking if certificate renewal is needed");

        // Load current certificate
        let cert_result =
            crate::cert::Certificate::load(&self.config.cert_path, &self.config.key_path).await;

        match cert_result {
            Ok(cert) => {
                let should_renew = cert.should_renew(self.config.renewal_days_before_expiry);
                let days_left = cert.days_until_expiry();

                if should_renew {
                    warn!("Certificate expires in {} days, renewal needed", days_left);
                } else {
                    info!("Certificate valid for {} days", days_left);
                }

                Ok(should_renew)
            }
            Err(e) => {
                warn!("Failed to load certificate: {}, will request new one", e);
                Ok(true)
            }
        }
    }

    /// Perform certificate renewal
    pub async fn renew(&self) -> Result<()> {
        info!("Renewing certificate");

        // Request new certificate
        let cert = self.client.request_certificate().await?;

        // Save certificate
        cert.save(&self.config.cert_path, &self.config.key_path)
            .await?;

        info!("Certificate renewed successfully");

        // Trigger reload (in real implementation, would notify server)
        self.reload_certificate().await?;

        Ok(())
    }

    /// Reload certificate without restarting server
    async fn reload_certificate(&self) -> Result<()> {
        info!("Reloading certificate");
        // In real implementation, would send signal to server to reload certs
        Ok(())
    }

    /// Renewal loop
    async fn renewal_loop(
        client: Arc<AcmeClient>,
        config: Arc<AcmeConfig>,
        running: Arc<RwLock<bool>>,
    ) {
        let mut check_interval = interval(Duration::from_secs(config.renewal_check_interval));

        loop {
            check_interval.tick().await;

            // Check if still running
            let is_running = *running.read().await;
            if !is_running {
                info!("Renewal loop stopping");
                break;
            }

            // Check if renewal is needed
            let cert_result =
                crate::cert::Certificate::load(&config.cert_path, &config.key_path).await;

            match cert_result {
                Ok(cert) => {
                    if cert.should_renew(config.renewal_days_before_expiry) {
                        info!("Certificate renewal needed");

                        // Attempt renewal
                        match client.request_certificate().await {
                            Ok(new_cert) => {
                                if let Err(e) =
                                    new_cert.save(&config.cert_path, &config.key_path).await
                                {
                                    error!("Failed to save renewed certificate: {}", e);
                                } else {
                                    info!("Certificate renewed successfully");
                                }
                            }
                            Err(e) => {
                                error!("Failed to renew certificate: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to load certificate: {}", e);
                }
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

    #[tokio::test]
    async fn test_renewal_manager_creation() {
        let config = create_test_config();
        let client = AcmeClient::new(config.clone()).unwrap();
        let manager = RenewalManager::new(client, config);

        let running = *manager.running.read().await;
        assert!(!running);
    }

    #[tokio::test]
    async fn test_start_stop() {
        let config = create_test_config();
        let client = AcmeClient::new(config.clone()).unwrap();
        let manager = RenewalManager::new(client, config);

        manager.start().await.unwrap();
        let running = *manager.running.read().await;
        assert!(running);

        manager.stop().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let running = *manager.running.read().await;
        assert!(!running);
    }

    #[tokio::test]
    async fn test_check_renewal_no_cert() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");

        let config = create_test_config().cert_paths(
            cert_path.to_str().unwrap().to_string(),
            key_path.to_str().unwrap().to_string(),
        );

        let client = AcmeClient::new(config.clone()).unwrap();
        let manager = RenewalManager::new(client, config);

        let should_renew = manager.check_renewal().await.unwrap();
        assert!(should_renew); // Should renew if cert doesn't exist
    }
}
