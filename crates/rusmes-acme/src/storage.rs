//! Certificate storage

use crate::Result;
use std::path::Path;
use tracing::{debug, info};

/// Certificate storage manager
pub struct CertificateStorage {
    cert_dir: String,
}

impl CertificateStorage {
    /// Create a new certificate storage
    pub fn new(cert_dir: String) -> Result<Self> {
        std::fs::create_dir_all(&cert_dir)?;
        Ok(Self { cert_dir })
    }

    /// Save certificate
    pub async fn save_certificate(
        &self,
        domain: &str,
        cert_pem: &str,
        key_pem: &str,
        chain_pem: &str,
    ) -> Result<()> {
        let cert_path = self.cert_path(domain);
        let key_path = self.key_path(domain);
        let chain_path = self.chain_path(domain);

        tokio::fs::write(&cert_path, cert_pem).await?;
        tokio::fs::write(&key_path, key_pem).await?;
        tokio::fs::write(&chain_path, chain_pem).await?;

        // Set appropriate permissions (read-only for cert, restricted for key)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let cert_perms = std::fs::Permissions::from_mode(0o644);
            let key_perms = std::fs::Permissions::from_mode(0o600);

            std::fs::set_permissions(&cert_path, cert_perms.clone())?;
            std::fs::set_permissions(&key_path, key_perms)?;
            std::fs::set_permissions(&chain_path, cert_perms)?;
        }

        info!("Certificate saved for domain: {}", domain);
        Ok(())
    }

    /// Load certificate
    pub async fn load_certificate(&self, domain: &str) -> Result<(String, String, String)> {
        let cert_path = self.cert_path(domain);
        let key_path = self.key_path(domain);
        let chain_path = self.chain_path(domain);

        let cert_pem = tokio::fs::read_to_string(&cert_path).await?;
        let key_pem = tokio::fs::read_to_string(&key_path).await?;
        let chain_pem = tokio::fs::read_to_string(&chain_path).await?;

        debug!("Certificate loaded for domain: {}", domain);
        Ok((cert_pem, key_pem, chain_pem))
    }

    /// Check if certificate exists
    pub fn certificate_exists(&self, domain: &str) -> bool {
        let cert_path = self.cert_path(domain);
        let key_path = self.key_path(domain);

        Path::new(&cert_path).exists() && Path::new(&key_path).exists()
    }

    /// Delete certificate
    pub async fn delete_certificate(&self, domain: &str) -> Result<()> {
        let cert_path = self.cert_path(domain);
        let key_path = self.key_path(domain);
        let chain_path = self.chain_path(domain);

        if Path::new(&cert_path).exists() {
            tokio::fs::remove_file(&cert_path).await?;
        }
        if Path::new(&key_path).exists() {
            tokio::fs::remove_file(&key_path).await?;
        }
        if Path::new(&chain_path).exists() {
            tokio::fs::remove_file(&chain_path).await?;
        }

        info!("Certificate deleted for domain: {}", domain);
        Ok(())
    }

    /// List all domains with certificates
    pub async fn list_domains(&self) -> Result<Vec<String>> {
        let mut domains = Vec::new();

        let mut entries = tokio::fs::read_dir(&self.cert_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if let Some(file_name) = path.file_name() {
                if let Some(name) = file_name.to_str() {
                    if name.ends_with(".crt") {
                        let domain = name.trim_end_matches(".crt");
                        domains.push(domain.to_string());
                    }
                }
            }
        }

        Ok(domains)
    }

    fn cert_path(&self, domain: &str) -> String {
        format!("{}/{}.crt", self.cert_dir, domain)
    }

    fn key_path(&self, domain: &str) -> String {
        format!("{}/{}.key", self.cert_dir, domain)
    }

    fn chain_path(&self, domain: &str) -> String {
        format!("{}/{}.chain", self.cert_dir, domain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_storage_creation() {
        let dir = tempdir().unwrap();
        let storage = CertificateStorage::new(dir.path().to_str().unwrap().to_string());
        assert!(storage.is_ok());
    }

    #[tokio::test]
    async fn test_save_load_certificate() {
        let dir = tempdir().unwrap();
        let storage = CertificateStorage::new(dir.path().to_str().unwrap().to_string()).unwrap();

        let domain = "example.com";
        let cert_pem = "TEST_CERT";
        let key_pem = "TEST_KEY";
        let chain_pem = "TEST_CHAIN";

        storage
            .save_certificate(domain, cert_pem, key_pem, chain_pem)
            .await
            .unwrap();

        assert!(storage.certificate_exists(domain));

        let (loaded_cert, loaded_key, loaded_chain) =
            storage.load_certificate(domain).await.unwrap();

        assert_eq!(loaded_cert, cert_pem);
        assert_eq!(loaded_key, key_pem);
        assert_eq!(loaded_chain, chain_pem);
    }

    #[tokio::test]
    async fn test_delete_certificate() {
        let dir = tempdir().unwrap();
        let storage = CertificateStorage::new(dir.path().to_str().unwrap().to_string()).unwrap();

        let domain = "example.com";
        storage
            .save_certificate(domain, "cert", "key", "chain")
            .await
            .unwrap();

        assert!(storage.certificate_exists(domain));

        storage.delete_certificate(domain).await.unwrap();

        assert!(!storage.certificate_exists(domain));
    }

    #[tokio::test]
    async fn test_list_domains() {
        let dir = tempdir().unwrap();
        let storage = CertificateStorage::new(dir.path().to_str().unwrap().to_string()).unwrap();

        storage
            .save_certificate("example1.com", "cert", "key", "chain")
            .await
            .unwrap();
        storage
            .save_certificate("example2.com", "cert", "key", "chain")
            .await
            .unwrap();

        let domains = storage.list_domains().await.unwrap();
        assert_eq!(domains.len(), 2);
        assert!(domains.contains(&"example1.com".to_string()));
        assert!(domains.contains(&"example2.com".to_string()));
    }

    #[tokio::test]
    async fn test_certificate_not_exists() {
        let dir = tempdir().unwrap();
        let storage = CertificateStorage::new(dir.path().to_str().unwrap().to_string()).unwrap();

        assert!(!storage.certificate_exists("nonexistent.com"));
    }
}
