//! ACME configuration

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcmeConfig {
    /// ACME directory URL (e.g., Let's Encrypt production)
    pub directory_url: String,

    /// Contact email for ACME account
    pub email: String,

    /// Domains to obtain certificates for
    pub domains: Vec<String>,

    /// Challenge type (http-01 or dns-01)
    pub challenge_type: ChallengeType,

    /// Certificate storage path
    pub cert_path: String,

    /// Private key storage path
    pub key_path: String,

    /// Days before expiry to renew certificate
    pub renewal_days_before_expiry: u32,

    /// Enable automatic renewal
    pub auto_renewal: bool,

    /// Renewal check interval in seconds
    pub renewal_check_interval: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ChallengeType {
    Http01,
    Dns01,
}

impl Default for AcmeConfig {
    fn default() -> Self {
        Self {
            directory_url: "https://acme-v02.api.letsencrypt.org/directory".to_string(),
            email: "admin@example.com".to_string(),
            domains: vec![],
            challenge_type: ChallengeType::Http01,
            cert_path: "/etc/rusmes/certs/cert.pem".to_string(),
            key_path: "/etc/rusmes/certs/key.pem".to_string(),
            renewal_days_before_expiry: 30,
            auto_renewal: true,
            renewal_check_interval: 3600, // 1 hour
        }
    }
}

impl AcmeConfig {
    /// Create a new ACME config
    pub fn new(email: String, domains: Vec<String>) -> Self {
        Self {
            email,
            domains,
            ..Default::default()
        }
    }

    /// Use Let's Encrypt staging server
    pub fn staging(mut self) -> Self {
        self.directory_url = "https://acme-staging-v02.api.letsencrypt.org/directory".to_string();
        self
    }

    /// Set challenge type
    pub fn challenge_type(mut self, challenge_type: ChallengeType) -> Self {
        self.challenge_type = challenge_type;
        self
    }

    /// Set certificate paths
    pub fn cert_paths(mut self, cert_path: String, key_path: String) -> Self {
        self.cert_path = cert_path;
        self.key_path = key_path;
        self
    }

    /// Set renewal configuration
    pub fn renewal(mut self, days_before_expiry: u32, check_interval: u64) -> Self {
        self.renewal_days_before_expiry = days_before_expiry;
        self.renewal_check_interval = check_interval;
        self
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.email.is_empty() {
            return Err("Email is required".to_string());
        }

        if self.domains.is_empty() {
            return Err("At least one domain is required".to_string());
        }

        if self.renewal_days_before_expiry == 0 {
            return Err("Renewal days before expiry must be greater than 0".to_string());
        }

        if self.renewal_days_before_expiry > 90 {
            return Err("Renewal days before expiry should not exceed 90".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AcmeConfig::default();
        assert_eq!(config.challenge_type, ChallengeType::Http01);
        assert_eq!(config.renewal_days_before_expiry, 30);
        assert!(config.auto_renewal);
    }

    #[test]
    fn test_config_builder() {
        let config = AcmeConfig::new(
            "test@example.com".to_string(),
            vec!["example.com".to_string()],
        )
        .staging()
        .challenge_type(ChallengeType::Dns01)
        .renewal(15, 1800);

        assert!(config.directory_url.contains("staging"));
        assert_eq!(config.challenge_type, ChallengeType::Dns01);
        assert_eq!(config.renewal_days_before_expiry, 15);
        assert_eq!(config.renewal_check_interval, 1800);
    }

    #[test]
    fn test_validation_empty_email() {
        let config = AcmeConfig {
            email: "".to_string(),
            domains: vec!["example.com".to_string()],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_no_domains() {
        let config = AcmeConfig {
            email: "test@example.com".to_string(),
            domains: vec![],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_success() {
        let config = AcmeConfig::new(
            "test@example.com".to_string(),
            vec!["example.com".to_string()],
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_challenge_type_serialization() {
        let json = serde_json::to_string(&ChallengeType::Http01).unwrap();
        assert_eq!(json, "\"http01\"");

        let json = serde_json::to_string(&ChallengeType::Dns01).unwrap();
        assert_eq!(json, "\"dns01\"");
    }
}
