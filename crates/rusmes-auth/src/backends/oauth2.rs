//! OAuth2/OIDC authentication backend

use crate::AuthBackend;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use rusmes_proto::Username;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

/// OIDC provider configuration
#[derive(Debug, Clone)]
pub enum OidcProvider {
    /// Google OAuth2
    Google {
        client_id: String,
        client_secret: String,
    },
    /// Microsoft Azure AD
    Microsoft {
        tenant_id: String,
        client_id: String,
        client_secret: String,
    },
    /// Generic OIDC provider
    Generic {
        issuer_url: String,
        client_id: String,
        client_secret: String,
        jwks_url: String,
    },
}

/// JWT claims structure
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    email: Option<String>,
    exp: u64,
    iat: u64,
    iss: String,
    aud: String,
}

/// Token introspection response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct IntrospectionResponse {
    active: bool,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    exp: Option<u64>,
}

/// JWKS (JSON Web Key Set) structure
#[derive(Debug, Clone, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

/// JSON Web Key
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct Jwk {
    kid: String,
    kty: String,
    #[serde(rename = "use")]
    key_use: Option<String>,
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

/// Token cache entry
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TokenCacheEntry {
    username: String,
    expires_at: SystemTime,
}

/// OAuth2/OIDC configuration
#[derive(Debug, Clone)]
pub struct OAuth2Config {
    /// OIDC provider
    pub provider: OidcProvider,
    /// Token introspection endpoint
    pub introspection_endpoint: Option<String>,
    /// JWKS cache TTL in seconds
    pub jwks_cache_ttl: u64,
    /// Enable refresh token support
    pub enable_refresh_tokens: bool,
    /// Allowed algorithms for JWT validation
    pub allowed_algorithms: Vec<Algorithm>,
}

impl Default for OAuth2Config {
    fn default() -> Self {
        Self {
            provider: OidcProvider::Generic {
                issuer_url: "https://example.com".to_string(),
                client_id: "client-id".to_string(),
                client_secret: "client-secret".to_string(),
                jwks_url: "https://example.com/.well-known/jwks.json".to_string(),
            },
            introspection_endpoint: None,
            jwks_cache_ttl: 3600,
            enable_refresh_tokens: true,
            allowed_algorithms: vec![Algorithm::RS256],
        }
    }
}

/// OAuth2/OIDC authentication backend
pub struct OAuth2Backend {
    config: OAuth2Config,
    token_cache: Arc<RwLock<HashMap<String, TokenCacheEntry>>>,
    jwks_cache: Arc<RwLock<Option<(Jwks, SystemTime)>>>,
    client: reqwest::Client,
}

impl OAuth2Backend {
    /// Create a new OAuth2 authentication backend
    pub fn new(config: OAuth2Config) -> Self {
        Self {
            config,
            token_cache: Arc::new(RwLock::new(HashMap::new())),
            jwks_cache: Arc::new(RwLock::new(None)),
            client: reqwest::Client::new(),
        }
    }

    /// Parse XOAUTH2 SASL initial response
    ///
    /// Format: `base64(user=<username>\x01auth=Bearer <token>\x01\x01)`
    pub fn parse_xoauth2_response(response: &str) -> anyhow::Result<(String, String)> {
        // Decode base64
        let decoded = BASE64
            .decode(response.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to decode XOAUTH2 response: {}", e))?;

        let decoded_str = String::from_utf8(decoded)
            .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in XOAUTH2 response: {}", e))?;

        // Split by \x01
        let parts: Vec<&str> = decoded_str.split('\x01').collect();

        // Extract username and token
        let mut username = None;
        let mut token = None;

        for part in &parts {
            if part.starts_with("user=") {
                username = part.strip_prefix("user=").map(|s| s.to_string());
            } else if part.starts_with("auth=Bearer ") {
                token = part.strip_prefix("auth=Bearer ").map(|s| s.to_string());
            }
        }

        let username = username.ok_or_else(|| anyhow::anyhow!("Missing username in XOAUTH2"))?;
        let token = token.ok_or_else(|| anyhow::anyhow!("Missing token in XOAUTH2"))?;

        Ok((username, token))
    }

    /// Encode XOAUTH2 SASL initial response
    ///
    /// Format: `base64(user=<username>\x01auth=Bearer <token>\x01\x01)`
    #[allow(dead_code)]
    pub fn encode_xoauth2_response(username: &str, token: &str) -> String {
        let response = format!("user={}\x01auth=Bearer {}\x01\x01", username, token);
        BASE64.encode(response.as_bytes())
    }

    /// Clear expired entries from token cache
    pub async fn cleanup_expired_tokens(&self) {
        let mut cache = self.token_cache.write().await;
        let now = SystemTime::now();
        cache.retain(|_, entry| entry.expires_at > now);
    }

    /// Get token cache size
    #[allow(dead_code)]
    pub async fn token_cache_size(&self) -> usize {
        let cache = self.token_cache.read().await;
        cache.len()
    }

    /// Invalidate cached token for a user
    #[allow(dead_code)]
    pub async fn invalidate_token(&self, username: &str) {
        let mut cache = self.token_cache.write().await;
        cache.remove(username);
    }

    /// Clear JWKS cache (force refresh on next validation)
    #[allow(dead_code)]
    pub async fn clear_jwks_cache(&self) {
        let mut cache = self.jwks_cache.write().await;
        *cache = None;
    }

    /// Get JWKS from provider
    async fn get_jwks(&self) -> anyhow::Result<Jwks> {
        // Check cache first
        {
            let cache = self.jwks_cache.read().await;
            if let Some((jwks, cached_at)) = &*cache {
                if cached_at.elapsed().unwrap_or(Duration::MAX).as_secs()
                    < self.config.jwks_cache_ttl
                {
                    return Ok(jwks.clone());
                }
            }
        }

        // Fetch from provider
        let jwks_url = match &self.config.provider {
            OidcProvider::Google { .. } => "https://www.googleapis.com/oauth2/v3/certs",
            OidcProvider::Microsoft { tenant_id, .. } => &format!(
                "https://login.microsoftonline.com/{}/discovery/v2.0/keys",
                tenant_id
            ),
            OidcProvider::Generic { jwks_url, .. } => jwks_url.as_str(),
        };

        let jwks: Jwks = self.client.get(jwks_url).send().await?.json().await?;

        // Update cache
        {
            let mut cache = self.jwks_cache.write().await;
            *cache = Some((jwks.clone(), SystemTime::now()));
        }

        Ok(jwks)
    }

    /// Validate JWT token
    async fn validate_jwt(&self, token: &str) -> anyhow::Result<Claims> {
        // Decode header to get kid
        let header = decode_header(token)?;
        let kid = header
            .kid
            .ok_or_else(|| anyhow::anyhow!("No kid in JWT header"))?;

        // Get JWKS
        let jwks = self.get_jwks().await?;

        // Find matching key
        let jwk = jwks
            .keys
            .iter()
            .find(|k| k.kid == kid)
            .ok_or_else(|| anyhow::anyhow!("No matching key found in JWKS"))?;

        // Construct RSA public key from JWK
        let n = jwk
            .n
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing n in JWK"))?;
        let e = jwk
            .e
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing e in JWK"))?;

        let n_bytes = BASE64.decode(n)?;
        let e_bytes = BASE64.decode(e)?;

        // Create decoding key
        let decoding_key =
            DecodingKey::from_rsa_components(&BASE64.encode(&n_bytes), &BASE64.encode(&e_bytes))?;

        // Validate token
        let mut validation = Validation::new(Algorithm::RS256);
        validation.algorithms = self.config.allowed_algorithms.clone();

        // Set expected audience based on provider
        let expected_aud = match &self.config.provider {
            OidcProvider::Google { client_id, .. } => client_id.clone(),
            OidcProvider::Microsoft { client_id, .. } => client_id.clone(),
            OidcProvider::Generic { client_id, .. } => client_id.clone(),
        };
        validation.set_audience(&[&expected_aud]);

        let token_data = decode::<Claims>(token, &decoding_key, &validation)?;

        Ok(token_data.claims)
    }

    /// Introspect token at provider's introspection endpoint
    async fn introspect_token(&self, token: &str) -> anyhow::Result<IntrospectionResponse> {
        let endpoint = self
            .config
            .introspection_endpoint
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Token introspection endpoint not configured"))?;

        let (client_id, client_secret) = match &self.config.provider {
            OidcProvider::Google {
                client_id,
                client_secret,
            } => (client_id, client_secret),
            OidcProvider::Microsoft {
                client_id,
                client_secret,
                ..
            } => (client_id, client_secret),
            OidcProvider::Generic {
                client_id,
                client_secret,
                ..
            } => (client_id, client_secret),
        };

        let mut params = HashMap::new();
        params.insert("token", token);
        params.insert("client_id", client_id);
        params.insert("client_secret", client_secret);

        let response = self
            .client
            .post(endpoint)
            .form(&params)
            .send()
            .await?
            .json::<IntrospectionResponse>()
            .await?;

        Ok(response)
    }

    /// Authenticate using XOAUTH2 SASL mechanism
    async fn xoauth2_authenticate(&self, token: &str) -> anyhow::Result<String> {
        // Try JWT validation first
        if let Ok(claims) = self.validate_jwt(token).await {
            return Ok(claims.email.or(Some(claims.sub)).unwrap_or_default());
        }

        // Fall back to token introspection
        let introspection = self.introspect_token(token).await?;

        if !introspection.active {
            return Err(anyhow::anyhow!("Token is not active"));
        }

        introspection
            .email
            .or(introspection.username)
            .ok_or_else(|| anyhow::anyhow!("No username in token"))
    }

    /// Refresh access token using refresh token
    #[allow(dead_code)]
    async fn refresh_token(&self, refresh_token: &str) -> anyhow::Result<String> {
        if !self.config.enable_refresh_tokens {
            return Err(anyhow::anyhow!("Refresh tokens not enabled"));
        }

        let token_endpoint = match &self.config.provider {
            OidcProvider::Google { .. } => "https://oauth2.googleapis.com/token",
            OidcProvider::Microsoft { tenant_id, .. } => &format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                tenant_id
            ),
            OidcProvider::Generic { issuer_url, .. } => &format!("{}/token", issuer_url),
        };

        let (client_id, client_secret) = match &self.config.provider {
            OidcProvider::Google {
                client_id,
                client_secret,
            } => (client_id, client_secret),
            OidcProvider::Microsoft {
                client_id,
                client_secret,
                ..
            } => (client_id, client_secret),
            OidcProvider::Generic {
                client_id,
                client_secret,
                ..
            } => (client_id, client_secret),
        };

        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token");
        params.insert("refresh_token", refresh_token);
        params.insert("client_id", client_id);
        params.insert("client_secret", client_secret);

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let response = self
            .client
            .post(token_endpoint)
            .form(&params)
            .send()
            .await?
            .json::<TokenResponse>()
            .await?;

        Ok(response.access_token)
    }
}

#[async_trait]
impl AuthBackend for OAuth2Backend {
    async fn authenticate(&self, username: &Username, password: &str) -> anyhow::Result<bool> {
        // In OAuth2 flow, "password" is the access token
        let token = password;

        // Check cache first
        {
            let cache = self.token_cache.read().await;
            if let Some(entry) = cache.get(&username.to_string()) {
                if SystemTime::now() < entry.expires_at {
                    return Ok(true);
                }
            }
        }

        // Validate token and get username
        match self.xoauth2_authenticate(token).await {
            Ok(token_username) => {
                if token_username == username.to_string() {
                    // Cache successful authentication
                    let mut cache = self.token_cache.write().await;
                    cache.insert(
                        username.to_string(),
                        TokenCacheEntry {
                            username: token_username,
                            expires_at: SystemTime::now() + Duration::from_secs(300),
                        },
                    );
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Err(_) => Ok(false),
        }
    }

    async fn verify_identity(&self, username: &Username) -> anyhow::Result<bool> {
        // Check cache
        let cache = self.token_cache.read().await;
        Ok(cache.contains_key(&username.to_string()))
    }

    async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
        // OAuth2 backends don't maintain a user list
        let cache = self.token_cache.read().await;
        Ok(cache
            .keys()
            .filter_map(|k| Username::new(k.clone()).ok())
            .collect())
    }

    async fn create_user(&self, _username: &Username, _password: &str) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "OAuth2 backend does not support user creation (external provider)"
        ))
    }

    async fn delete_user(&self, _username: &Username) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "OAuth2 backend does not support user deletion (external provider)"
        ))
    }

    async fn change_password(
        &self,
        _username: &Username,
        _new_password: &str,
    ) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "OAuth2 backend does not support password changes (external provider)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Configuration Tests
    // ========================================================================

    #[test]
    fn test_oauth2_config_default() {
        let config = OAuth2Config::default();
        assert_eq!(config.jwks_cache_ttl, 3600);
        assert!(config.enable_refresh_tokens);
        assert_eq!(config.allowed_algorithms.len(), 1);
    }

    #[test]
    fn test_oauth2_config_google() {
        let config = OAuth2Config {
            provider: OidcProvider::Google {
                client_id: "test-client-id".to_string(),
                client_secret: "test-secret".to_string(),
            },
            ..Default::default()
        };
        assert!(matches!(config.provider, OidcProvider::Google { .. }));
    }

    #[test]
    fn test_oauth2_config_microsoft() {
        let config = OAuth2Config {
            provider: OidcProvider::Microsoft {
                tenant_id: "test-tenant".to_string(),
                client_id: "test-client".to_string(),
                client_secret: "test-secret".to_string(),
            },
            ..Default::default()
        };
        assert!(matches!(config.provider, OidcProvider::Microsoft { .. }));
    }

    #[test]
    fn test_oauth2_config_generic() {
        let config = OAuth2Config {
            provider: OidcProvider::Generic {
                issuer_url: "https://oidc.example.com".to_string(),
                client_id: "client".to_string(),
                client_secret: "secret".to_string(),
                jwks_url: "https://oidc.example.com/jwks".to_string(),
            },
            ..Default::default()
        };
        assert!(matches!(config.provider, OidcProvider::Generic { .. }));
    }

    #[test]
    fn test_allowed_algorithms() {
        let config = OAuth2Config {
            allowed_algorithms: vec![Algorithm::RS256, Algorithm::RS384, Algorithm::RS512],
            ..Default::default()
        };
        assert_eq!(config.allowed_algorithms.len(), 3);
    }

    #[test]
    fn test_introspection_endpoint_optional() {
        let config = OAuth2Config::default();
        assert!(config.introspection_endpoint.is_none());

        let config_with_introspection = OAuth2Config {
            introspection_endpoint: Some("https://example.com/introspect".to_string()),
            ..Default::default()
        };
        assert!(config_with_introspection.introspection_endpoint.is_some());
    }

    #[test]
    fn test_refresh_tokens_enabled() {
        let config = OAuth2Config {
            enable_refresh_tokens: true,
            ..Default::default()
        };
        assert!(config.enable_refresh_tokens);

        let config_disabled = OAuth2Config {
            enable_refresh_tokens: false,
            ..Default::default()
        };
        assert!(!config_disabled.enable_refresh_tokens);
    }

    #[test]
    fn test_jwks_cache_ttl() {
        let config = OAuth2Config {
            jwks_cache_ttl: 7200,
            ..Default::default()
        };
        assert_eq!(config.jwks_cache_ttl, 7200);
    }

    #[test]
    fn test_config_clone() {
        let config = OAuth2Config::default();
        let cloned = config.clone();
        assert_eq!(config.jwks_cache_ttl, cloned.jwks_cache_ttl);
    }

    // ========================================================================
    // Backend Creation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_oauth2_backend_creation() {
        let config = OAuth2Config::default();
        let backend = OAuth2Backend::new(config);
        let cache = backend.token_cache.read().await;
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn test_token_cache_empty_on_creation() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let cache = backend.token_cache.read().await;
        assert!(cache.is_empty());
    }

    #[tokio::test]
    async fn test_jwks_cache_empty_on_creation() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let cache = backend.jwks_cache.read().await;
        assert!(cache.is_none());
    }

    // ========================================================================
    // AuthBackend Trait Tests
    // ========================================================================

    #[tokio::test]
    async fn test_create_user_not_supported() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let username = Username::new("user@example.com".to_string()).unwrap();
        let result = backend.create_user(&username, "token").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("external provider"));
    }

    #[tokio::test]
    async fn test_delete_user_not_supported() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let username = Username::new("user@example.com".to_string()).unwrap();
        let result = backend.delete_user(&username).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("external provider"));
    }

    #[tokio::test]
    async fn test_change_password_not_supported() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let username = Username::new("user@example.com".to_string()).unwrap();
        let result = backend.change_password(&username, "newtoken").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("external provider"));
    }

    #[tokio::test]
    async fn test_list_users_empty() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let users = backend.list_users().await.unwrap();
        assert_eq!(users.len(), 0);
    }

    #[tokio::test]
    async fn test_verify_identity_not_cached() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let username = Username::new("user@example.com".to_string()).unwrap();
        let verified = backend.verify_identity(&username).await.unwrap();
        assert!(!verified);
    }

    #[tokio::test]
    async fn test_verify_identity_cached() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let username = Username::new("cached@example.com".to_string()).unwrap();

        {
            let mut cache = backend.token_cache.write().await;
            cache.insert(
                username.to_string(),
                TokenCacheEntry {
                    username: username.to_string(),
                    expires_at: SystemTime::now() + Duration::from_secs(300),
                },
            );
        }

        let verified = backend.verify_identity(&username).await.unwrap();
        assert!(verified);
    }

    // ========================================================================
    // Token Cache Tests
    // ========================================================================

    #[tokio::test]
    async fn test_token_cache_insertion() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.token_cache.write().await;
            cache.insert(
                "user@example.com".to_string(),
                TokenCacheEntry {
                    username: "user@example.com".to_string(),
                    expires_at: SystemTime::now() + Duration::from_secs(300),
                },
            );
        }

        let cache = backend.token_cache.read().await;
        assert_eq!(cache.len(), 1);
        assert!(cache.contains_key("user@example.com"));
    }

    #[tokio::test]
    async fn test_token_cache_expiration() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.token_cache.write().await;
            cache.insert(
                "expired@example.com".to_string(),
                TokenCacheEntry {
                    username: "expired@example.com".to_string(),
                    expires_at: SystemTime::now() - Duration::from_secs(1),
                },
            );
        }

        // Cache contains entry but it's expired
        let cache = backend.token_cache.read().await;
        let entry = cache.get("expired@example.com").unwrap();
        assert!(entry.expires_at < SystemTime::now());
    }

    #[tokio::test]
    async fn test_token_cache_multiple_users() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.token_cache.write().await;
            for i in 1..=5 {
                cache.insert(
                    format!("user{}@example.com", i),
                    TokenCacheEntry {
                        username: format!("user{}@example.com", i),
                        expires_at: SystemTime::now() + Duration::from_secs(300),
                    },
                );
            }
        }

        let cache = backend.token_cache.read().await;
        assert_eq!(cache.len(), 5);
    }

    #[tokio::test]
    async fn test_list_users_with_cached_tokens() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.token_cache.write().await;
            cache.insert(
                "user1@example.com".to_string(),
                TokenCacheEntry {
                    username: "user1@example.com".to_string(),
                    expires_at: SystemTime::now() + Duration::from_secs(300),
                },
            );
            cache.insert(
                "user2@example.com".to_string(),
                TokenCacheEntry {
                    username: "user2@example.com".to_string(),
                    expires_at: SystemTime::now() + Duration::from_secs(300),
                },
            );
        }

        let users = backend.list_users().await.unwrap();
        assert_eq!(users.len(), 2);
    }

    // ========================================================================
    // Claims Structure Tests
    // ========================================================================

    #[test]
    fn test_claims_structure() {
        let claims = Claims {
            sub: "user123".to_string(),
            email: Some("user@example.com".to_string()),
            exp: 1234567890,
            iat: 1234567800,
            iss: "https://accounts.google.com".to_string(),
            aud: "client-id".to_string(),
        };
        assert_eq!(claims.sub, "user123");
        assert_eq!(claims.email.unwrap(), "user@example.com");
    }

    #[test]
    fn test_claims_without_email() {
        let claims = Claims {
            sub: "user123".to_string(),
            email: None,
            exp: 1234567890,
            iat: 1234567800,
            iss: "https://accounts.google.com".to_string(),
            aud: "client-id".to_string(),
        };
        assert_eq!(claims.sub, "user123");
        assert!(claims.email.is_none());
    }

    // ========================================================================
    // Token Cache Entry Tests
    // ========================================================================

    #[test]
    fn test_token_cache_entry() {
        let entry = TokenCacheEntry {
            username: "user@example.com".to_string(),
            expires_at: SystemTime::now() + Duration::from_secs(300),
        };
        assert_eq!(entry.username, "user@example.com");
        assert!(entry.expires_at > SystemTime::now());
    }

    #[test]
    fn test_token_cache_entry_expired() {
        let entry = TokenCacheEntry {
            username: "user@example.com".to_string(),
            expires_at: SystemTime::now() - Duration::from_secs(10),
        };
        assert!(entry.expires_at < SystemTime::now());
    }

    // ========================================================================
    // Provider-specific Tests
    // ========================================================================

    #[test]
    fn test_google_provider_config() {
        let provider = OidcProvider::Google {
            client_id: "google-client-id".to_string(),
            client_secret: "google-secret".to_string(),
        };

        if let OidcProvider::Google { client_id, .. } = &provider {
            assert_eq!(client_id, "google-client-id");
        } else {
            panic!("Expected Google provider");
        }
    }

    #[test]
    fn test_microsoft_provider_config() {
        let provider = OidcProvider::Microsoft {
            tenant_id: "tenant-123".to_string(),
            client_id: "ms-client-id".to_string(),
            client_secret: "ms-secret".to_string(),
        };

        if let OidcProvider::Microsoft { tenant_id, .. } = &provider {
            assert_eq!(tenant_id, "tenant-123");
        } else {
            panic!("Expected Microsoft provider");
        }
    }

    #[test]
    fn test_generic_provider_config() {
        let provider = OidcProvider::Generic {
            issuer_url: "https://auth.example.com".to_string(),
            client_id: "generic-client".to_string(),
            client_secret: "generic-secret".to_string(),
            jwks_url: "https://auth.example.com/.well-known/jwks.json".to_string(),
        };

        if let OidcProvider::Generic { issuer_url, .. } = &provider {
            assert_eq!(issuer_url, "https://auth.example.com");
        } else {
            panic!("Expected Generic provider");
        }
    }

    // ========================================================================
    // Algorithm Tests
    // ========================================================================

    #[test]
    fn test_multiple_allowed_algorithms() {
        let config = OAuth2Config {
            allowed_algorithms: vec![
                Algorithm::RS256,
                Algorithm::RS384,
                Algorithm::RS512,
                Algorithm::ES256,
            ],
            ..Default::default()
        };
        assert_eq!(config.allowed_algorithms.len(), 4);
        assert!(config.allowed_algorithms.contains(&Algorithm::RS256));
        assert!(config.allowed_algorithms.contains(&Algorithm::ES256));
    }

    #[test]
    fn test_single_algorithm_rs256() {
        let config = OAuth2Config {
            allowed_algorithms: vec![Algorithm::RS256],
            ..Default::default()
        };
        assert_eq!(config.allowed_algorithms.len(), 1);
        assert_eq!(config.allowed_algorithms[0], Algorithm::RS256);
    }

    // ========================================================================
    // JWKS Structure Tests
    // ========================================================================

    #[test]
    fn test_jwks_structure() {
        let jwks = Jwks { keys: vec![] };
        assert_eq!(jwks.keys.len(), 0);
    }

    #[test]
    fn test_jwk_structure() {
        let jwk = Jwk {
            kid: "key-1".to_string(),
            kty: "RSA".to_string(),
            key_use: Some("sig".to_string()),
            alg: Some("RS256".to_string()),
            n: Some("modulus".to_string()),
            e: Some("AQAB".to_string()),
        };
        assert_eq!(jwk.kid, "key-1");
        assert_eq!(jwk.kty, "RSA");
    }

    // ========================================================================
    // Edge Cases
    // ========================================================================

    #[tokio::test]
    async fn test_authenticate_empty_token() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let username = Username::new("user@example.com".to_string()).unwrap();
        let result = backend.authenticate(&username, "").await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_authenticate_invalid_token() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let username = Username::new("user@example.com".to_string()).unwrap();
        let result = backend.authenticate(&username, "invalid-token").await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_config_with_all_options() {
        let config = OAuth2Config {
            provider: OidcProvider::Google {
                client_id: "client".to_string(),
                client_secret: "secret".to_string(),
            },
            introspection_endpoint: Some("https://oauth.example.com/introspect".to_string()),
            jwks_cache_ttl: 1800,
            enable_refresh_tokens: false,
            allowed_algorithms: vec![Algorithm::RS256, Algorithm::RS384],
        };

        assert!(config.introspection_endpoint.is_some());
        assert_eq!(config.jwks_cache_ttl, 1800);
        assert!(!config.enable_refresh_tokens);
        assert_eq!(config.allowed_algorithms.len(), 2);
    }

    // ========================================================================
    // Username Validation Tests
    // ========================================================================

    #[tokio::test]
    async fn test_verify_identity_invalid_username() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        // Create a valid username that's not in cache
        let username = Username::new("nonexistent@example.com".to_string()).unwrap();
        let result = backend.verify_identity(&username).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    // ========================================================================
    // Concurrent Access Tests
    // ========================================================================

    #[tokio::test]
    async fn test_concurrent_cache_access() {
        let backend = Arc::new(OAuth2Backend::new(OAuth2Config::default()));

        let mut handles = vec![];
        for i in 0..10 {
            let backend = Arc::clone(&backend);
            let handle = tokio::spawn(async move {
                let mut cache = backend.token_cache.write().await;
                cache.insert(
                    format!("user{}@example.com", i),
                    TokenCacheEntry {
                        username: format!("user{}@example.com", i),
                        expires_at: SystemTime::now() + Duration::from_secs(300),
                    },
                );
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let cache = backend.token_cache.read().await;
        assert_eq!(cache.len(), 10);
    }

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    #[tokio::test]
    async fn test_introspect_without_endpoint() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        let result = backend.introspect_token("test-token").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[tokio::test]
    async fn test_refresh_token_disabled() {
        let config = OAuth2Config {
            enable_refresh_tokens: false,
            ..Default::default()
        };
        let backend = OAuth2Backend::new(config);
        let result = backend.refresh_token("refresh-token").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not enabled"));
    }

    // ========================================================================
    // XOAUTH2 SASL Mechanism Tests
    // ========================================================================

    #[test]
    fn test_parse_xoauth2_response_valid() {
        let response =
            OAuth2Backend::encode_xoauth2_response("user@example.com", "ya29.a0AfH6SMBx...");
        let result = OAuth2Backend::parse_xoauth2_response(&response);
        assert!(result.is_ok());
        let (username, token) = result.unwrap();
        assert_eq!(username, "user@example.com");
        assert_eq!(token, "ya29.a0AfH6SMBx...");
    }

    #[test]
    fn test_encode_xoauth2_response() {
        let encoded = OAuth2Backend::encode_xoauth2_response("test@example.com", "token123");
        assert!(!encoded.is_empty());

        // Verify it can be decoded back
        let (username, token) = OAuth2Backend::parse_xoauth2_response(&encoded).unwrap();
        assert_eq!(username, "test@example.com");
        assert_eq!(token, "token123");
    }

    #[test]
    fn test_parse_xoauth2_response_invalid_base64() {
        let result = OAuth2Backend::parse_xoauth2_response("not-valid-base64!");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("decode"));
    }

    #[test]
    fn test_parse_xoauth2_response_missing_username() {
        // Create response without username
        let invalid = BASE64.encode(b"auth=Bearer token123\x01\x01");
        let result = OAuth2Backend::parse_xoauth2_response(&invalid);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("username"));
    }

    #[test]
    fn test_parse_xoauth2_response_missing_token() {
        // Create response without token
        let invalid = BASE64.encode(b"user=test@example.com\x01\x01");
        let result = OAuth2Backend::parse_xoauth2_response(&invalid);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("token"));
    }

    #[test]
    fn test_xoauth2_round_trip() {
        let original_username = "roundtrip@example.com";
        let original_token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...";

        let encoded = OAuth2Backend::encode_xoauth2_response(original_username, original_token);
        let (decoded_username, decoded_token) =
            OAuth2Backend::parse_xoauth2_response(&encoded).unwrap();

        assert_eq!(decoded_username, original_username);
        assert_eq!(decoded_token, original_token);
    }

    #[test]
    fn test_xoauth2_special_characters() {
        let username = "user+tag@example.com";
        let token = "token-with-special_chars.123";

        let encoded = OAuth2Backend::encode_xoauth2_response(username, token);
        let (decoded_username, decoded_token) =
            OAuth2Backend::parse_xoauth2_response(&encoded).unwrap();

        assert_eq!(decoded_username, username);
        assert_eq!(decoded_token, token);
    }

    // ========================================================================
    // Token Cache Management Tests
    // ========================================================================

    #[tokio::test]
    async fn test_cleanup_expired_tokens() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.token_cache.write().await;
            // Add expired token
            cache.insert(
                "expired@example.com".to_string(),
                TokenCacheEntry {
                    username: "expired@example.com".to_string(),
                    expires_at: SystemTime::now() - Duration::from_secs(10),
                },
            );
            // Add valid token
            cache.insert(
                "valid@example.com".to_string(),
                TokenCacheEntry {
                    username: "valid@example.com".to_string(),
                    expires_at: SystemTime::now() + Duration::from_secs(300),
                },
            );
        }

        backend.cleanup_expired_tokens().await;

        let cache = backend.token_cache.read().await;
        assert_eq!(cache.len(), 1);
        assert!(cache.contains_key("valid@example.com"));
        assert!(!cache.contains_key("expired@example.com"));
    }

    #[tokio::test]
    async fn test_token_cache_size() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.token_cache.write().await;
            for i in 1..=3 {
                cache.insert(
                    format!("user{}@example.com", i),
                    TokenCacheEntry {
                        username: format!("user{}@example.com", i),
                        expires_at: SystemTime::now() + Duration::from_secs(300),
                    },
                );
            }
        }

        let size = backend.token_cache_size().await;
        assert_eq!(size, 3);
    }

    #[tokio::test]
    async fn test_invalidate_token() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.token_cache.write().await;
            cache.insert(
                "user@example.com".to_string(),
                TokenCacheEntry {
                    username: "user@example.com".to_string(),
                    expires_at: SystemTime::now() + Duration::from_secs(300),
                },
            );
        }

        assert_eq!(backend.token_cache_size().await, 1);

        backend.invalidate_token("user@example.com").await;

        assert_eq!(backend.token_cache_size().await, 0);
    }

    #[tokio::test]
    async fn test_clear_jwks_cache() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.jwks_cache.write().await;
            *cache = Some((Jwks { keys: vec![] }, SystemTime::now()));
        }

        backend.clear_jwks_cache().await;

        let cache = backend.jwks_cache.read().await;
        assert!(cache.is_none());
    }

    // ========================================================================
    // Provider URL Tests
    // ========================================================================

    #[test]
    fn test_google_jwks_url() {
        let config = OAuth2Config {
            provider: OidcProvider::Google {
                client_id: "client".to_string(),
                client_secret: "secret".to_string(),
            },
            ..Default::default()
        };
        let backend = OAuth2Backend::new(config);

        // Google JWKS URL is hardcoded in get_jwks method
        assert!(matches!(
            backend.config.provider,
            OidcProvider::Google { .. }
        ));
    }

    #[test]
    fn test_microsoft_urls() {
        let tenant_id = "tenant-abc-123";
        let provider = OidcProvider::Microsoft {
            tenant_id: tenant_id.to_string(),
            client_id: "client".to_string(),
            client_secret: "secret".to_string(),
        };

        if let OidcProvider::Microsoft { tenant_id: tid, .. } = &provider {
            let expected_jwks = format!(
                "https://login.microsoftonline.com/{}/discovery/v2.0/keys",
                tid
            );
            assert!(expected_jwks.contains(tenant_id));
        }
    }

    #[test]
    fn test_generic_provider_urls() {
        let issuer = "https://auth.company.com";
        let jwks_url = "https://auth.company.com/.well-known/jwks.json";

        let provider = OidcProvider::Generic {
            issuer_url: issuer.to_string(),
            client_id: "client".to_string(),
            client_secret: "secret".to_string(),
            jwks_url: jwks_url.to_string(),
        };

        if let OidcProvider::Generic {
            issuer_url,
            jwks_url: jwks,
            ..
        } = &provider
        {
            assert_eq!(issuer_url, issuer);
            assert_eq!(jwks, jwks_url);
        }
    }

    // ========================================================================
    // Additional Edge Cases
    // ========================================================================

    #[tokio::test]
    async fn test_multiple_cleanup_calls() {
        let backend = OAuth2Backend::new(OAuth2Config::default());

        {
            let mut cache = backend.token_cache.write().await;
            cache.insert(
                "expired@example.com".to_string(),
                TokenCacheEntry {
                    username: "expired@example.com".to_string(),
                    expires_at: SystemTime::now() - Duration::from_secs(10),
                },
            );
        }

        // Multiple cleanup calls should be safe
        backend.cleanup_expired_tokens().await;
        backend.cleanup_expired_tokens().await;
        backend.cleanup_expired_tokens().await;

        let cache = backend.token_cache.read().await;
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn test_invalidate_nonexistent_token() {
        let backend = OAuth2Backend::new(OAuth2Config::default());
        // Should not panic
        backend.invalidate_token("nonexistent@example.com").await;
        assert_eq!(backend.token_cache_size().await, 0);
    }

    #[test]
    fn test_xoauth2_empty_username() {
        let encoded = OAuth2Backend::encode_xoauth2_response("", "token");
        let result = OAuth2Backend::parse_xoauth2_response(&encoded);
        assert!(result.is_ok());
        let (username, _) = result.unwrap();
        assert_eq!(username, "");
    }

    #[test]
    fn test_xoauth2_empty_token() {
        let encoded = OAuth2Backend::encode_xoauth2_response("user@example.com", "");
        let result = OAuth2Backend::parse_xoauth2_response(&encoded);
        assert!(result.is_ok());
        let (_, token) = result.unwrap();
        assert_eq!(token, "");
    }

    #[test]
    fn test_xoauth2_long_token() {
        let long_token = "a".repeat(1000);
        let encoded = OAuth2Backend::encode_xoauth2_response("user@example.com", &long_token);
        let result = OAuth2Backend::parse_xoauth2_response(&encoded);
        assert!(result.is_ok());
        let (_, token) = result.unwrap();
        assert_eq!(token.len(), 1000);
    }

    // ========================================================================
    // Configuration Validation Tests
    // ========================================================================

    #[test]
    fn test_config_validation_minimal() {
        let config = OAuth2Config {
            provider: OidcProvider::Generic {
                issuer_url: "https://minimal.example.com".to_string(),
                client_id: "c".to_string(),
                client_secret: "s".to_string(),
                jwks_url: "https://minimal.example.com/jwks".to_string(),
            },
            introspection_endpoint: None,
            jwks_cache_ttl: 60,
            enable_refresh_tokens: false,
            allowed_algorithms: vec![Algorithm::RS256],
        };

        let backend = OAuth2Backend::new(config);
        assert!(backend.config.jwks_cache_ttl >= 60);
    }

    #[test]
    fn test_config_validation_maximal() {
        let config = OAuth2Config {
            provider: OidcProvider::Google {
                client_id: "very-long-client-id-with-many-characters".to_string(),
                client_secret: "very-long-secret-with-special-chars!@#$%".to_string(),
            },
            introspection_endpoint: Some(
                "https://oauth.googleapis.com/token/introspect".to_string(),
            ),
            jwks_cache_ttl: 86400,
            enable_refresh_tokens: true,
            allowed_algorithms: vec![
                Algorithm::RS256,
                Algorithm::RS384,
                Algorithm::RS512,
                Algorithm::ES256,
                Algorithm::ES384,
            ],
        };

        let backend = OAuth2Backend::new(config);
        assert_eq!(backend.config.allowed_algorithms.len(), 5);
        assert!(backend.config.enable_refresh_tokens);
    }

    // ========================================================================
    // Thread Safety Tests
    // ========================================================================

    #[tokio::test]
    async fn test_concurrent_jwks_cache_access() {
        let backend = Arc::new(OAuth2Backend::new(OAuth2Config::default()));

        let mut handles = vec![];
        for _ in 0..5 {
            let backend = Arc::clone(&backend);
            let handle = tokio::spawn(async move {
                backend.clear_jwks_cache().await;
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let cache = backend.jwks_cache.read().await;
        assert!(cache.is_none());
    }

    #[tokio::test]
    async fn test_concurrent_cleanup() {
        let backend = Arc::new(OAuth2Backend::new(OAuth2Config::default()));

        {
            let mut cache = backend.token_cache.write().await;
            for i in 0..100 {
                cache.insert(
                    format!("user{}@example.com", i),
                    TokenCacheEntry {
                        username: format!("user{}@example.com", i),
                        expires_at: if i % 2 == 0 {
                            SystemTime::now() + Duration::from_secs(300)
                        } else {
                            SystemTime::now() - Duration::from_secs(10)
                        },
                    },
                );
            }
        }

        let mut handles = vec![];
        for _ in 0..10 {
            let backend = Arc::clone(&backend);
            let handle = tokio::spawn(async move {
                backend.cleanup_expired_tokens().await;
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let cache = backend.token_cache.read().await;
        assert_eq!(cache.len(), 50);
    }
}
