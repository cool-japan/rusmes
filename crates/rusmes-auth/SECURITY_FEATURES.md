# Authentication Security Module

The `rusmes-auth` security module provides comprehensive authentication security features for RusMES.

## Features

### 1. Brute Force Protection

Protects against brute force attacks with configurable account lockout:

- Tracks failed authentication attempts per user and IP address
- Locks accounts after exceeding maximum failed attempts
- Configurable lockout duration and attempt window
- Progressive lockout: duration doubles with each subsequent lockout
- Separate tracking for user accounts and IP addresses
- Administrative functions to manually unlock accounts

**Configuration:**
```rust
use rusmes_auth::security::BruteForceConfig;

let config = BruteForceConfig {
    max_attempts: 5,              // Lock after 5 failed attempts
    lockout_duration_secs: 900,   // 15 minutes lockout
    attempt_window_secs: 300,     // Count attempts in 5-minute window
    progressive_lockout: true,    // Enable progressive lockout
};
```

### 2. Password Strength Validation

Enforces strong password policies with configurable rules:

- Minimum password length
- Required character types (uppercase, lowercase, digits, special)
- Shannon entropy calculation
- Banned password list (common passwords)
- Detailed validation error messages

**Configuration:**
```rust
use rusmes_auth::security::PasswordStrengthConfig;

let config = PasswordStrengthConfig {
    min_length: 10,
    require_uppercase: true,
    require_lowercase: true,
    require_digit: true,
    require_special: true,
    min_entropy_bits: 3.5,
    banned_passwords: vec![
        "password".to_string(),
        "admin".to_string(),
    ],
};
```

### 3. Rate Limiting

Prevents abuse by limiting authentication attempts per IP address:

- Configurable request limits per time window
- Automatic cleanup of old request records
- Per-IP tracking
- Administrative reset function

**Configuration:**
```rust
use rusmes_auth::security::RateLimitConfig;

let config = RateLimitConfig {
    max_requests: 10,   // 10 requests
    window_secs: 60,    // per minute
};
```

### 4. Audit Logging

Comprehensive audit logging for security monitoring:

- Logs all authentication events (success, failure, lockout, etc.)
- Includes timestamps, usernames, and IP addresses
- Query logs by user or recent entries
- Configurable in-memory storage (production should use external storage)

**Events Logged:**
- Successful authentication
- Failed authentication
- Account locked
- Rate limit exceeded
- Password changed
- User created
- User deleted

## Usage

### Basic Setup

```rust
use rusmes_auth::security::{AuthSecurity, SecurityConfig};

// Create with default configuration
let security = AuthSecurity::new(SecurityConfig::default());

// Or use builder pattern for custom configuration
use rusmes_auth::security::AuthSecurityBuilder;

let security = AuthSecurityBuilder::new()
    .brute_force_config(brute_force_config)
    .password_strength_config(password_config)
    .rate_limit_config(rate_limit_config)
    .enable_audit_log(true)
    .build();
```

### Integration with AuthBackend

```rust
use rusmes_auth::{AuthBackend, security::AuthSecurity};
use std::net::IpAddr;

async fn secure_authenticate<B: AuthBackend>(
    backend: &B,
    security: &AuthSecurity,
    username: &str,
    password: &str,
    ip: Option<IpAddr>,
) -> anyhow::Result<bool> {
    // Check if authentication is allowed
    security.check_auth_attempt(username, ip).await?;

    // Perform authentication
    let result = backend.authenticate(&username.parse()?, password).await?;

    if result {
        // Record success
        security.record_auth_success(username, ip).await;
        Ok(true)
    } else {
        // Record failure (triggers brute force tracking)
        security.record_auth_failure(username, ip).await;
        Ok(false)
    }
}
```

### Password Validation

```rust
// Validate password strength
let result = security.validate_password("MyStr0ng!Pass#2024");
if result.valid {
    println!("Password is strong (entropy: {:.2} bits)", result.entropy_bits);
} else {
    println!("Password validation failed:");
    for error in result.errors {
        println!("  - {}", error);
    }
}

// Or use check method for Result
security.check_password_strength("weak")?;
```

### Administrative Functions

```rust
// Unlock a user account
security.unlock_user("john@example.com").await;

// Unlock an IP address
security.unlock_ip(&"192.168.1.100".parse()?).await;

// Reset rate limit for IP
security.reset_rate_limit(&"192.168.1.100".parse()?).await;

// View audit log
if let Some(entries) = security.get_audit_log(100).await {
    for entry in entries {
        println!("{:?} - {} at {}", entry.event, entry.username, entry.timestamp);
    }
}

// View audit log for specific user
if let Some(entries) = security.get_user_audit_log("john@example.com").await {
    println!("Found {} events for user", entries.len());
}
```

## Example

See `examples/security_integration.rs` for a complete working example demonstrating all features.

Run the example with:
```bash
cargo run --example security_integration -p rusmes-auth
```

## Security Considerations

1. **In-Memory Storage**: The current implementation uses in-memory storage for audit logs and tracking data. For production use, consider implementing persistent storage.

2. **Distributed Systems**: If running multiple RusMES instances, brute force protection and rate limiting should be coordinated across instances using a shared data store (e.g., Redis).

3. **Password Storage**: The security module validates password strength but doesn't handle password hashing. Use the existing backend implementations (bcrypt, Argon2, SCRAM-SHA-256) for secure password storage.

4. **IP Spoofing**: IP-based protections assume the IP address is trustworthy. In environments with proxies, ensure you're using the real client IP (e.g., from X-Forwarded-For header).

5. **Audit Log Retention**: The in-memory audit log has a configurable size limit. Implement log rotation and archival for long-term retention.

## Thread Safety

All components are thread-safe and designed for concurrent access:
- Uses `Arc<RwLock<>>` for shared state
- All public methods are safe to call from multiple threads
- No data races or deadlocks in normal operation
