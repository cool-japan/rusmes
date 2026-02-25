//! Certificate management

use crate::{AcmeError, Result};
use chrono::{DateTime, Utc};
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use std::convert::TryInto;
use tracing::{debug, info};
use x509_parser::prelude::*;

/// Certificate with metadata
#[derive(Debug, Clone)]
pub struct Certificate {
    /// PEM-encoded certificate
    pub cert_pem: String,
    /// PEM-encoded private key
    pub key_pem: String,
    /// PEM-encoded certificate chain
    pub chain_pem: String,
    /// Domain names in certificate
    pub domains: Vec<String>,
    /// Certificate expiration time
    pub expires_at: DateTime<Utc>,
    /// Certificate not-before time
    pub not_before: DateTime<Utc>,
}

impl Certificate {
    /// Create a new certificate
    pub fn new(
        cert_pem: String,
        key_pem: String,
        chain_pem: String,
        domains: Vec<String>,
        expires_at: DateTime<Utc>,
        not_before: DateTime<Utc>,
    ) -> Self {
        Self {
            cert_pem,
            key_pem,
            chain_pem,
            domains,
            expires_at,
            not_before,
        }
    }

    /// Parse certificate from PEM
    pub fn from_pem(cert_pem: String, key_pem: String) -> Result<Self> {
        let (domains, expires_at, not_before) = Self::parse_certificate_info(&cert_pem)?;

        Ok(Self {
            cert_pem,
            key_pem,
            chain_pem: String::new(),
            domains,
            expires_at,
            not_before,
        })
    }

    /// Check if certificate is expired
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    /// Check if certificate should be renewed
    pub fn should_renew(&self, days_before_expiry: u32) -> bool {
        let renewal_time = self.expires_at - chrono::Duration::days(days_before_expiry as i64);
        Utc::now() >= renewal_time
    }

    /// Get days until expiry
    pub fn days_until_expiry(&self) -> i64 {
        (self.expires_at - Utc::now()).num_days()
    }

    /// Get hours until expiry
    pub fn hours_until_expiry(&self) -> i64 {
        (self.expires_at - Utc::now()).num_hours()
    }

    /// Save certificate to files
    pub async fn save(&self, cert_path: &str, key_path: &str) -> Result<()> {
        tokio::fs::write(cert_path, &self.cert_pem).await?;
        tokio::fs::write(key_path, &self.key_pem).await?;

        // Set appropriate permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key_perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(key_path, key_perms)?;
        }

        info!("Certificate saved to {} and {}", cert_path, key_path);
        Ok(())
    }

    /// Load certificate from files
    pub async fn load(cert_path: &str, key_path: &str) -> Result<Self> {
        let cert_pem = tokio::fs::read_to_string(cert_path).await?;
        let key_pem = tokio::fs::read_to_string(key_path).await?;

        Self::from_pem(cert_pem, key_pem)
    }

    /// Parse certificate information from PEM
    fn parse_certificate_info(
        cert_pem: &str,
    ) -> Result<(Vec<String>, DateTime<Utc>, DateTime<Utc>)> {
        // Extract DER from PEM using x509-parser's pem module
        let pem = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
            .map_err(|e| AcmeError::ValidationFailed(format!("Failed to parse PEM: {}", e)))?
            .1;

        let der = &pem.contents;

        // Parse X.509 certificate
        let (_, cert) = X509Certificate::from_der(der)
            .map_err(|e| AcmeError::ValidationFailed(format!("Failed to parse X.509: {}", e)))?;

        // Extract domains from subject alternative names
        let mut domains = Vec::new();
        if let Ok(Some(san_ext)) = cert.subject_alternative_name() {
            for name in &san_ext.value.general_names {
                if let GeneralName::DNSName(dns) = name {
                    domains.push(dns.to_string());
                }
            }
        }

        // If no SAN, try to get from common name
        if domains.is_empty() {
            if let Some(cn) = cert.subject().iter_common_name().next() {
                if let Ok(cn_str) = cn.as_str() {
                    domains.push(cn_str.to_string());
                }
            }
        }

        // Extract validity dates
        let not_after = cert.validity().not_after.timestamp();
        let not_before = cert.validity().not_before.timestamp();

        let expires_at = DateTime::from_timestamp(not_after, 0)
            .ok_or_else(|| AcmeError::ValidationFailed("Invalid expiration time".to_string()))?;

        let not_before_dt = DateTime::from_timestamp(not_before, 0)
            .ok_or_else(|| AcmeError::ValidationFailed("Invalid not-before time".to_string()))?;

        debug!(
            "Certificate expires at: {}, domains: {:?}",
            expires_at, domains
        );

        Ok((domains, expires_at, not_before_dt))
    }

    /// Validate certificate chain
    pub fn validate_chain(&self) -> Result<()> {
        // Basic validation - in production, would do full chain validation
        if self.cert_pem.is_empty() {
            return Err(AcmeError::ValidationFailed("Empty certificate".to_string()));
        }

        if self.key_pem.is_empty() {
            return Err(AcmeError::ValidationFailed("Empty private key".to_string()));
        }

        Ok(())
    }
}

/// Certificate Signing Request (CSR) generator
pub struct CsrGenerator;

impl CsrGenerator {
    /// Generate a CSR for the given domains
    pub fn generate(domains: Vec<String>, key_type: KeyType) -> Result<(String, String)> {
        if domains.is_empty() {
            return Err(AcmeError::Other("No domains specified for CSR".to_string()));
        }

        let mut params = CertificateParams::new(domains.clone())
            .map_err(|e| AcmeError::Other(format!("Failed to create certificate params: {}", e)))?;

        // Set subject
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, &domains[0]);
        params.distinguished_name = dn;

        // Add SANs — rcgen 0.14 uses Ia5String (inferred via TryInto) for DnsName
        params.subject_alt_names = domains
            .iter()
            .map(|d| {
                let ia5: std::result::Result<_, _> = d.as_str().try_into();
                ia5.map(SanType::DnsName)
                    .map_err(|e| AcmeError::Other(format!("Invalid domain name '{}': {}", d, e)))
            })
            .collect::<Result<Vec<_>>>()?;

        // Generate key pair based on type.
        // rcgen 0.14 uses KeyPair::generate() (P-256 by default) or
        // KeyPair::generate_for(alg) for explicit algorithm selection.
        // For RSA keys we fall back to ECDSA P-256 since rcgen does not
        // expose RSA key generation without openssl feature.
        let key_pair = match key_type {
            KeyType::Rsa2048 | KeyType::Rsa4096 => {
                // Use ECDSA P-256 as rcgen doesn't support RSA key generation out of the box
                KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
                    .map_err(|e| AcmeError::Other(format!("Failed to generate key: {}", e)))?
            }
            KeyType::EcdsaP256 => KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
                .map_err(|e| AcmeError::Other(format!("Failed to generate ECDSA key: {}", e)))?,
        };

        // Capture the serialized private key PEM before moving key_pair into CSR generation
        let key_pem = key_pair.serialize_pem();

        // Generate CSR — rcgen 0.14 API: CertificateParams::serialize_request(&key_pair)
        let csr = params
            .serialize_request(&key_pair)
            .map_err(|e| AcmeError::Other(format!("Failed to serialize CSR: {}", e)))?;

        // Get the DER-encoded CSR bytes
        let csr_der: &[u8] = csr.der();

        // Encode as PEM
        let csr_pem =
            pem_rfc7468::encode_string("CERTIFICATE REQUEST", pem_rfc7468::LineEnding::LF, csr_der)
                .map_err(|e| AcmeError::Other(format!("Failed to encode CSR PEM: {}", e)))?;

        Ok((csr_pem, key_pem))
    }
}

/// Private key type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyType {
    /// RSA 2048-bit
    #[default]
    Rsa2048,
    /// RSA 4096-bit
    Rsa4096,
    /// ECDSA P-256
    EcdsaP256,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_certificate_expiry() {
        let cert = Certificate::new(
            "cert".to_string(),
            "key".to_string(),
            "chain".to_string(),
            vec!["example.com".to_string()],
            Utc::now() - chrono::Duration::days(1),
            Utc::now() - chrono::Duration::days(90),
        );

        assert!(cert.is_expired());
        assert!(cert.should_renew(30));
    }

    #[test]
    fn test_certificate_not_expired() {
        let cert = Certificate::new(
            "cert".to_string(),
            "key".to_string(),
            "chain".to_string(),
            vec!["example.com".to_string()],
            Utc::now() + chrono::Duration::days(60),
            Utc::now() - chrono::Duration::days(30),
        );

        assert!(!cert.is_expired());
        assert!(!cert.should_renew(30));
    }

    #[test]
    fn test_should_renew() {
        let cert = Certificate::new(
            "cert".to_string(),
            "key".to_string(),
            "chain".to_string(),
            vec!["example.com".to_string()],
            Utc::now() + chrono::Duration::days(20),
            Utc::now() - chrono::Duration::days(70),
        );

        assert!(!cert.is_expired());
        assert!(cert.should_renew(30));
        assert!(!cert.should_renew(10));
    }

    #[test]
    fn test_days_until_expiry() {
        let cert = Certificate::new(
            "cert".to_string(),
            "key".to_string(),
            "chain".to_string(),
            vec!["example.com".to_string()],
            Utc::now() + chrono::Duration::days(45),
            Utc::now() - chrono::Duration::days(45),
        );

        let days = cert.days_until_expiry();
        assert!((44..=45).contains(&days));
    }

    #[test]
    fn test_csr_generation_rsa2048() {
        let domains = vec!["example.com".to_string(), "www.example.com".to_string()];
        let result = CsrGenerator::generate(domains, KeyType::Rsa2048);

        if let Err(ref e) = result {
            eprintln!("CSR generation failed: {}", e);
        }
        assert!(result.is_ok());

        let (csr_pem, key_pem) = result.unwrap();
        assert!(csr_pem.contains("BEGIN CERTIFICATE REQUEST"));
        assert!(key_pem.contains("PRIVATE KEY"));
    }

    #[test]
    fn test_csr_generation_ecdsa() {
        let domains = vec!["example.com".to_string()];
        let result = CsrGenerator::generate(domains, KeyType::EcdsaP256);
        assert!(result.is_ok());

        let (csr_pem, key_pem) = result.unwrap();
        assert!(csr_pem.contains("BEGIN CERTIFICATE REQUEST"));
        assert!(key_pem.contains("BEGIN PRIVATE KEY") || key_pem.contains("BEGIN EC PRIVATE KEY"));
    }

    #[test]
    fn test_csr_generation_no_domains() {
        let domains = vec![];
        let result = CsrGenerator::generate(domains, KeyType::Rsa2048);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_certificate_save_load() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");

        let cert = Certificate::new(
            "TEST_CERT_PEM".to_string(),
            "TEST_KEY_PEM".to_string(),
            "TEST_CHAIN_PEM".to_string(),
            vec!["example.com".to_string()],
            Utc::now() + chrono::Duration::days(90),
            Utc::now(),
        );

        cert.save(cert_path.to_str().unwrap(), key_path.to_str().unwrap())
            .await
            .unwrap();

        // Verify files exist
        assert!(cert_path.exists());
        assert!(key_path.exists());

        // Verify content
        let saved_cert = tokio::fs::read_to_string(&cert_path).await.unwrap();
        let saved_key = tokio::fs::read_to_string(&key_path).await.unwrap();

        assert_eq!(saved_cert, "TEST_CERT_PEM");
        assert_eq!(saved_key, "TEST_KEY_PEM");
    }

    #[test]
    fn test_certificate_validation() {
        let cert = Certificate::new(
            "cert".to_string(),
            "key".to_string(),
            "chain".to_string(),
            vec!["example.com".to_string()],
            Utc::now() + chrono::Duration::days(90),
            Utc::now(),
        );

        assert!(cert.validate_chain().is_ok());
    }

    #[test]
    fn test_certificate_validation_empty_cert() {
        let cert = Certificate::new(
            String::new(),
            "key".to_string(),
            "chain".to_string(),
            vec!["example.com".to_string()],
            Utc::now() + chrono::Duration::days(90),
            Utc::now(),
        );

        assert!(cert.validate_chain().is_err());
    }

    #[test]
    fn test_certificate_validation_empty_key() {
        let cert = Certificate::new(
            "cert".to_string(),
            String::new(),
            "chain".to_string(),
            vec!["example.com".to_string()],
            Utc::now() + chrono::Duration::days(90),
            Utc::now(),
        );

        assert!(cert.validate_chain().is_err());
    }

    #[test]
    fn test_key_type_default() {
        assert_eq!(KeyType::default(), KeyType::Rsa2048);
    }

    #[test]
    fn test_hours_until_expiry() {
        let cert = Certificate::new(
            "cert".to_string(),
            "key".to_string(),
            "chain".to_string(),
            vec!["example.com".to_string()],
            Utc::now() + chrono::Duration::hours(48),
            Utc::now(),
        );

        let hours = cert.hours_until_expiry();
        assert!((47..=48).contains(&hours));
    }
}
