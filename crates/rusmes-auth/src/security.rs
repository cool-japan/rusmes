//! Authentication security module
//!
//! Provides comprehensive security features for authentication:
//! - Brute force protection with account lockout
//! - Password strength validation (length, complexity, entropy)
//! - Audit logging for authentication events
//! - Rate limiting on authentication attempts per IP
//! - Configurable lockout policies

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

// ============================================================================
// Configuration Structures
// ============================================================================

/// Configuration for brute force protection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BruteForceConfig {
    /// Maximum failed attempts before lockout
    pub max_attempts: u32,
    /// Lockout duration in seconds
    pub lockout_duration_secs: u64,
    /// Time window for counting failed attempts (in seconds)
    pub attempt_window_secs: u64,
    /// Whether to enable progressive lockout (increasing duration)
    pub progressive_lockout: bool,
}

impl Default for BruteForceConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            lockout_duration_secs: 900, // 15 minutes
            attempt_window_secs: 300,   // 5 minutes
            progressive_lockout: true,
        }
    }
}

/// Configuration for password strength validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordStrengthConfig {
    /// Minimum password length
    pub min_length: usize,
    /// Require at least one uppercase letter
    pub require_uppercase: bool,
    /// Require at least one lowercase letter
    pub require_lowercase: bool,
    /// Require at least one digit
    pub require_digit: bool,
    /// Require at least one special character
    pub require_special: bool,
    /// Minimum entropy bits (Shannon entropy)
    pub min_entropy_bits: f64,
    /// List of common/banned passwords
    pub banned_passwords: Vec<String>,
}

impl Default for PasswordStrengthConfig {
    fn default() -> Self {
        Self {
            min_length: 8,
            require_uppercase: true,
            require_lowercase: true,
            require_digit: true,
            require_special: true,
            min_entropy_bits: 3.0,
            banned_passwords: vec![
                "password".to_string(),
                "123456".to_string(),
                "qwerty".to_string(),
                "admin".to_string(),
                "letmein".to_string(),
            ],
        }
    }
}

/// Configuration for rate limiting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum requests per time window
    pub max_requests: u32,
    /// Time window in seconds
    pub window_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 10,
            window_secs: 60, // 1 minute
        }
    }
}

/// Overall security configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Brute force protection settings
    pub brute_force: BruteForceConfig,
    /// Password strength validation settings
    pub password_strength: PasswordStrengthConfig,
    /// Rate limiting settings
    pub rate_limit: RateLimitConfig,
    /// Enable audit logging
    pub enable_audit_log: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            brute_force: BruteForceConfig::default(),
            password_strength: PasswordStrengthConfig::default(),
            rate_limit: RateLimitConfig::default(),
            enable_audit_log: true,
        }
    }
}

// ============================================================================
// Audit Logging
// ============================================================================

/// Authentication event type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthEvent {
    /// Successful authentication
    Success,
    /// Failed authentication (wrong password)
    Failure,
    /// Account locked due to brute force
    AccountLocked,
    /// Rate limit exceeded
    RateLimitExceeded,
    /// Password changed
    PasswordChanged,
    /// User created
    UserCreated,
    /// User deleted
    UserDeleted,
}

/// Authentication audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    /// Timestamp (Unix epoch seconds)
    pub timestamp: u64,
    /// Event type
    pub event: AuthEvent,
    /// Username involved
    pub username: String,
    /// Source IP address
    pub ip_address: Option<IpAddr>,
    /// Additional details
    pub details: Option<String>,
}

/// Audit logger for authentication events
pub struct AuditLogger {
    /// In-memory log entries (for demonstration; production would use external storage)
    entries: Arc<RwLock<Vec<AuditLogEntry>>>,
    /// Maximum entries to keep in memory
    max_entries: usize,
}

impl AuditLogger {
    /// Create a new audit logger
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(Vec::new())),
            max_entries,
        }
    }

    /// Log an authentication event
    pub async fn log(
        &self,
        event: AuthEvent,
        username: String,
        ip_address: Option<IpAddr>,
        details: Option<String>,
    ) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let entry = AuditLogEntry {
            timestamp,
            event,
            username,
            ip_address,
            details,
        };

        let mut entries = self.entries.write().await;
        entries.push(entry);

        // Keep only the most recent entries
        if entries.len() > self.max_entries {
            let start = entries.len() - self.max_entries;
            *entries = entries[start..].to_vec();
        }
    }

    /// Get recent audit log entries
    pub async fn get_recent(&self, count: usize) -> Vec<AuditLogEntry> {
        let entries = self.entries.read().await;
        let start = entries.len().saturating_sub(count);
        entries[start..].to_vec()
    }

    /// Get audit log entries for a specific username
    pub async fn get_for_user(&self, username: &str) -> Vec<AuditLogEntry> {
        let entries = self.entries.read().await;
        entries
            .iter()
            .filter(|e| e.username == username)
            .cloned()
            .collect()
    }
}

// ============================================================================
// Brute Force Protection
// ============================================================================

/// Failed authentication attempt record
#[derive(Debug, Clone)]
struct FailedAttempt {
    /// Timestamp of the attempt
    timestamp: u64,
}

/// Account lockout information
#[derive(Debug, Clone)]
struct LockoutInfo {
    /// When the account was locked
    locked_at: u64,
    /// Duration of the lockout in seconds
    duration_secs: u64,
    /// Number of times this account has been locked
    lockout_count: u32,
}

/// Brute force protector with account lockout
pub struct BruteForceProtector {
    /// Configuration
    config: BruteForceConfig,
    /// Failed attempts by username
    user_attempts: Arc<RwLock<HashMap<String, Vec<FailedAttempt>>>>,
    /// Failed attempts by IP address
    ip_attempts: Arc<RwLock<HashMap<IpAddr, Vec<FailedAttempt>>>>,
    /// Locked accounts
    locked_accounts: Arc<RwLock<HashMap<String, LockoutInfo>>>,
    /// Locked IP addresses
    locked_ips: Arc<RwLock<HashMap<IpAddr, LockoutInfo>>>,
}

impl BruteForceProtector {
    /// Create a new brute force protector
    pub fn new(config: BruteForceConfig) -> Self {
        Self {
            config,
            user_attempts: Arc::new(RwLock::new(HashMap::new())),
            ip_attempts: Arc::new(RwLock::new(HashMap::new())),
            locked_accounts: Arc::new(RwLock::new(HashMap::new())),
            locked_ips: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get current timestamp
    fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Check if a username is currently locked
    pub async fn is_user_locked(&self, username: &str) -> bool {
        let locked = self.locked_accounts.read().await;
        if let Some(lockout) = locked.get(username) {
            let now = Self::current_timestamp();
            let unlock_time = lockout.locked_at + lockout.duration_secs;
            if now < unlock_time {
                return true;
            }
        }
        false
    }

    /// Check if an IP address is currently locked
    pub async fn is_ip_locked(&self, ip: &IpAddr) -> bool {
        let locked = self.locked_ips.read().await;
        if let Some(lockout) = locked.get(ip) {
            let now = Self::current_timestamp();
            let unlock_time = lockout.locked_at + lockout.duration_secs;
            if now < unlock_time {
                return true;
            }
        }
        false
    }

    /// Record a failed authentication attempt
    pub async fn record_failed_attempt(&self, username: &str, ip: Option<IpAddr>) {
        let now = Self::current_timestamp();
        let cutoff = now - self.config.attempt_window_secs;

        // Record failed attempt for username
        {
            let mut attempts = self.user_attempts.write().await;
            let user_attempts = attempts
                .entry(username.to_string())
                .or_insert_with(Vec::new);

            // Remove old attempts outside the window
            user_attempts.retain(|a| a.timestamp > cutoff);

            // Add new attempt
            user_attempts.push(FailedAttempt { timestamp: now });

            // Check if we should lock the account
            if user_attempts.len() as u32 >= self.config.max_attempts {
                drop(attempts); // Release the lock before acquiring another
                self.lock_user(username).await;
            }
        }

        // Record failed attempt for IP address
        if let Some(ip_addr) = ip {
            let mut attempts = self.ip_attempts.write().await;
            let ip_attempts = attempts.entry(ip_addr).or_insert_with(Vec::new);

            // Remove old attempts outside the window
            ip_attempts.retain(|a| a.timestamp > cutoff);

            // Add new attempt
            ip_attempts.push(FailedAttempt { timestamp: now });

            // Check if we should lock the IP
            if ip_attempts.len() as u32 >= self.config.max_attempts {
                drop(attempts); // Release the lock before acquiring another
                self.lock_ip(&ip_addr).await;
            }
        }
    }

    /// Lock a user account
    async fn lock_user(&self, username: &str) {
        let now = Self::current_timestamp();
        let mut locked = self.locked_accounts.write().await;

        let lockout_count = locked
            .get(username)
            .map(|l| l.lockout_count + 1)
            .unwrap_or(1);

        let duration_secs = if self.config.progressive_lockout {
            // Progressive lockout: double the duration each time
            self.config.lockout_duration_secs * (2_u64.pow(lockout_count.saturating_sub(1)))
        } else {
            self.config.lockout_duration_secs
        };

        locked.insert(
            username.to_string(),
            LockoutInfo {
                locked_at: now,
                duration_secs,
                lockout_count,
            },
        );

        // Clear failed attempts for this user
        let mut attempts = self.user_attempts.write().await;
        attempts.remove(username);
    }

    /// Lock an IP address
    async fn lock_ip(&self, ip: &IpAddr) {
        let now = Self::current_timestamp();
        let mut locked = self.locked_ips.write().await;

        let lockout_count = locked.get(ip).map(|l| l.lockout_count + 1).unwrap_or(1);

        let duration_secs = if self.config.progressive_lockout {
            // Progressive lockout: double the duration each time
            self.config.lockout_duration_secs * (2_u64.pow(lockout_count.saturating_sub(1)))
        } else {
            self.config.lockout_duration_secs
        };

        locked.insert(
            *ip,
            LockoutInfo {
                locked_at: now,
                duration_secs,
                lockout_count,
            },
        );

        // Clear failed attempts for this IP
        let mut attempts = self.ip_attempts.write().await;
        attempts.remove(ip);
    }

    /// Clear failed attempts for a user (e.g., after successful login)
    pub async fn clear_user_attempts(&self, username: &str) {
        let mut attempts = self.user_attempts.write().await;
        attempts.remove(username);
    }

    /// Clear failed attempts for an IP address
    pub async fn clear_ip_attempts(&self, ip: &IpAddr) {
        let mut attempts = self.ip_attempts.write().await;
        attempts.remove(ip);
    }

    /// Manually unlock a user account (admin function)
    pub async fn unlock_user(&self, username: &str) {
        let mut locked = self.locked_accounts.write().await;
        locked.remove(username);

        let mut attempts = self.user_attempts.write().await;
        attempts.remove(username);
    }

    /// Manually unlock an IP address (admin function)
    pub async fn unlock_ip(&self, ip: &IpAddr) {
        let mut locked = self.locked_ips.write().await;
        locked.remove(ip);

        let mut attempts = self.ip_attempts.write().await;
        attempts.remove(ip);
    }

    /// Get time remaining until account unlock (in seconds)
    pub async fn get_unlock_time(&self, username: &str) -> Option<u64> {
        let locked = self.locked_accounts.read().await;
        if let Some(lockout) = locked.get(username) {
            let now = Self::current_timestamp();
            let unlock_time = lockout.locked_at + lockout.duration_secs;
            if now < unlock_time {
                return Some(unlock_time - now);
            }
        }
        None
    }
}

// ============================================================================
// Password Strength Validation
// ============================================================================

/// Password strength validation result
#[derive(Debug, Clone)]
pub struct PasswordStrengthResult {
    /// Whether the password is valid
    pub valid: bool,
    /// List of validation errors
    pub errors: Vec<String>,
    /// Calculated entropy in bits
    pub entropy_bits: f64,
}

/// Password strength validator
pub struct PasswordStrengthValidator {
    /// Configuration
    config: PasswordStrengthConfig,
}

impl PasswordStrengthValidator {
    /// Create a new password strength validator
    pub fn new(config: PasswordStrengthConfig) -> Self {
        Self { config }
    }

    /// Validate password strength
    pub fn validate(&self, password: &str) -> PasswordStrengthResult {
        let mut errors = Vec::new();

        // Check minimum length
        if password.len() < self.config.min_length {
            errors.push(format!(
                "Password must be at least {} characters long",
                self.config.min_length
            ));
        }

        // Check uppercase requirement
        if self.config.require_uppercase && !password.chars().any(|c| c.is_uppercase()) {
            errors.push("Password must contain at least one uppercase letter".to_string());
        }

        // Check lowercase requirement
        if self.config.require_lowercase && !password.chars().any(|c| c.is_lowercase()) {
            errors.push("Password must contain at least one lowercase letter".to_string());
        }

        // Check digit requirement
        if self.config.require_digit && !password.chars().any(|c| c.is_ascii_digit()) {
            errors.push("Password must contain at least one digit".to_string());
        }

        // Check special character requirement
        if self.config.require_special && !password.chars().any(|c| !c.is_alphanumeric()) {
            errors.push("Password must contain at least one special character".to_string());
        }

        // Check banned passwords
        let password_lower = password.to_lowercase();
        for banned in &self.config.banned_passwords {
            if password_lower.contains(&banned.to_lowercase()) {
                errors.push("Password contains a commonly used word or pattern".to_string());
                break;
            }
        }

        // Calculate Shannon entropy
        let entropy = self.calculate_entropy(password);
        if entropy < self.config.min_entropy_bits {
            errors.push(format!(
                "Password entropy too low ({:.2} bits, minimum {:.2} bits required)",
                entropy, self.config.min_entropy_bits
            ));
        }

        PasswordStrengthResult {
            valid: errors.is_empty(),
            errors,
            entropy_bits: entropy,
        }
    }

    /// Calculate Shannon entropy of a password
    fn calculate_entropy(&self, password: &str) -> f64 {
        if password.is_empty() {
            return 0.0;
        }

        let mut char_counts: HashMap<char, usize> = HashMap::new();
        for c in password.chars() {
            *char_counts.entry(c).or_insert(0) += 1;
        }

        let len = password.len() as f64;
        let mut entropy = 0.0;

        for count in char_counts.values() {
            let probability = *count as f64 / len;
            entropy -= probability * probability.log2();
        }

        entropy * len
    }
}

// ============================================================================
// Rate Limiting
// ============================================================================

/// Rate limiter for authentication attempts
pub struct RateLimiter {
    /// Configuration
    config: RateLimitConfig,
    /// Request counts by IP address
    request_counts: Arc<RwLock<HashMap<IpAddr, Vec<u64>>>>,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            request_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if a request from an IP address should be allowed
    pub async fn check_rate_limit(&self, ip: &IpAddr) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let cutoff = now - self.config.window_secs;

        let mut counts = self.request_counts.write().await;
        let ip_counts = counts.entry(*ip).or_insert_with(Vec::new);

        // Remove old requests outside the window
        ip_counts.retain(|&timestamp| timestamp > cutoff);

        // Check if we're over the limit
        if ip_counts.len() >= self.config.max_requests as usize {
            return false;
        }

        // Record this request
        ip_counts.push(now);
        true
    }

    /// Reset rate limit for an IP address (admin function)
    pub async fn reset_limit(&self, ip: &IpAddr) {
        let mut counts = self.request_counts.write().await;
        counts.remove(ip);
    }
}

// ============================================================================
// Main Security Manager
// ============================================================================

/// Main authentication security manager
pub struct AuthSecurity {
    /// Security configuration
    config: SecurityConfig,
    /// Brute force protector
    brute_force: BruteForceProtector,
    /// Password strength validator
    password_validator: PasswordStrengthValidator,
    /// Rate limiter
    rate_limiter: RateLimiter,
    /// Audit logger
    audit_logger: Option<AuditLogger>,
}

impl AuthSecurity {
    /// Create a new authentication security manager
    pub fn new(config: SecurityConfig) -> Self {
        let audit_logger = if config.enable_audit_log {
            Some(AuditLogger::new(10000)) // Keep last 10k entries
        } else {
            None
        };

        Self {
            brute_force: BruteForceProtector::new(config.brute_force.clone()),
            password_validator: PasswordStrengthValidator::new(config.password_strength.clone()),
            rate_limiter: RateLimiter::new(config.rate_limit.clone()),
            audit_logger,
            config,
        }
    }

    /// Get the brute force protector
    pub fn brute_force(&self) -> &BruteForceProtector {
        &self.brute_force
    }

    /// Check if authentication attempt should be allowed
    ///
    /// Returns Ok(()) if allowed, Err with reason if not
    pub async fn check_auth_attempt(&self, username: &str, ip: Option<IpAddr>) -> Result<()> {
        // Check rate limit
        if let Some(ip_addr) = ip {
            if !self.rate_limiter.check_rate_limit(&ip_addr).await {
                if let Some(logger) = &self.audit_logger {
                    logger
                        .log(
                            AuthEvent::RateLimitExceeded,
                            username.to_string(),
                            Some(ip_addr),
                            None,
                        )
                        .await;
                }
                return Err(anyhow!("Rate limit exceeded for IP address"));
            }

            // Check IP lockout
            if self.brute_force.is_ip_locked(&ip_addr).await {
                if let Some(logger) = &self.audit_logger {
                    logger
                        .log(
                            AuthEvent::AccountLocked,
                            username.to_string(),
                            Some(ip_addr),
                            Some("IP address locked".to_string()),
                        )
                        .await;
                }
                return Err(anyhow!("IP address is temporarily locked"));
            }
        }

        // Check user account lockout
        if self.brute_force.is_user_locked(username).await {
            if let Some(remaining) = self.brute_force.get_unlock_time(username).await {
                if let Some(logger) = &self.audit_logger {
                    logger
                        .log(
                            AuthEvent::AccountLocked,
                            username.to_string(),
                            ip,
                            Some(format!("Account locked for {} seconds", remaining)),
                        )
                        .await;
                }
                return Err(anyhow!(
                    "Account is temporarily locked. Try again in {} seconds",
                    remaining
                ));
            }
        }

        Ok(())
    }

    /// Record successful authentication
    pub async fn record_auth_success(&self, username: &str, ip: Option<IpAddr>) {
        // Clear failed attempts
        self.brute_force.clear_user_attempts(username).await;
        if let Some(ip_addr) = ip {
            self.brute_force.clear_ip_attempts(&ip_addr).await;
        }

        // Log success
        if let Some(logger) = &self.audit_logger {
            logger
                .log(AuthEvent::Success, username.to_string(), ip, None)
                .await;
        }
    }

    /// Record failed authentication
    pub async fn record_auth_failure(&self, username: &str, ip: Option<IpAddr>) {
        // Record failed attempt
        self.brute_force.record_failed_attempt(username, ip).await;

        // Log failure
        if let Some(logger) = &self.audit_logger {
            logger
                .log(AuthEvent::Failure, username.to_string(), ip, None)
                .await;
        }
    }

    /// Validate password strength
    pub fn validate_password(&self, password: &str) -> PasswordStrengthResult {
        self.password_validator.validate(password)
    }

    /// Validate password strength and return Result
    pub fn check_password_strength(&self, password: &str) -> Result<()> {
        let result = self.password_validator.validate(password);
        if result.valid {
            Ok(())
        } else {
            Err(anyhow!(
                "Password strength validation failed: {}",
                result.errors.join(", ")
            ))
        }
    }

    /// Log a password change event
    pub async fn log_password_change(&self, username: &str, ip: Option<IpAddr>) {
        if let Some(logger) = &self.audit_logger {
            logger
                .log(AuthEvent::PasswordChanged, username.to_string(), ip, None)
                .await;
        }
    }

    /// Log a user creation event
    pub async fn log_user_created(&self, username: &str, ip: Option<IpAddr>) {
        if let Some(logger) = &self.audit_logger {
            logger
                .log(AuthEvent::UserCreated, username.to_string(), ip, None)
                .await;
        }
    }

    /// Log a user deletion event
    pub async fn log_user_deleted(&self, username: &str, ip: Option<IpAddr>) {
        if let Some(logger) = &self.audit_logger {
            logger
                .log(AuthEvent::UserDeleted, username.to_string(), ip, None)
                .await;
        }
    }

    /// Get audit log entries (admin function)
    pub async fn get_audit_log(&self, count: usize) -> Option<Vec<AuditLogEntry>> {
        if let Some(logger) = &self.audit_logger {
            Some(logger.get_recent(count).await)
        } else {
            None
        }
    }

    /// Get audit log for specific user (admin function)
    pub async fn get_user_audit_log(&self, username: &str) -> Option<Vec<AuditLogEntry>> {
        if let Some(logger) = &self.audit_logger {
            Some(logger.get_for_user(username).await)
        } else {
            None
        }
    }

    /// Manually unlock a user account (admin function)
    pub async fn unlock_user(&self, username: &str) {
        self.brute_force.unlock_user(username).await;
    }

    /// Manually unlock an IP address (admin function)
    pub async fn unlock_ip(&self, ip: &IpAddr) {
        self.brute_force.unlock_ip(ip).await;
    }

    /// Reset rate limit for an IP address (admin function)
    pub async fn reset_rate_limit(&self, ip: &IpAddr) {
        self.rate_limiter.reset_limit(ip).await;
    }

    /// Get the security configuration
    pub fn config(&self) -> &SecurityConfig {
        &self.config
    }
}

// ============================================================================
// Builder Pattern for AuthSecurity
// ============================================================================

/// Builder for AuthSecurity with fluent API
pub struct AuthSecurityBuilder {
    config: SecurityConfig,
}

impl AuthSecurityBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: SecurityConfig::default(),
        }
    }

    /// Set brute force configuration
    pub fn brute_force_config(mut self, config: BruteForceConfig) -> Self {
        self.config.brute_force = config;
        self
    }

    /// Set password strength configuration
    pub fn password_strength_config(mut self, config: PasswordStrengthConfig) -> Self {
        self.config.password_strength = config;
        self
    }

    /// Set rate limit configuration
    pub fn rate_limit_config(mut self, config: RateLimitConfig) -> Self {
        self.config.rate_limit = config;
        self
    }

    /// Enable or disable audit logging
    pub fn enable_audit_log(mut self, enable: bool) -> Self {
        self.config.enable_audit_log = enable;
        self
    }

    /// Build the AuthSecurity instance
    pub fn build(self) -> AuthSecurity {
        AuthSecurity::new(self.config)
    }
}

impl Default for AuthSecurityBuilder {
    fn default() -> Self {
        Self::new()
    }
}
