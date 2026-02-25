//! Integration tests for ACME functionality

use rusmes_acme::*;
use tempfile::tempdir;

#[tokio::test]
async fn test_certificate_lifecycle() {
    let dir = tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");

    // Create certificate with proper data
    let cert = Certificate::new(
        "TEST_CERT_PEM_DATA".to_string(),
        "TEST_KEY_PEM_DATA".to_string(),
        "TEST_CHAIN_PEM_DATA".to_string(),
        vec!["example.com".to_string()],
        chrono::Utc::now() + chrono::Duration::days(90),
        chrono::Utc::now(),
    );

    // Save certificate
    cert.save(cert_path.to_str().unwrap(), key_path.to_str().unwrap())
        .await
        .unwrap();

    // Verify files exist
    assert!(cert_path.exists());
    assert!(key_path.exists());

    // Read back the raw data
    let saved_cert = tokio::fs::read_to_string(&cert_path).await.unwrap();
    let saved_key = tokio::fs::read_to_string(&key_path).await.unwrap();

    assert_eq!(saved_cert, "TEST_CERT_PEM_DATA");
    assert_eq!(saved_key, "TEST_KEY_PEM_DATA");
}

#[tokio::test]
async fn test_http01_challenge_flow() {
    let handler = Http01Handler::new();

    // Add challenge
    handler
        .add_challenge("token123".to_string(), "key_auth_123".to_string())
        .await;

    // Handle request
    let response = handler.handle_request("token123").await.unwrap();
    assert_eq!(response, "key_auth_123");

    // Remove challenge
    handler.remove_challenge("token123").await;

    // Verify removed
    let result = handler.handle_request("token123").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_dns01_challenge_flow() {
    use rusmes_acme::dns01::MockDnsProvider;

    let provider = Box::new(MockDnsProvider::new());
    let handler = Dns01Handler::with_provider(provider);

    // Setup challenge
    handler
        .setup("example.com", "dns_value_123".to_string())
        .await
        .unwrap();

    // Verify propagation
    let verified = handler
        .verify_propagation("example.com", "dns_value_123")
        .await
        .unwrap();
    assert!(verified);

    // Cleanup
    handler.cleanup("example.com").await.unwrap();

    // Verify cleaned up
    let verified = handler
        .verify_propagation("example.com", "dns_value_123")
        .await
        .unwrap();
    assert!(!verified);
}

#[tokio::test]
async fn test_certificate_storage() {
    let dir = tempdir().unwrap();
    let storage =
        storage::CertificateStorage::new(dir.path().to_str().unwrap().to_string()).unwrap();

    // Save certificate
    storage
        .save_certificate("example.com", "cert_pem", "key_pem", "chain_pem")
        .await
        .unwrap();

    // Check exists
    assert!(storage.certificate_exists("example.com"));

    // Load certificate
    let (cert, key, chain) = storage.load_certificate("example.com").await.unwrap();
    assert_eq!(cert, "cert_pem");
    assert_eq!(key, "key_pem");
    assert_eq!(chain, "chain_pem");

    // List domains
    let domains = storage.list_domains().await.unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0], "example.com");

    // Delete certificate
    storage.delete_certificate("example.com").await.unwrap();
    assert!(!storage.certificate_exists("example.com"));
}

#[tokio::test]
async fn test_renewal_manager_check() {
    let dir = tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");

    let config = AcmeConfig::new(
        "test@example.com".to_string(),
        vec!["example.com".to_string()],
    )
    .staging()
    .cert_paths(
        cert_path.to_str().unwrap().to_string(),
        key_path.to_str().unwrap().to_string(),
    );

    let client = AcmeClient::new(config.clone()).unwrap();
    let manager = RenewalManager::new(client, config);

    // Check renewal when cert doesn't exist
    let should_renew = manager.check_renewal().await.unwrap();
    assert!(should_renew);
}

#[tokio::test]
async fn test_csr_generation_all_key_types() {
    let domains = vec!["example.com".to_string(), "www.example.com".to_string()];

    // RSA 2048
    let (csr, key) = CsrGenerator::generate(domains.clone(), KeyType::Rsa2048).unwrap();
    assert!(csr.contains("BEGIN CERTIFICATE REQUEST"));
    assert!(key.contains("PRIVATE KEY"));

    // RSA 4096
    let (csr, key) = CsrGenerator::generate(domains.clone(), KeyType::Rsa4096).unwrap();
    assert!(csr.contains("BEGIN CERTIFICATE REQUEST"));
    assert!(key.contains("PRIVATE KEY"));

    // ECDSA P-256
    let (csr, key) = CsrGenerator::generate(domains.clone(), KeyType::EcdsaP256).unwrap();
    assert!(csr.contains("BEGIN CERTIFICATE REQUEST"));
    assert!(key.contains("PRIVATE KEY"));
}

#[test]
fn test_config_validation_rules() {
    // Valid config
    let config = AcmeConfig::new(
        "test@example.com".to_string(),
        vec!["example.com".to_string()],
    );
    assert!(config.validate().is_ok());

    // Empty email
    let config = AcmeConfig {
        email: String::new(),
        ..Default::default()
    };
    assert!(config.validate().is_err());

    // No domains
    let config = AcmeConfig {
        domains: vec![],
        ..Default::default()
    };
    assert!(config.validate().is_err());

    // Invalid renewal days (0)
    let config = AcmeConfig {
        renewal_days_before_expiry: 0,
        ..Default::default()
    };
    assert!(config.validate().is_err());

    // Invalid renewal days (> 90)
    let config = AcmeConfig {
        renewal_days_before_expiry: 91,
        ..Default::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn test_challenge_type_serde() {
    use serde_json;

    // Serialize
    let http01 = ChallengeType::Http01;
    let json = serde_json::to_string(&http01).unwrap();
    assert_eq!(json, "\"http01\"");

    let dns01 = ChallengeType::Dns01;
    let json = serde_json::to_string(&dns01).unwrap();
    assert_eq!(json, "\"dns01\"");

    // Deserialize
    let http01: ChallengeType = serde_json::from_str("\"http01\"").unwrap();
    assert_eq!(http01, ChallengeType::Http01);

    let dns01: ChallengeType = serde_json::from_str("\"dns01\"").unwrap();
    assert_eq!(dns01, ChallengeType::Dns01);
}

#[tokio::test]
async fn test_multiple_domain_certificate() {
    let domains = vec![
        "example.com".to_string(),
        "www.example.com".to_string(),
        "mail.example.com".to_string(),
    ];

    let (csr, _key) = CsrGenerator::generate(domains.clone(), KeyType::Rsa2048).unwrap();
    assert!(csr.contains("BEGIN CERTIFICATE REQUEST"));
}

#[tokio::test]
async fn test_challenge_manager_operations() {
    use rusmes_acme::challenge::ChallengeManager;

    let manager = ChallengeManager::new();

    // Add multiple challenges
    manager
        .add_http_challenge("token1".to_string(), "auth1".to_string())
        .await;
    manager
        .add_http_challenge("token2".to_string(), "auth2".to_string())
        .await;

    // Get challenges
    assert_eq!(
        manager.get_http_challenge("token1").await,
        Some("auth1".to_string())
    );
    assert_eq!(
        manager.get_http_challenge("token2").await,
        Some("auth2".to_string())
    );

    // Clear all
    manager.clear().await;

    assert_eq!(manager.get_http_challenge("token1").await, None);
    assert_eq!(manager.get_http_challenge("token2").await, None);
}

#[test]
fn test_certificate_expiry_calculations() {
    let now = chrono::Utc::now();

    // Expired certificate
    let cert = Certificate::new(
        "cert".to_string(),
        "key".to_string(),
        "chain".to_string(),
        vec!["example.com".to_string()],
        now - chrono::Duration::days(1),
        now - chrono::Duration::days(91),
    );
    assert!(cert.is_expired());
    assert!(cert.days_until_expiry() < 0);

    // Valid certificate, no renewal needed
    let cert = Certificate::new(
        "cert".to_string(),
        "key".to_string(),
        "chain".to_string(),
        vec!["example.com".to_string()],
        now + chrono::Duration::days(60),
        now - chrono::Duration::days(30),
    );
    assert!(!cert.is_expired());
    assert!(!cert.should_renew(30));

    // Valid certificate, renewal needed
    let cert = Certificate::new(
        "cert".to_string(),
        "key".to_string(),
        "chain".to_string(),
        vec!["example.com".to_string()],
        now + chrono::Duration::days(20),
        now - chrono::Duration::days(70),
    );
    assert!(!cert.is_expired());
    assert!(cert.should_renew(30));
}

#[tokio::test]
async fn test_dns01_wait_for_propagation() {
    use rusmes_acme::dns01::MockDnsProvider;

    let provider = Box::new(MockDnsProvider::new());
    let handler = Dns01Handler::with_provider(provider);

    // Setup challenge
    handler
        .setup("example.com", "test_value".to_string())
        .await
        .unwrap();

    // Wait for propagation (should succeed immediately with mock)
    let result = handler
        .wait_for_propagation("example.com", "test_value", 3, 1)
        .await;
    assert!(result.is_ok());

    // Wait for wrong value (should timeout)
    let result = handler
        .wait_for_propagation("example.com", "wrong_value", 2, 1)
        .await;
    assert!(result.is_err());
}

#[test]
fn test_well_known_path_format() {
    let path = Http01Handler::well_known_path("test_token_123");
    assert_eq!(path, "/.well-known/acme-challenge/test_token_123");
}

#[test]
fn test_acme_error_types() {
    use rusmes_acme::AcmeError;

    let err = AcmeError::Protocol("test".to_string());
    assert_eq!(err.to_string(), "ACME protocol error: test");

    let err = AcmeError::ChallengeFailed("test".to_string());
    assert_eq!(err.to_string(), "Challenge failed: test");

    let err = AcmeError::ValidationFailed("test".to_string());
    assert_eq!(err.to_string(), "Certificate validation failed: test");
}
