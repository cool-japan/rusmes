//! PAM (Pluggable Authentication Modules) authentication backend

use crate::AuthBackend;
use async_trait::async_trait;
use pam::Authenticator;
use rusmes_proto::Username;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for PAM authentication
#[derive(Debug, Clone)]
pub struct PamConfig {
    /// PAM service name (default: "rusmes")
    pub service_name: String,
}

impl Default for PamConfig {
    fn default() -> Self {
        Self {
            service_name: "rusmes".to_string(),
        }
    }
}

impl PamConfig {
    /// Create a new PamConfig with a specific service name
    pub fn new(service_name: String) -> Self {
        Self { service_name }
    }

    /// Create a PamConfig with the default service name
    pub fn with_default_service() -> Self {
        Self::default()
    }
}

/// PAM authentication backend
pub struct PamAuthBackend {
    config: PamConfig,
    user_cache: Arc<RwLock<std::collections::HashSet<String>>>,
}

impl PamAuthBackend {
    /// Create a new PAM authentication backend with the given configuration
    pub fn new(config: PamConfig) -> Self {
        Self {
            config,
            user_cache: Arc::new(RwLock::new(std::collections::HashSet::new())),
        }
    }

    /// Create a new PAM authentication backend with default configuration
    pub fn with_default_service() -> Self {
        Self::new(PamConfig::default())
    }
}

#[async_trait]
impl AuthBackend for PamAuthBackend {
    async fn authenticate(&self, username: &Username, password: &str) -> anyhow::Result<bool> {
        let username_str = username.to_string();
        let password_str = password.to_string();
        let service_name = self.config.service_name.clone();

        // Run PAM authentication in blocking thread pool
        let result: bool = tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let mut authenticator = Authenticator::with_password(&service_name)
                .map_err(|e| anyhow::anyhow!("PAM initialization failed: {}", e))?;

            authenticator
                .get_handler()
                .set_credentials(&username_str, &password_str);

            match authenticator.authenticate() {
                Ok(_) => Ok::<bool, anyhow::Error>(true),
                Err(_) => {
                    // PAM authentication failed (wrong password, user not found, etc.)
                    Ok::<bool, anyhow::Error>(false)
                }
            }
        })
        .await??;

        // Cache successful authentication
        if result {
            self.user_cache.write().await.insert(username.to_string());
        }

        Ok(result)
    }

    async fn verify_identity(&self, username: &Username) -> anyhow::Result<bool> {
        // Check cache first
        {
            let cache = self.user_cache.read().await;
            if cache.contains(&username.to_string()) {
                return Ok(true);
            }
        }

        // Check if user exists in system
        let username_str = username.to_string();
        let exists = tokio::task::spawn_blocking(move || {
            #[cfg(unix)]
            {
                use std::ffi::CString;
                let c_username = match CString::new(username_str.as_str()) {
                    Ok(s) => s,
                    Err(_) => return false,
                };

                unsafe {
                    let pwd = libc::getpwnam(c_username.as_ptr());
                    !pwd.is_null()
                }
            }

            #[cfg(not(unix))]
            {
                false
            }
        })
        .await?;

        if exists {
            self.user_cache.write().await.insert(username.to_string());
        }

        Ok(exists)
    }

    async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
        // PAM doesn't provide user enumeration capability
        // Return cached users only
        let cache = self.user_cache.read().await;
        let users: Vec<Username> = cache
            .iter()
            .filter_map(|u| Username::new(u.clone()).ok())
            .collect();
        Ok(users)
    }

    async fn create_user(&self, _username: &Username, _password: &str) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "PAM backend does not support user creation (system users must be created via useradd or similar)"
        ))
    }

    async fn delete_user(&self, _username: &Username) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "PAM backend does not support user deletion (system users must be deleted via userdel or similar)"
        ))
    }

    async fn change_password(
        &self,
        _username: &Username,
        _new_password: &str,
    ) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "PAM backend does not support password changes (use passwd command or similar)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pam_config_default() {
        let config = PamConfig::default();
        assert_eq!(config.service_name, "rusmes");
    }

    #[test]
    fn test_pam_config_custom() {
        let config = PamConfig::new("custom-service".to_string());
        assert_eq!(config.service_name, "custom-service");
    }

    #[test]
    fn test_pam_config_with_default_service() {
        let config = PamConfig::with_default_service();
        assert_eq!(config.service_name, "rusmes");
    }

    #[test]
    fn test_pam_backend_creation() {
        let config = PamConfig::default();
        let backend = PamAuthBackend::new(config);
        assert_eq!(backend.config.service_name, "rusmes");
    }

    #[test]
    fn test_pam_backend_with_default_service() {
        let backend = PamAuthBackend::with_default_service();
        assert_eq!(backend.config.service_name, "rusmes");
    }

    #[tokio::test]
    async fn test_pam_backend_cache_empty_on_creation() {
        let backend = PamAuthBackend::with_default_service();
        let cache = backend.user_cache.read().await;
        assert!(cache.is_empty());
    }

    #[tokio::test]
    async fn test_create_user_not_supported() {
        let backend = PamAuthBackend::with_default_service();
        let username = Username::new("newuser".to_string()).unwrap();
        let result = backend.create_user(&username, "password").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not support user creation"));
    }

    #[tokio::test]
    async fn test_delete_user_not_supported() {
        let backend = PamAuthBackend::with_default_service();
        let username = Username::new("user".to_string()).unwrap();
        let result = backend.delete_user(&username).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not support user deletion"));
    }

    #[tokio::test]
    async fn test_change_password_not_supported() {
        let backend = PamAuthBackend::with_default_service();
        let username = Username::new("user".to_string()).unwrap();
        let result = backend.change_password(&username, "newpass").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not support password changes"));
    }

    #[tokio::test]
    async fn test_list_users_returns_cached() {
        let backend = PamAuthBackend::with_default_service();

        // Insert some users into cache
        backend.user_cache.write().await.insert("user1".to_string());
        backend.user_cache.write().await.insert("user2".to_string());

        let users = backend.list_users().await.unwrap();
        assert_eq!(users.len(), 2);

        let usernames: Vec<String> = users.iter().map(|u| u.to_string()).collect();
        assert!(usernames.contains(&"user1".to_string()));
        assert!(usernames.contains(&"user2".to_string()));
    }

    #[tokio::test]
    async fn test_verify_identity_cache() {
        let backend = PamAuthBackend::with_default_service();
        let username = Username::new("cached_user".to_string()).unwrap();

        // Insert into cache
        backend
            .user_cache
            .write()
            .await
            .insert("cached_user".to_string());

        // Should return true from cache
        let result = backend.verify_identity(&username).await.unwrap();
        assert!(result);
    }

    #[test]
    fn test_multiple_pam_configs() {
        let config1 = PamConfig::new("service1".to_string());
        let config2 = PamConfig::new("service2".to_string());
        let config3 = PamConfig::default();

        assert_eq!(config1.service_name, "service1");
        assert_eq!(config2.service_name, "service2");
        assert_eq!(config3.service_name, "rusmes");
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let backend = PamAuthBackend::with_default_service();

        // Initially empty
        {
            let cache = backend.user_cache.read().await;
            assert_eq!(cache.len(), 0);
        }

        // Add users
        {
            let mut cache = backend.user_cache.write().await;
            cache.insert("user1".to_string());
            cache.insert("user2".to_string());
            cache.insert("user3".to_string());
        }

        // Check size
        {
            let cache = backend.user_cache.read().await;
            assert_eq!(cache.len(), 3);
            assert!(cache.contains("user1"));
            assert!(cache.contains("user2"));
            assert!(cache.contains("user3"));
        }
    }

    #[tokio::test]
    async fn test_list_users_empty_cache() {
        let backend = PamAuthBackend::with_default_service();
        let users = backend.list_users().await.unwrap();
        assert!(users.is_empty());
    }

    #[test]
    fn test_pam_config_clone() {
        let config1 = PamConfig::new("test-service".to_string());
        let config2 = config1.clone();
        assert_eq!(config1.service_name, config2.service_name);
    }
}
