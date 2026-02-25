//! Example demonstrating how to integrate the security module with AuthBackend
//!
//! This example shows:
//! - Creating an AuthSecurity instance with custom configuration
//! - Wrapping an existing AuthBackend with security features
//! - Handling authentication with brute force protection
//! - Password strength validation
//! - Rate limiting
//! - Audit logging

use rusmes_auth::file::FileAuthBackend;
use rusmes_auth::security::{
    AuthSecurity, AuthSecurityBuilder, BruteForceConfig, PasswordStrengthConfig, RateLimitConfig,
    SecurityConfig,
};
use rusmes_auth::AuthBackend;
use rusmes_proto::Username;
use std::net::IpAddr;
use std::str::FromStr;

/// Secure wrapper around any AuthBackend
struct SecureAuthBackend<T: AuthBackend> {
    backend: T,
    security: AuthSecurity,
}

impl<T: AuthBackend> SecureAuthBackend<T> {
    /// Create a new secure auth backend
    fn new(backend: T, security: AuthSecurity) -> Self {
        Self { backend, security }
    }

    /// Authenticate with security checks
    async fn authenticate_secure(
        &self,
        username: &Username,
        password: &str,
        ip: Option<IpAddr>,
    ) -> anyhow::Result<bool> {
        let username_str = username.to_string();

        // Check if authentication attempt is allowed (rate limiting, lockout, etc.)
        self.security.check_auth_attempt(&username_str, ip).await?;

        // Perform actual authentication
        let result = self.backend.authenticate(username, password).await?;

        if result {
            // Record successful authentication
            self.security.record_auth_success(&username_str, ip).await;
            Ok(true)
        } else {
            // Record failed authentication
            self.security.record_auth_failure(&username_str, ip).await;
            Ok(false)
        }
    }

    /// Create user with password strength validation
    async fn create_user_secure(
        &self,
        username: &Username,
        password: &str,
        ip: Option<IpAddr>,
    ) -> anyhow::Result<()> {
        // Validate password strength
        self.security.check_password_strength(password)?;

        // Create the user
        self.backend.create_user(username, password).await?;

        // Log the event
        self.security
            .log_user_created(&username.to_string(), ip)
            .await;

        Ok(())
    }

    /// Change password with strength validation
    #[allow(dead_code)]
    async fn change_password_secure(
        &self,
        username: &Username,
        new_password: &str,
        ip: Option<IpAddr>,
    ) -> anyhow::Result<()> {
        // Validate password strength
        self.security.check_password_strength(new_password)?;

        // Change the password
        self.backend.change_password(username, new_password).await?;

        // Log the event
        self.security
            .log_password_change(&username.to_string(), ip)
            .await;

        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== RusMES Authentication Security Integration Example ===\n");

    // 1. Create custom security configuration
    println!("1. Creating security configuration...");

    let brute_force_config = BruteForceConfig {
        max_attempts: 3,
        lockout_duration_secs: 300, // 5 minutes
        attempt_window_secs: 60,    // 1 minute
        progressive_lockout: true,
    };

    let password_config = PasswordStrengthConfig {
        min_length: 10,
        require_uppercase: true,
        require_lowercase: true,
        require_digit: true,
        require_special: true,
        min_entropy_bits: 3.5,
        banned_passwords: vec![
            "password".to_string(),
            "admin".to_string(),
            "welcome".to_string(),
        ],
    };

    let rate_limit_config = RateLimitConfig {
        max_requests: 5,
        window_secs: 60,
    };

    let security_config = SecurityConfig {
        brute_force: brute_force_config,
        password_strength: password_config,
        rate_limit: rate_limit_config,
        enable_audit_log: true,
    };

    println!(
        "  ✓ Security config: {} max attempts, {} sec lockout",
        security_config.brute_force.max_attempts, security_config.brute_force.lockout_duration_secs
    );

    // 2. Create AuthSecurity instance (using builder pattern)
    println!("\n2. Creating AuthSecurity instance...");
    let auth_security = AuthSecurityBuilder::new()
        .brute_force_config(security_config.brute_force.clone())
        .password_strength_config(security_config.password_strength.clone())
        .rate_limit_config(security_config.rate_limit.clone())
        .enable_audit_log(true)
        .build();

    println!("  ✓ AuthSecurity created with audit logging enabled");

    // 3. Create a file-based auth backend
    println!("\n3. Creating file-based authentication backend...");
    let backend = FileAuthBackend::new("/tmp/rusmes_auth_security_example.txt").await?;
    println!("  ✓ FileAuthBackend created");

    // 4. Wrap backend with security
    println!("\n4. Creating secure auth backend wrapper...");
    let secure_backend = SecureAuthBackend::new(backend, auth_security);
    println!("  ✓ Secure wrapper created");

    // 5. Demonstrate password strength validation
    println!("\n5. Testing password strength validation...");

    let weak_passwords = vec![
        ("weak", "Too short and simple"),
        ("password123", "Contains banned word 'password'"),
        ("NoSpecial1", "Missing special character"),
    ];

    for (password, reason) in weak_passwords {
        let result = secure_backend.security.validate_password(password);
        println!("  × Password '{}': {}", password, reason);
        if !result.valid {
            println!("    Errors: {}", result.errors.join("; "));
        }
        println!("    Entropy: {:.2} bits", result.entropy_bits);
    }

    let strong_password = "MyStr0ng!Pass#2024";
    let result = secure_backend.security.validate_password(strong_password);
    println!(
        "  ✓ Password '{}': {}",
        strong_password,
        if result.valid { "VALID" } else { "INVALID" }
    );
    println!("    Entropy: {:.2} bits", result.entropy_bits);

    // 6. Create a user with password validation
    println!("\n6. Creating user with strong password...");
    let username = Username::from_str("testuser@example.com")?;
    let ip = Some(IpAddr::from_str("192.168.1.100")?);

    secure_backend
        .create_user_secure(&username, strong_password, ip)
        .await?;
    println!("  ✓ User 'testuser@example.com' created successfully");

    // 7. Demonstrate successful authentication
    println!("\n7. Testing successful authentication...");
    let auth_result = secure_backend
        .authenticate_secure(&username, strong_password, ip)
        .await?;
    println!("  ✓ Authentication result: {}", auth_result);

    // 8. Demonstrate failed authentication and brute force protection
    println!("\n8. Testing brute force protection...");
    let wrong_password = "WrongPass123!";

    for attempt in 1..=5 {
        println!("  Attempt #{}: Wrong password", attempt);
        match secure_backend
            .authenticate_secure(&username, wrong_password, ip)
            .await
        {
            Ok(result) => {
                println!("    Authentication result: {}", result);
                if !result {
                    println!("    Failed attempt recorded");
                }
            }
            Err(e) => {
                println!("    ✓ Blocked: {}", e);
                break;
            }
        }
    }

    // 9. Show lockout status
    println!("\n9. Checking lockout status...");
    if let Some(remaining) = secure_backend
        .security
        .brute_force()
        .get_unlock_time(&username.to_string())
        .await
    {
        println!("  ✓ Account locked for {} seconds", remaining);
    } else {
        println!("  Account not locked");
    }

    // 10. Demonstrate rate limiting
    println!("\n10. Testing rate limiting...");
    let another_user = Username::from_str("another@example.com")?;
    let another_ip = IpAddr::from_str("192.168.1.101")?;

    secure_backend
        .create_user_secure(&another_user, strong_password, Some(another_ip))
        .await?;

    for attempt in 1..=10 {
        match secure_backend
            .security
            .check_auth_attempt(&another_user.to_string(), Some(another_ip))
            .await
        {
            Ok(()) => {
                println!("  Attempt #{}: Allowed", attempt);
            }
            Err(e) => {
                println!("  Attempt #{}: ✓ Rate limited: {}", attempt, e);
                break;
            }
        }
    }

    // 11. View audit log
    println!("\n11. Viewing audit log...");
    if let Some(entries) = secure_backend.security.get_audit_log(20).await {
        println!("  Recent audit log entries ({} total):", entries.len());
        for (i, entry) in entries.iter().rev().take(10).enumerate() {
            println!(
                "    [{}] {:?} - {} (IP: {:?})",
                i + 1,
                entry.event,
                entry.username,
                entry.ip_address
            );
        }
    }

    // 12. Administrative functions
    println!("\n12. Testing administrative functions...");
    secure_backend
        .security
        .unlock_user(&username.to_string())
        .await;
    println!("  ✓ User 'testuser@example.com' manually unlocked");

    if let Some(ip_addr) = ip {
        secure_backend.security.unlock_ip(&ip_addr).await;
        println!("  ✓ IP {} manually unlocked", ip_addr);
        secure_backend.security.reset_rate_limit(&ip_addr).await;
        println!("  ✓ Rate limit reset for IP {}", ip_addr);
    }

    // 13. Verify unlocked user can authenticate
    println!("\n13. Verifying unlocked user can authenticate...");
    match secure_backend
        .authenticate_secure(&username, strong_password, ip)
        .await
    {
        Ok(result) => {
            println!("  ✓ Authentication successful: {}", result);
        }
        Err(e) => {
            println!("  × Authentication failed: {}", e);
        }
    }

    println!("\n=== Example completed successfully! ===");
    Ok(())
}
