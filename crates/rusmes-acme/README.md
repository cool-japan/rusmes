# rusmes-acme

ACME v2 protocol client for automatic TLS certificate management with Let's Encrypt integration.

## Features

- **ACME v2 Protocol**: Full support for RFC 8555 ACME protocol
- **Let's Encrypt Integration**: Production and staging environments
- **Challenge Types**:
  - HTTP-01: Serve challenges via HTTP server
  - DNS-01: Automated DNS TXT record management
- **Automatic Renewal**: Background task for certificate renewal
- **Certificate Management**:
  - CSR generation (RSA 2048/4096, ECDSA P-256)
  - Certificate parsing and validation
  - Expiry checking and monitoring
  - Hot-reload support
- **Storage**: Filesystem-based certificate storage with proper permissions
- **Extensible**: Pluggable DNS provider system

## Usage

### Basic Example

```rust
use rusmes_acme::{AcmeClient, AcmeConfig, ChallengeType, Http01Handler};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure ACME client
    let config = AcmeConfig::new(
        "admin@example.com".to_string(),
        vec!["example.com".to_string(), "www.example.com".to_string()],
    )
    .staging() // Use Let's Encrypt staging for testing
    .challenge_type(ChallengeType::Http01)
    .cert_paths(
        "/etc/rusmes/certs/cert.pem".to_string(),
        "/etc/rusmes/certs/key.pem".to_string(),
    )
    .renewal(30, 3600); // Renew 30 days before expiry, check every hour

    // Create HTTP-01 challenge handler
    let http_handler = Http01Handler::new();

    // Create ACME client
    let client = AcmeClient::new(config)?
        .with_http01_handler(http_handler);

    // Request certificate
    let certificate = client.request_certificate().await?;

    // Save certificate
    certificate.save("/etc/rusmes/certs/cert.pem", "/etc/rusmes/certs/key.pem").await?;

    Ok(())
}
```

### Automatic Renewal

```rust
use rusmes_acme::{AcmeClient, AcmeConfig, RenewalManager};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AcmeConfig::new(
        "admin@example.com".to_string(),
        vec!["example.com".to_string()],
    );

    let client = AcmeClient::new(config.clone())?;
    let manager = RenewalManager::new(client, config);

    // Start automatic renewal
    manager.start().await?;

    // Keep running...
    tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;

    // Stop renewal
    manager.stop().await;

    Ok(())
}
```

### DNS-01 Challenge

```rust
use rusmes_acme::{AcmeClient, AcmeConfig, ChallengeType, Dns01Handler, DnsProvider};
use async_trait::async_trait;

// Implement custom DNS provider
struct CloudflareDns {
    api_token: String,
}

#[async_trait]
impl DnsProvider for CloudflareDns {
    async fn create_txt_record(&self, domain: &str, name: &str, value: &str) -> rusmes_acme::Result<()> {
        // Call Cloudflare API to create TXT record
        Ok(())
    }

    async fn delete_txt_record(&self, domain: &str, name: &str) -> rusmes_acme::Result<()> {
        // Call Cloudflare API to delete TXT record
        Ok(())
    }

    async fn verify_txt_record(&self, domain: &str, name: &str, expected_value: &str) -> rusmes_acme::Result<bool> {
        // Query DNS to verify propagation
        Ok(true)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AcmeConfig::new(
        "admin@example.com".to_string(),
        vec!["example.com".to_string()],
    )
    .challenge_type(ChallengeType::Dns01);

    let dns_provider = Box::new(CloudflareDns {
        api_token: "your-api-token".to_string(),
    });

    let dns_handler = Dns01Handler::with_provider(dns_provider);

    let client = AcmeClient::new(config)?
        .with_dns01_handler(dns_handler);

    let certificate = client.request_certificate().await?;

    Ok(())
}
```

### HTTP-01 Server Integration

The HTTP-01 challenge handler needs to be integrated with your HTTP server:

```rust
use rusmes_acme::Http01Handler;
use std::sync::Arc;

async fn acme_challenge_handler(
    token: String,
    handler: Arc<Http01Handler>,
) -> Result<String, String> {
    handler.handle_request(&token)
        .await
        .map_err(|e| e.to_string())
}

// In your HTTP server setup:
// GET /.well-known/acme-challenge/{token} -> acme_challenge_handler
```

## Configuration

### AcmeConfig

```rust
pub struct AcmeConfig {
    /// ACME directory URL
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

    /// Days before expiry to renew certificate (default: 30)
    pub renewal_days_before_expiry: u32,

    /// Enable automatic renewal (default: true)
    pub auto_renewal: bool,

    /// Renewal check interval in seconds (default: 3600)
    pub renewal_check_interval: u64,
}
```

### Builder Methods

```rust
let config = AcmeConfig::new(email, domains)
    .staging()                           // Use Let's Encrypt staging
    .challenge_type(ChallengeType::Http01)
    .cert_paths(cert_path, key_path)
    .renewal(days_before, check_interval);
```

## Certificate Types

### KeyType

- `KeyType::Rsa2048` - RSA 2048-bit (currently mapped to ECDSA P-256)
- `KeyType::Rsa4096` - RSA 4096-bit (currently mapped to ECDSA P-256)
- `KeyType::EcdsaP256` - ECDSA P-256 (recommended)

Note: Due to rcgen limitations, RSA key generation currently falls back to ECDSA P-256.

## Kubernetes Integration

For Kubernetes deployments, you can use cert-manager annotations:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: rusmes
  annotations:
    cert-manager.io/issuer: "letsencrypt-prod"
    cert-manager.io/cluster-issuer: "letsencrypt-prod"
```

Or use the built-in ACME client with a persistent volume for certificates:

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: rusmes-certs
spec:
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 1Gi
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rusmes
spec:
  template:
    spec:
      volumes:
        - name: certs
          persistentVolumeClaim:
            claimName: rusmes-certs
      containers:
        - name: rusmes
          volumeMounts:
            - name: certs
              mountPath: /etc/rusmes/certs
          env:
            - name: ACME_EMAIL
              value: "admin@example.com"
            - name: ACME_DOMAINS
              value: "mail.example.com"
```

## Testing

Run the test suite:

```bash
cargo test
```

Run specific test:

```bash
cargo test test_http01_challenge_flow
```

Run with output:

```bash
cargo test -- --nocapture
```

## Security Considerations

1. **Private Key Protection**: Private keys are stored with 0600 permissions on Unix systems
2. **Email Contact**: Provide a valid email for Let's Encrypt notifications
3. **Rate Limits**: Use staging environment for testing to avoid hitting production rate limits
4. **DNS Propagation**: DNS-01 challenges wait for propagation before validation
5. **Certificate Validation**: Certificates are validated before use

## Rate Limits

Let's Encrypt has the following rate limits:

- **Production**: 50 certificates per registered domain per week
- **Staging**: Much higher limits for testing
- **Failed Validation**: 5 failures per account per hostname per hour

Always use the staging environment during development!

## Error Handling

The crate provides detailed error types:

```rust
pub enum AcmeError {
    Protocol(String),           // ACME protocol errors
    ChallengeFailed(String),    // Challenge validation errors
    ValidationFailed(String),   // Certificate validation errors
    Storage(String),            // Storage errors
    Http(reqwest::Error),       // HTTP errors
    Io(std::io::Error),         // I/O errors
    Json(serde_json::Error),    // JSON errors
    Other(String),              // Other errors
}
```

## Contributing

When contributing, ensure:

1. All tests pass: `cargo test`
2. No warnings: `cargo build 2>&1 | grep warning`
3. Code is formatted: `cargo fmt`
4. Clippy is happy: `cargo clippy`

## License

This crate is part of the rusmes project and follows the same license.

## References

- [RFC 8555 - ACME Protocol](https://www.rfc-editor.org/rfc/rfc8555.html)
- [Let's Encrypt Documentation](https://letsencrypt.org/docs/)
- [instant-acme crate](https://docs.rs/instant-acme/)
