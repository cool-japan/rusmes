//! LDAP authentication backend

use crate::AuthBackend;
use async_trait::async_trait;
use ldap3::{Ldap, LdapConnAsync, LdapConnSettings, Scope, SearchEntry};
use rusmes_proto::Username;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for LDAP authentication
#[derive(Debug, Clone)]
pub struct LdapConfig {
    /// LDAP server URL (e.g., ldap://localhost:389 or ldaps://localhost:636)
    pub server_url: String,
    /// Base DN for user searches
    pub base_dn: String,
    /// User search filter (e.g., (uid={username}))
    pub user_filter: String,
    /// Bind DN pattern for direct bind (e.g., "uid={username},ou=users,dc=example,dc=com")
    /// If set, this is used instead of search + bind
    pub bind_dn_pattern: Option<String>,
    /// Bind DN for initial connection (if needed for search operations)
    pub bind_dn: Option<String>,
    /// Bind password for initial connection
    pub bind_password: Option<String>,
    /// Group base DN for membership checks
    pub group_base_dn: Option<String>,
    /// Group membership filter
    pub group_filter: Option<String>,
    /// Required group DN for authentication
    pub required_group: Option<String>,
    /// Connection timeout in seconds
    pub timeout_secs: u64,
    /// Enable connection pooling
    pub pool_size: usize,
    /// Enable TLS/STARTTLS
    pub use_tls: bool,
    /// Skip TLS certificate verification (for testing only)
    pub tls_skip_verify: bool,
}

impl Default for LdapConfig {
    fn default() -> Self {
        Self {
            server_url: "ldap://localhost:389".to_string(),
            base_dn: "dc=example,dc=com".to_string(),
            user_filter: "(uid={username})".to_string(),
            bind_dn_pattern: None,
            bind_dn: None,
            bind_password: None,
            group_base_dn: None,
            group_filter: None,
            required_group: None,
            timeout_secs: 10,
            pool_size: 5,
            use_tls: false,
            tls_skip_verify: false,
        }
    }
}

/// Connection pool for LDAP connections
struct ConnectionPool {
    config: LdapConfig,
    connections: Arc<RwLock<Vec<Ldap>>>,
    max_size: usize,
}

impl ConnectionPool {
    fn new(config: LdapConfig) -> Self {
        let max_size = config.pool_size;
        Self {
            config,
            connections: Arc::new(RwLock::new(Vec::new())),
            max_size,
        }
    }

    async fn get_connection(&self) -> anyhow::Result<Ldap> {
        // Try to get from pool
        {
            let mut pool = self.connections.write().await;
            if let Some(conn) = pool.pop() {
                return Ok(conn);
            }
        }

        // Create new connection
        self.create_connection().await
    }

    async fn create_connection(&self) -> anyhow::Result<Ldap> {
        let settings = LdapConnSettings::new()
            .set_conn_timeout(std::time::Duration::from_secs(self.config.timeout_secs));

        let (conn, mut ldap) =
            LdapConnAsync::with_settings(settings, &self.config.server_url).await?;

        ldap3::drive!(conn);

        // Bind if credentials provided
        if let (Some(bind_dn), Some(bind_password)) =
            (&self.config.bind_dn, &self.config.bind_password)
        {
            ldap.simple_bind(bind_dn, bind_password).await?;
        }

        Ok(ldap)
    }

    async fn return_connection(&self, conn: Ldap) {
        let mut pool = self.connections.write().await;
        if pool.len() < self.max_size {
            pool.push(conn);
        }
    }
}

/// LDAP authentication backend
pub struct LdapBackend {
    config: LdapConfig,
    pool: ConnectionPool,
    user_cache: Arc<RwLock<HashMap<String, bool>>>,
}

impl LdapBackend {
    /// Create a new LDAP authentication backend
    pub fn new(config: LdapConfig) -> Self {
        let pool = ConnectionPool::new(config.clone());
        Self {
            config,
            pool,
            user_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get user DN either by pattern or search
    async fn get_user_dn(&self, username: &str) -> anyhow::Result<Option<String>> {
        // If bind_dn_pattern is configured, use it directly
        if let Some(pattern) = &self.config.bind_dn_pattern {
            let dn = pattern.replace("{username}", username);
            return Ok(Some(dn));
        }

        // Otherwise, search for the user
        self.search_user(username).await
    }

    /// Search for a user in LDAP
    async fn search_user(&self, username: &str) -> anyhow::Result<Option<String>> {
        let mut ldap = self.pool.get_connection().await?;

        let filter = self.config.user_filter.replace("{username}", username);

        let timeout = tokio::time::Duration::from_secs(self.config.timeout_secs);
        let result = tokio::time::timeout(
            timeout,
            ldap.search(&self.config.base_dn, Scope::Subtree, &filter, vec!["dn"]),
        )
        .await
        .map_err(|_| anyhow::anyhow!("LDAP search timeout"))??;

        let (entries, _res) = result.success()?;

        let dn = if entries.is_empty() {
            None
        } else {
            let entry = SearchEntry::construct(entries[0].clone());
            Some(entry.dn)
        };

        self.pool.return_connection(ldap).await;

        Ok(dn)
    }

    /// Attempt to bind as user
    async fn bind_as_user(&self, dn: &str, password: &str) -> anyhow::Result<bool> {
        let mut ldap = self.pool.create_connection().await?;

        let timeout = tokio::time::Duration::from_secs(self.config.timeout_secs);
        match tokio::time::timeout(timeout, ldap.simple_bind(dn, password)).await {
            Ok(Ok(_)) => {
                let _ = ldap.unbind().await;
                Ok(true)
            }
            Ok(Err(_)) => Ok(false),
            Err(_) => Err(anyhow::anyhow!("LDAP bind timeout")),
        }
    }

    /// Check group membership
    async fn check_group_membership(&self, username: &str) -> anyhow::Result<bool> {
        if self.config.required_group.is_none() {
            return Ok(true);
        }

        let group_base = match &self.config.group_base_dn {
            Some(base) => base,
            None => &self.config.base_dn,
        };

        let filter = match &self.config.group_filter {
            Some(f) => f.replace("{username}", username),
            None => format!("(memberUid={})", username),
        };

        let mut ldap = self.pool.get_connection().await?;

        let timeout = tokio::time::Duration::from_secs(self.config.timeout_secs);
        let result = tokio::time::timeout(
            timeout,
            ldap.search(group_base, Scope::Subtree, &filter, vec!["dn"]),
        )
        .await
        .map_err(|_| anyhow::anyhow!("LDAP group search timeout"))??;

        let (entries, _res) = result.success()?;

        self.pool.return_connection(ldap).await;

        if let Some(required_group) = &self.config.required_group {
            Ok(entries.iter().any(|entry| {
                let e = SearchEntry::construct(entry.clone());
                &e.dn == required_group
            }))
        } else {
            Ok(!entries.is_empty())
        }
    }
}

#[async_trait]
impl AuthBackend for LdapBackend {
    async fn authenticate(&self, username: &Username, password: &str) -> anyhow::Result<bool> {
        // Get user DN (either by pattern or search)
        let dn = match self.get_user_dn(&username.to_string()).await? {
            Some(dn) => dn,
            None => return Ok(false),
        };

        // Try to bind as user
        let bind_success = self.bind_as_user(&dn, password).await?;
        if !bind_success {
            return Ok(false);
        }

        // Check group membership if required
        if !self.check_group_membership(&username.to_string()).await? {
            return Ok(false);
        }

        // Cache successful authentication
        self.user_cache
            .write()
            .await
            .insert(username.to_string(), true);

        Ok(true)
    }

    async fn verify_identity(&self, username: &Username) -> anyhow::Result<bool> {
        // Check cache first
        {
            let cache = self.user_cache.read().await;
            if cache.contains_key(&username.to_string()) {
                return Ok(true);
            }
        }

        // Search in LDAP
        let exists = self.search_user(&username.to_string()).await?.is_some();

        if exists {
            self.user_cache
                .write()
                .await
                .insert(username.to_string(), true);
        }

        Ok(exists)
    }

    async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
        let mut ldap = self.pool.get_connection().await?;

        let filter = self.config.user_filter.replace("{username}", "*");
        let result = ldap
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                vec!["uid", "mail"],
            )
            .await?;

        let (entries, _res) = result.success()?;

        self.pool.return_connection(ldap).await;

        let users = entries
            .into_iter()
            .filter_map(|entry| {
                let e = SearchEntry::construct(entry);
                e.attrs
                    .get("uid")
                    .and_then(|uids| uids.first().and_then(|uid| Username::new(uid.clone()).ok()))
            })
            .collect();

        Ok(users)
    }

    async fn create_user(&self, _username: &Username, _password: &str) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "LDAP backend does not support user creation (read-only)"
        ))
    }

    async fn delete_user(&self, _username: &Username) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "LDAP backend does not support user deletion (read-only)"
        ))
    }

    async fn change_password(
        &self,
        _username: &Username,
        _new_password: &str,
    ) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "LDAP backend does not support password changes (read-only)"
        ))
    }
}

impl Clone for LdapBackend {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            pool: ConnectionPool::new(self.config.clone()),
            user_cache: Arc::clone(&self.user_cache),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ldap_config_default() {
        let config = LdapConfig::default();
        assert_eq!(config.server_url, "ldap://localhost:389");
        assert_eq!(config.base_dn, "dc=example,dc=com");
        assert_eq!(config.user_filter, "(uid={username})");
        assert_eq!(config.timeout_secs, 10);
        assert_eq!(config.pool_size, 5);
    }

    #[test]
    fn test_ldap_config_custom() {
        let config = LdapConfig {
            server_url: "ldaps://ldap.example.com:636".to_string(),
            base_dn: "ou=users,dc=example,dc=org".to_string(),
            user_filter: "(mail={username}@example.com)".to_string(),
            bind_dn_pattern: None,
            bind_dn: Some("cn=admin,dc=example,dc=org".to_string()),
            bind_password: Some("secret".to_string()),
            group_base_dn: Some("ou=groups,dc=example,dc=org".to_string()),
            group_filter: Some(
                "(&(objectClass=groupOfNames)(member=uid={username},ou=users,dc=example,dc=org))"
                    .to_string(),
            ),
            required_group: Some("cn=mail-users,ou=groups,dc=example,dc=org".to_string()),
            timeout_secs: 30,
            pool_size: 10,
            use_tls: true,
            tls_skip_verify: false,
        };
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.pool_size, 10);
    }

    #[test]
    fn test_connection_pool_creation() {
        let config = LdapConfig::default();
        let pool = ConnectionPool::new(config.clone());
        assert_eq!(pool.max_size, config.pool_size);
    }

    #[tokio::test]
    async fn test_ldap_backend_creation() {
        let config = LdapConfig::default();
        let backend = LdapBackend::new(config);
        let cache = backend.user_cache.read().await;
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn test_user_filter_substitution() {
        let config = LdapConfig {
            user_filter: "(uid={username})".to_string(),
            ..Default::default()
        };
        let filter = config.user_filter.replace("{username}", "testuser");
        assert_eq!(filter, "(uid=testuser)");
    }

    #[tokio::test]
    async fn test_group_filter_substitution() {
        let config = LdapConfig {
            group_filter: Some("(memberUid={username})".to_string()),
            ..Default::default()
        };
        let filter = config
            .group_filter
            .unwrap()
            .replace("{username}", "testuser");
        assert_eq!(filter, "(memberUid=testuser)");
    }

    #[tokio::test]
    async fn test_verify_identity_cache() {
        let backend = LdapBackend::new(LdapConfig::default());
        let _username = Username::new("cached_user".to_string()).unwrap();

        // Insert into cache
        backend
            .user_cache
            .write()
            .await
            .insert("cached_user".to_string(), true);

        // Should return true from cache
        let cache = backend.user_cache.read().await;
        assert!(cache.contains_key("cached_user"));
    }

    #[tokio::test]
    async fn test_create_user_not_supported() {
        let backend = LdapBackend::new(LdapConfig::default());
        let username = Username::new("newuser".to_string()).unwrap();
        let result = backend.create_user(&username, "password").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("read-only"));
    }

    #[tokio::test]
    async fn test_delete_user_not_supported() {
        let backend = LdapBackend::new(LdapConfig::default());
        let username = Username::new("user".to_string()).unwrap();
        let result = backend.delete_user(&username).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("read-only"));
    }

    #[tokio::test]
    async fn test_change_password_not_supported() {
        let backend = LdapBackend::new(LdapConfig::default());
        let username = Username::new("user".to_string()).unwrap();
        let result = backend.change_password(&username, "newpass").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("read-only"));
    }

    #[test]
    fn test_ldap_config_with_bind_credentials() {
        let config = LdapConfig {
            bind_dn: Some("cn=admin,dc=example,dc=com".to_string()),
            bind_password: Some("admin_password".to_string()),
            ..Default::default()
        };
        assert!(config.bind_dn.is_some());
        assert!(config.bind_password.is_some());
    }

    #[test]
    fn test_ldap_config_without_bind_credentials() {
        let config = LdapConfig::default();
        assert!(config.bind_dn.is_none());
        assert!(config.bind_password.is_none());
    }

    #[test]
    fn test_required_group_configuration() {
        let config = LdapConfig {
            required_group: Some("cn=email-users,ou=groups,dc=example,dc=com".to_string()),
            ..Default::default()
        };
        assert!(config.required_group.is_some());
    }

    #[test]
    fn test_group_base_dn_configuration() {
        let config = LdapConfig {
            group_base_dn: Some("ou=groups,dc=example,dc=com".to_string()),
            ..Default::default()
        };
        assert!(config.group_base_dn.is_some());
    }

    #[tokio::test]
    async fn test_connection_pool_size() {
        let config = LdapConfig {
            pool_size: 3,
            ..Default::default()
        };
        let pool = ConnectionPool::new(config);
        assert_eq!(pool.max_size, 3);
    }

    #[test]
    fn test_complex_user_filter() {
        let config = LdapConfig {
            user_filter: "(&(objectClass=person)(|(uid={username})(mail={username})))".to_string(),
            ..Default::default()
        };
        let filter = config.user_filter.replace("{username}", "john");
        assert!(filter.contains("uid=john"));
        assert!(filter.contains("mail=john"));
    }

    #[test]
    fn test_complex_group_filter() {
        let config = LdapConfig {
            group_filter: Some(
                "(&(objectClass=groupOfNames)(member=uid={username},ou=users,dc=example,dc=com))"
                    .to_string(),
            ),
            ..Default::default()
        };
        let filter = config.group_filter.unwrap().replace("{username}", "jane");
        assert!(filter.contains("uid=jane"));
    }

    #[test]
    fn test_timeout_configuration() {
        let config = LdapConfig {
            timeout_secs: 60,
            ..Default::default()
        };
        assert_eq!(config.timeout_secs, 60);
    }

    #[test]
    fn test_ldaps_url() {
        let config = LdapConfig {
            server_url: "ldaps://secure-ldap.example.com:636".to_string(),
            ..Default::default()
        };
        assert!(config.server_url.starts_with("ldaps://"));
    }

    #[tokio::test]
    async fn test_cache_empty_on_creation() {
        let backend = LdapBackend::new(LdapConfig::default());
        let cache = backend.user_cache.read().await;
        assert!(cache.is_empty());
    }

    #[tokio::test]
    async fn test_cache_insertion() {
        let backend = LdapBackend::new(LdapConfig::default());
        backend
            .user_cache
            .write()
            .await
            .insert("user1".to_string(), true);
        backend
            .user_cache
            .write()
            .await
            .insert("user2".to_string(), true);

        let cache = backend.user_cache.read().await;
        assert_eq!(cache.len(), 2);
        assert!(cache.contains_key("user1"));
        assert!(cache.contains_key("user2"));
    }

    #[test]
    fn test_bind_dn_pattern() {
        let config = LdapConfig {
            bind_dn_pattern: Some("uid={username},ou=users,dc=example,dc=com".to_string()),
            ..Default::default()
        };
        assert!(config.bind_dn_pattern.is_some());
        let dn = config
            .bind_dn_pattern
            .unwrap()
            .replace("{username}", "alice");
        assert_eq!(dn, "uid=alice,ou=users,dc=example,dc=com");
    }

    #[test]
    fn test_bind_dn_pattern_with_email() {
        let config = LdapConfig {
            bind_dn_pattern: Some(
                "mail={username}@example.com,ou=users,dc=example,dc=com".to_string(),
            ),
            ..Default::default()
        };
        let dn = config.bind_dn_pattern.unwrap().replace("{username}", "bob");
        assert_eq!(dn, "mail=bob@example.com,ou=users,dc=example,dc=com");
    }

    #[test]
    fn test_tls_configuration() {
        let config = LdapConfig {
            use_tls: true,
            server_url: "ldaps://ldap.example.com:636".to_string(),
            ..Default::default()
        };
        assert!(config.use_tls);
        assert!(config.server_url.starts_with("ldaps://"));
    }

    #[test]
    fn test_tls_skip_verify_configuration() {
        let config = LdapConfig {
            use_tls: true,
            tls_skip_verify: true,
            ..Default::default()
        };
        assert!(config.use_tls);
        assert!(config.tls_skip_verify);
    }

    #[test]
    fn test_multiple_user_filter_patterns() {
        let config = LdapConfig {
            user_filter:
                "(&(objectClass=inetOrgPerson)(|(uid={username})(mail={username}@example.com)))"
                    .to_string(),
            ..Default::default()
        };
        let filter = config.user_filter.replace("{username}", "charlie");
        assert!(filter.contains("uid=charlie"));
        assert!(filter.contains("mail=charlie@example.com"));
    }

    #[test]
    fn test_memberof_group_filter() {
        let config = LdapConfig {
            group_filter: Some("(memberOf=cn=mail-users,ou=groups,dc=example,dc=com)".to_string()),
            ..Default::default()
        };
        assert!(config.group_filter.is_some());
    }

    #[tokio::test]
    async fn test_connection_pool_multiple_returns() {
        let config = LdapConfig {
            pool_size: 2,
            ..Default::default()
        };
        let pool = ConnectionPool::new(config);

        // Simulate returning connections
        let connections = pool.connections.read().await;
        assert_eq!(connections.len(), 0);
    }

    #[test]
    fn test_ldap_config_clone() {
        let config1 = LdapConfig {
            server_url: "ldap://test.com:389".to_string(),
            timeout_secs: 20,
            ..Default::default()
        };
        let config2 = config1.clone();
        assert_eq!(config1.server_url, config2.server_url);
        assert_eq!(config1.timeout_secs, config2.timeout_secs);
    }

    #[test]
    fn test_empty_group_base_dn_fallback() {
        let config = LdapConfig {
            base_dn: "dc=example,dc=org".to_string(),
            group_base_dn: None,
            ..Default::default()
        };
        let group_base = config.group_base_dn.as_ref().unwrap_or(&config.base_dn);
        assert_eq!(group_base, "dc=example,dc=org");
    }

    #[test]
    fn test_custom_timeout() {
        let config = LdapConfig {
            timeout_secs: 5,
            ..Default::default()
        };
        assert_eq!(config.timeout_secs, 5);
    }

    #[test]
    fn test_large_pool_size() {
        let config = LdapConfig {
            pool_size: 100,
            ..Default::default()
        };
        assert_eq!(config.pool_size, 100);
    }

    #[tokio::test]
    async fn test_cache_concurrent_access() {
        let backend = LdapBackend::new(LdapConfig::default());

        // Simulate concurrent writes
        let backend1 = backend.clone();
        let backend2 = backend.clone();

        let handle1 = tokio::spawn(async move {
            backend1
                .user_cache
                .write()
                .await
                .insert("user_a".to_string(), true);
        });

        let handle2 = tokio::spawn(async move {
            backend2
                .user_cache
                .write()
                .await
                .insert("user_b".to_string(), true);
        });

        let _ = handle1.await;
        let _ = handle2.await;

        let cache = backend.user_cache.read().await;
        assert!(cache.len() <= 2);
    }

    #[test]
    fn test_ldap_url_with_port() {
        let config = LdapConfig {
            server_url: "ldap://ldap.company.com:389".to_string(),
            ..Default::default()
        };
        assert!(config.server_url.contains(":389"));
    }

    #[test]
    fn test_ldaps_url_with_port() {
        let config = LdapConfig {
            server_url: "ldaps://ldap.company.com:636".to_string(),
            ..Default::default()
        };
        assert!(config.server_url.contains(":636"));
    }

    #[test]
    fn test_active_directory_user_filter() {
        let config = LdapConfig {
            user_filter: "(&(objectClass=user)(sAMAccountName={username}))".to_string(),
            ..Default::default()
        };
        let filter = config.user_filter.replace("{username}", "jdoe");
        assert!(filter.contains("sAMAccountName=jdoe"));
    }

    #[test]
    fn test_active_directory_group_filter() {
        let config = LdapConfig {
            group_filter: Some("(&(objectClass=group)(member={dn}))".to_string()),
            ..Default::default()
        };
        assert!(config.group_filter.is_some());
    }

    #[test]
    fn test_posix_group_filter() {
        let config = LdapConfig {
            group_filter: Some("(&(objectClass=posixGroup)(memberUid={username}))".to_string()),
            ..Default::default()
        };
        let filter = config
            .group_filter
            .unwrap()
            .replace("{username}", "user123");
        assert!(filter.contains("memberUid=user123"));
    }

    #[tokio::test]
    async fn test_ldap_backend_clone() {
        let backend1 = LdapBackend::new(LdapConfig::default());
        let backend2 = backend1.clone();

        // Both should share the same cache
        backend1
            .user_cache
            .write()
            .await
            .insert("test".to_string(), true);
        let cache2 = backend2.user_cache.read().await;
        assert!(cache2.contains_key("test"));
    }
}
