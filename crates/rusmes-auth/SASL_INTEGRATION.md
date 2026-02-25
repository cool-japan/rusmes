# SASL Framework Integration Guide

## Overview

The RusMES SASL (Simple Authentication and Security Layer) framework provides a comprehensive, protocol-agnostic authentication system supporting multiple mechanisms. This implementation follows RFC 4422 and mechanism-specific RFCs.

## Supported Mechanisms

| Mechanism | RFC | Security | Status | Use Case |
|-----------|-----|----------|--------|----------|
| PLAIN | RFC 4616 | Low (requires TLS) | ✅ Complete | Simple password auth |
| LOGIN | Legacy | Low (requires TLS) | ✅ Complete | Legacy clients |
| CRAM-MD5 | RFC 2195 | Medium | ✅ Complete | Challenge-response |
| SCRAM-SHA-256 | RFC 5802, 7677 | High | ✅ Complete | Modern secure auth |
| XOAUTH2 | RFC 7628 | High | ✅ Complete | OAuth 2.0 tokens |

## Architecture

### Core Components

```rust
use rusmes_auth::sasl::{
    SaslConfig,      // Configuration
    SaslServer,      // Server instance
    SaslMechanism,   // Trait for mechanisms
    SaslStep,        // Authentication step result
    SaslState,       // Current state
};
```

### Authentication Flow

```
┌──────────────┐                    ┌──────────────┐
│   Client     │                    │   Server     │
└──────┬───────┘                    └──────┬───────┘
       │                                   │
       │  1. Request Authentication        │
       │──────────────────────────────────>│
       │                                   │
       │  2. Challenge (if needed)         │
       │<──────────────────────────────────│
       │                                   │
       │  3. Response                      │
       │──────────────────────────────────>│
       │                                   │
       │  4. Success/Failure               │
       │<──────────────────────────────────│
       │                                   │
```

## Quick Start

### 1. Setup

Add to your `Cargo.toml`:

```toml
[dependencies]
rusmes-auth = { path = "../rusmes-auth" }
```

### 2. Configure SASL Server

```rust
use rusmes_auth::sasl::{SaslConfig, SaslServer};

let config = SaslConfig {
    enabled_mechanisms: vec![
        "PLAIN".to_string(),
        "LOGIN".to_string(),
        "CRAM-MD5".to_string(),
        "SCRAM-SHA-256".to_string(),
        "XOAUTH2".to_string(),
    ],
    hostname: "mail.example.com".to_string(),
};

let sasl_server = SaslServer::new(config);
```

### 3. Authenticate User

```rust
use rusmes_auth::sasl::SaslStep;

let mut mechanism = sasl_server.create_mechanism("PLAIN")?;
let response = b"\0username\0password";

match mechanism.step(response, &auth_backend).await? {
    SaslStep::Done { success, username } => {
        if success {
            println!("Authenticated as: {:?}", username);
        }
    }
    SaslStep::Challenge { data } => {
        // Send challenge to client
    }
    SaslStep::Continue => {
        // Wait for more data
    }
}
```

## Mechanism Details

### PLAIN (RFC 4616)

**Format:** `\0authzid\0authcid\0password`

**Security:** Low - transmits password in clear text (requires TLS)

**Example:**
```rust
// Client sends: \0alice\0secret123
let mut mechanism = sasl_server.create_mechanism("PLAIN")?;
let response = b"\0alice\0secret123";
let result = mechanism.step(response, &auth_backend).await?;
```

**Use cases:**
- Simple authentication
- Must be used with TLS/SSL
- Good for testing

### LOGIN (Legacy)

**Format:** Multi-step base64-encoded username and password

**Security:** Low - transmits password in clear text (requires TLS)

**Flow:**
1. Server: `334 VXNlcm5hbWU6` (base64: "Username:")
2. Client: `YWxpY2U=` (base64: "alice")
3. Server: `334 UGFzc3dvcmQ6` (base64: "Password:")
4. Client: `c2VjcmV0MTIz` (base64: "secret123")
5. Server: `235 Authentication successful`

**Example:**
```rust
let mut mechanism = sasl_server.create_mechanism("LOGIN")?;

// Step 1: Initial
let result = mechanism.step(b"", &auth_backend).await?;

// Step 2: Username
let result = mechanism.step(b"alice", &auth_backend).await?;

// Step 3: Password
let result = mechanism.step(b"secret123", &auth_backend).await?;
```

### CRAM-MD5 (RFC 2195)

**Format:** Challenge-response using HMAC-MD5

**Security:** Medium - password not transmitted, but MD5 is weak

**Flow:**
1. Server sends challenge: `<timestamp.random@hostname>`
2. Client computes: `HMAC-MD5(password, challenge)`
3. Client sends: `username hmac`

**Example:**
```rust
let mut mechanism = sasl_server.create_mechanism("CRAM-MD5")?;

// Server sends challenge
let result = mechanism.step(b"", &auth_backend).await?;

// Client sends username + HMAC
let response = b"alice 3c6e0b8a9c15224a8228b9a98ca1531d";
let result = mechanism.step(response, &auth_backend).await?;
```

**Note:** CRAM-MD5 requires backend support for password verification or plaintext password storage.

### SCRAM-SHA-256 (RFC 5802, RFC 7677)

**Format:** Salted Challenge Response Authentication Mechanism

**Security:** High - uses PBKDF2, salting, and SHA-256

**Flow:**
1. Client sends: `n,,n=username,r=clientNonce`
2. Server sends: `r=clientNonce+serverNonce,s=salt,i=iterations`
3. Client sends: `c=biws,r=nonce,p=proof`
4. Server sends: `v=serverSignature`

**Example:**
```rust
let mut mechanism = sasl_server.create_mechanism("SCRAM-SHA-256")?;

// Client first message
let client_first = b"n,,n=alice,r=rOprNGfwEbeRWgbNEkqO";
let result = mechanism.step(client_first, &auth_backend).await?;

// Client final message
let client_final = b"c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=...";
let result = mechanism.step(client_final, &auth_backend).await?;
```

**Requirements:**
- Backend must implement `get_scram_params()`, `get_scram_stored_key()`, `get_scram_server_key()`
- Use `store_scram_credentials()` when setting passwords

### XOAUTH2 (RFC 7628)

**Format:** OAuth 2.0 bearer token authentication

**Security:** High - uses OAuth tokens

**Format:** `user=username\x01auth=Bearer token\x01\x01`

**Example:**
```rust
let mut mechanism = sasl_server.create_mechanism("XOAUTH2")?;

let response = b"user=alice@example.com\x01auth=Bearer ya29.AHES6Z...\x01\x01";
let result = mechanism.step(response, &auth_backend).await?;
```

**Use cases:**
- Gmail OAuth integration
- Modern web applications
- Token-based authentication

## Protocol Integration

### SMTP (RFC 4954)

```rust
// In EHLO response
println!("250-AUTH PLAIN LOGIN CRAM-MD5 SCRAM-SHA-256");

// Handle AUTH command
if line.starts_with("AUTH") {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let mechanism_name = parts[1];
    let initial_response = parts.get(2);

    let mut mechanism = sasl_server.create_mechanism(mechanism_name)?;

    if let Some(resp) = initial_response {
        let decoded = base64::decode(resp)?;
        let result = mechanism.step(&decoded, &auth_backend).await?;
        // Handle result
    }
}
```

### IMAP (RFC 3501)

```rust
// In CAPABILITY response
println!("* CAPABILITY IMAP4rev1 AUTH=PLAIN AUTH=CRAM-MD5 AUTH=SCRAM-SHA-256");

// Handle AUTHENTICATE command
if line.starts_with("A001 AUTHENTICATE") {
    let mechanism_name = line.split_whitespace().nth(2).unwrap();
    let mut mechanism = sasl_server.create_mechanism(mechanism_name)?;

    loop {
        match mechanism.step(&client_data, &auth_backend).await? {
            SaslStep::Done { success, username } => {
                if success {
                    println!("A001 OK Authenticated");
                } else {
                    println!("A001 NO Authentication failed");
                }
                break;
            }
            SaslStep::Challenge { data } => {
                println!("+ {}", base64::encode(&data));
            }
            SaslStep::Continue => {
                println!("+");
            }
        }
    }
}
```

### POP3 (RFC 5034)

```rust
// In CAPA response
println!("+OK");
println!("SASL PLAIN CRAM-MD5");
println!(".");

// Handle AUTH command
if line.starts_with("AUTH") {
    let mechanism_name = line.split_whitespace().nth(1).unwrap();
    let mut mechanism = sasl_server.create_mechanism(mechanism_name)?;

    match mechanism.step(&client_data, &auth_backend).await? {
        SaslStep::Done { success, .. } => {
            if success {
                println!("+OK Logged in");
            } else {
                println!("-ERR Authentication failed");
            }
        }
        SaslStep::Challenge { data } => {
            println!("+ {}", base64::encode(&data));
        }
        _ => {}
    }
}
```

## Backend Implementation

### Required Methods

All backends must implement:

```rust
#[async_trait]
pub trait AuthBackend: Send + Sync {
    async fn authenticate(&self, username: &Username, password: &str) -> Result<bool>;
    async fn verify_identity(&self, username: &Username) -> Result<bool>;
    // ... other methods
}
```

### SCRAM-SHA-256 Support

For SCRAM-SHA-256, backends must implement:

```rust
async fn get_scram_params(&self, username: &str) -> Result<(Vec<u8>, u32)>;
async fn get_scram_stored_key(&self, username: &str) -> Result<Vec<u8>>;
async fn get_scram_server_key(&self, username: &str) -> Result<Vec<u8>>;
async fn store_scram_credentials(
    &self,
    username: &Username,
    salt: Vec<u8>,
    iterations: u32,
    stored_key: Vec<u8>,
    server_key: Vec<u8>,
) -> Result<()>;
```

## Security Considerations

### TLS Requirement

**ALWAYS use TLS/SSL with PLAIN and LOGIN:**

```rust
if !connection.is_encrypted() && (mechanism == "PLAIN" || mechanism == "LOGIN") {
    return Err("TLS required for PLAIN/LOGIN authentication");
}
```

### Rate Limiting

Implement rate limiting to prevent brute force attacks:

```rust
use std::collections::HashMap;
use std::time::{Duration, Instant};

struct RateLimiter {
    attempts: HashMap<String, Vec<Instant>>,
    max_attempts: usize,
    window: Duration,
}

impl RateLimiter {
    fn check(&mut self, username: &str) -> bool {
        let now = Instant::now();
        let attempts = self.attempts.entry(username.to_string()).or_insert_with(Vec::new);

        // Remove old attempts
        attempts.retain(|&time| now.duration_since(time) < self.window);

        if attempts.len() >= self.max_attempts {
            return false;
        }

        attempts.push(now);
        true
    }
}
```

### Logging

Log authentication attempts:

```rust
match result {
    SaslStep::Done { success, username } => {
        if success {
            log::info!("Successful authentication for {:?}", username);
        } else {
            log::warn!("Failed authentication attempt for {:?}", username);
        }
    }
    _ => {}
}
```

### Mechanism Selection

Prefer stronger mechanisms:

```rust
let config = SaslConfig {
    enabled_mechanisms: vec![
        "SCRAM-SHA-256".to_string(),  // Strongest
        "CRAM-MD5".to_string(),        // Medium
        "PLAIN".to_string(),           // Weakest (TLS only)
    ],
    hostname: "mail.example.com".to_string(),
};
```

## Testing

### Unit Tests

Run mechanism tests:

```bash
cargo test -p rusmes-auth --lib sasl
```

All 33 tests should pass:
- 5 PLAIN tests
- 3 LOGIN tests
- 6 CRAM-MD5 tests
- 8 SCRAM-SHA-256 tests
- 4 XOAUTH2 tests
- 7 SASL Server tests

### Integration Testing

See `examples/sasl_integration.rs` for a complete integration example:

```bash
cargo run -p rusmes-auth --example sasl_integration
```

## Performance

### Benchmarks

SCRAM-SHA-256 is the most computationally expensive (PBKDF2 iterations):
- PLAIN: ~1μs
- LOGIN: ~1μs
- CRAM-MD5: ~10μs
- SCRAM-SHA-256: ~50ms (4096 iterations)
- XOAUTH2: ~1μs

### Optimization Tips

1. **Cache SCRAM credentials** - Don't recompute on every authentication
2. **Adjust PBKDF2 iterations** - Balance security vs. performance
3. **Use connection pooling** - Reuse auth backends
4. **Implement early returns** - Check username existence before expensive operations

## Troubleshooting

### Common Issues

**1. "Mechanism not supported"**

```rust
// Ensure mechanism is enabled in config
let config = SaslConfig {
    enabled_mechanisms: vec!["PLAIN".to_string()],
    // ...
};
```

**2. "SCRAM-SHA-256 credential storage not implemented"**

```rust
// Backend must implement SCRAM methods
impl AuthBackend for MyBackend {
    async fn get_scram_params(&self, username: &str) -> Result<(Vec<u8>, u32)> {
        // Return (salt, iterations)
    }
    // ...
}
```

**3. "Invalid base64 encoding"**

```rust
// Ensure proper base64 encoding
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
let encoded = BASE64.encode(data);
```

## References

- [RFC 4422 - SASL](https://tools.ietf.org/html/rfc4422)
- [RFC 4616 - PLAIN](https://tools.ietf.org/html/rfc4616)
- [RFC 2195 - CRAM-MD5](https://tools.ietf.org/html/rfc2195)
- [RFC 5802 - SCRAM](https://tools.ietf.org/html/rfc5802)
- [RFC 7677 - SCRAM-SHA-256](https://tools.ietf.org/html/rfc7677)
- [RFC 7628 - XOAUTH2](https://tools.ietf.org/html/rfc7628)
- [RFC 4954 - SMTP AUTH](https://tools.ietf.org/html/rfc4954)
- [RFC 3501 - IMAP](https://tools.ietf.org/html/rfc3501)
- [RFC 5034 - POP3 SASL](https://tools.ietf.org/html/rfc5034)

## License

Apache-2.0
