//! OAuth2/OIDC Authentication Integration Example
//!
//! This example demonstrates how to use the OAuth2 backend for authentication
//! with different providers (Google, Microsoft, Generic OIDC).
//!
//! Run with:
//! ```bash
//! cargo run --example oauth2_integration
//! ```

use rusmes_auth::backends::oauth2::{OAuth2Backend, OAuth2Config, OidcProvider};
use rusmes_auth::AuthBackend;
use rusmes_proto::Username;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== OAuth2/OIDC Authentication Backend Examples ===\n");

    // Example 1: Google OAuth2
    println!("1. Google OAuth2 Configuration");
    let google_config = OAuth2Config {
        provider: OidcProvider::Google {
            client_id: "your-google-client-id.apps.googleusercontent.com".to_string(),
            client_secret: "your-google-client-secret".to_string(),
        },
        introspection_endpoint: Some("https://oauth2.googleapis.com/tokeninfo".to_string()),
        jwks_cache_ttl: 3600,
        enable_refresh_tokens: true,
        allowed_algorithms: vec![jsonwebtoken::Algorithm::RS256],
    };
    let google_backend = OAuth2Backend::new(google_config);
    println!("   ✓ Google OAuth2 backend created");
    println!("   - JWKS URL: https://www.googleapis.com/oauth2/v3/certs");
    println!("   - Token introspection: enabled");
    println!();

    // Example 2: Microsoft Azure AD
    println!("2. Microsoft Azure AD Configuration");
    let microsoft_config = OAuth2Config {
        provider: OidcProvider::Microsoft {
            tenant_id: "your-tenant-id".to_string(),
            client_id: "your-azure-client-id".to_string(),
            client_secret: "your-azure-client-secret".to_string(),
        },
        introspection_endpoint: None,
        jwks_cache_ttl: 7200,
        enable_refresh_tokens: true,
        allowed_algorithms: vec![
            jsonwebtoken::Algorithm::RS256,
            jsonwebtoken::Algorithm::RS384,
        ],
    };
    let _microsoft_backend = OAuth2Backend::new(microsoft_config);
    println!("   ✓ Microsoft Azure AD backend created");
    println!("   - Tenant-specific endpoints");
    println!("   - Multiple algorithms supported");
    println!();

    // Example 3: Generic OIDC Provider
    println!("3. Generic OIDC Provider Configuration");
    let generic_config = OAuth2Config {
        provider: OidcProvider::Generic {
            issuer_url: "https://auth.example.com".to_string(),
            client_id: "your-oidc-client-id".to_string(),
            client_secret: "your-oidc-client-secret".to_string(),
            jwks_url: "https://auth.example.com/.well-known/jwks.json".to_string(),
        },
        introspection_endpoint: Some("https://auth.example.com/oauth/introspect".to_string()),
        jwks_cache_ttl: 1800,
        enable_refresh_tokens: true,
        allowed_algorithms: vec![
            jsonwebtoken::Algorithm::RS256,
            jsonwebtoken::Algorithm::ES256,
        ],
    };
    let _generic_backend = OAuth2Backend::new(generic_config);
    println!("   ✓ Generic OIDC backend created");
    println!("   - Custom issuer URL");
    println!("   - Custom JWKS endpoint");
    println!();

    // Example 4: XOAUTH2 SASL Mechanism
    println!("4. XOAUTH2 SASL Mechanism Example");
    let username = "user@example.com";
    let access_token = "ya29.a0AfH6SMBx...";
    let xoauth2_encoded = OAuth2Backend::encode_xoauth2_response(username, access_token);
    println!("   ✓ XOAUTH2 response encoded");
    println!(
        "   - Format: base64(user={}\\x01auth=Bearer {}\\x01\\x01)",
        username, access_token
    );
    println!("   - Encoded: {}...", &xoauth2_encoded[..40]);

    let (decoded_user, decoded_token) = OAuth2Backend::parse_xoauth2_response(&xoauth2_encoded)?;
    println!("   ✓ XOAUTH2 response decoded");
    println!("   - Username: {}", decoded_user);
    let token_preview = if decoded_token.len() > 20 {
        &decoded_token[..20]
    } else {
        &decoded_token
    };
    println!("   - Token: {}...", token_preview);
    println!();

    // Example 5: Token Cache Management
    println!("5. Token Cache Management");
    let cache_size_before = google_backend.token_cache_size().await;
    println!("   - Cache size before: {}", cache_size_before);

    // Simulate authentication (would fail in real scenario without valid token)
    let test_username = Username::new("test@example.com".to_string())?;
    let result = google_backend
        .authenticate(&test_username, "invalid-token")
        .await;
    println!("   - Authentication with invalid token: {}", result.is_ok());

    // Cleanup expired tokens
    google_backend.cleanup_expired_tokens().await;
    println!("   ✓ Expired tokens cleaned up");

    // Clear JWKS cache
    google_backend.clear_jwks_cache().await;
    println!("   ✓ JWKS cache cleared");
    println!();

    // Example 6: Backend Capabilities
    println!("6. OAuth2 Backend Capabilities");
    println!("   ✓ JWT validation with JWKS");
    println!("   ✓ Token introspection endpoint support");
    println!("   ✓ Refresh token handling");
    println!("   ✓ Multiple OIDC providers");
    println!("   ✓ Token caching with TTL");
    println!("   ✓ XOAUTH2 SASL mechanism");
    println!("   ✓ Concurrent access safety");
    println!();

    // Example 7: Error Handling
    println!("7. Error Handling Examples");

    // User management not supported
    let result = google_backend.create_user(&test_username, "token").await;
    match result {
        Err(e) if e.to_string().contains("external provider") => {
            println!("   ✓ User creation correctly rejected (external provider)");
        }
        _ => println!("   ✗ Unexpected result for user creation"),
    }

    let result = google_backend.delete_user(&test_username).await;
    match result {
        Err(e) if e.to_string().contains("external provider") => {
            println!("   ✓ User deletion correctly rejected (external provider)");
        }
        _ => println!("   ✗ Unexpected result for user deletion"),
    }

    let result = google_backend
        .change_password(&test_username, "new-token")
        .await;
    match result {
        Err(e) if e.to_string().contains("external provider") => {
            println!("   ✓ Password change correctly rejected (external provider)");
        }
        _ => println!("   ✗ Unexpected result for password change"),
    }
    println!();

    // Example 8: Configuration Best Practices
    println!("8. Configuration Best Practices");
    println!("   - Use appropriate JWKS cache TTL (1-2 hours recommended)");
    println!("   - Enable refresh tokens for better UX");
    println!("   - Use RS256 algorithm for maximum compatibility");
    println!("   - Configure introspection endpoint for opaque tokens");
    println!("   - Implement token rotation for security");
    println!("   - Use environment variables for client secrets");
    println!();

    // Example 9: IMAP/SMTP Integration
    println!("9. IMAP/SMTP XOAUTH2 Integration");
    println!("   IMAP XOAUTH2 Authentication:");
    println!("   > A001 AUTHENTICATE XOAUTH2");
    println!("   < +");
    println!("   > <base64-encoded-xoauth2-response>");
    println!("   < A001 OK Authenticated");
    println!();
    println!("   SMTP XOAUTH2 Authentication:");
    println!("   > AUTH XOAUTH2");
    println!("   < 334");
    println!("   > <base64-encoded-xoauth2-response>");
    println!("   < 235 2.7.0 Authentication successful");
    println!();

    // Example 10: Production Deployment
    println!("10. Production Deployment Checklist");
    println!("   □ Store client secrets in secure vault (not in code)");
    println!("   □ Use HTTPS for all OAuth endpoints");
    println!("   □ Implement rate limiting for token validation");
    println!("   □ Monitor token cache hit rate");
    println!("   □ Set up alerting for authentication failures");
    println!("   □ Regularly rotate client secrets");
    println!("   □ Configure appropriate token TTLs");
    println!("   □ Test with multiple providers");
    println!("   □ Implement graceful degradation");
    println!("   □ Document OAuth flow for users");
    println!();

    println!("=== Example Complete ===");
    println!("\nFor production use, configure with real OAuth2 credentials:");
    println!("1. Register application with OAuth provider");
    println!("2. Configure redirect URIs");
    println!("3. Obtain client ID and secret");
    println!("4. Set up JWKS endpoint access");
    println!("5. Test authentication flow");

    Ok(())
}
