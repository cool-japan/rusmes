//! Tests for the `[performance]` configuration section (Item 1).

use rusmes_config::{PerformanceConfig, ServerConfig};

/// Helper TOML that is valid in every field except the part under test.
fn minimal_toml_with(extra: &str) -> String {
    format!(
        r#"
domain = "example.com"
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 25
max_message_size = "50MB"

[storage]
backend = "filesystem"
path = "/var/mail"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"

{extra}
"#
    )
}

#[test]
fn test_performance_config_defaults() {
    // A TOML without a [performance] section should yield PerformanceConfig::default().
    let config: ServerConfig = toml::from_str(&minimal_toml_with("")).unwrap();
    let perf = &config.performance;
    let expected = PerformanceConfig::default();

    assert_eq!(perf.worker_threads, expected.worker_threads);
    assert_eq!(perf.imap_pool_size, expected.imap_pool_size);
    assert_eq!(perf.smtp_pool_size, expected.smtp_pool_size);
    assert_eq!(perf.read_buffer_kb, expected.read_buffer_kb);
    assert_eq!(perf.write_buffer_kb, expected.write_buffer_kb);

    // Confirm the concrete default values match the spec.
    assert_eq!(perf.worker_threads, None);
    assert_eq!(perf.imap_pool_size, 64);
    assert_eq!(perf.smtp_pool_size, 64);
    assert_eq!(perf.read_buffer_kb, 64);
    assert_eq!(perf.write_buffer_kb, 64);
}

#[test]
fn test_performance_config_explicit() {
    // Explicit [performance] section should override all fields.
    let extra = r#"
[performance]
worker_threads = 4
imap_pool_size = 128
smtp_pool_size = 256
read_buffer_kb = 32
write_buffer_kb = 16
"#;
    let config: ServerConfig = toml::from_str(&minimal_toml_with(extra)).unwrap();
    let perf = &config.performance;

    assert_eq!(perf.worker_threads, Some(4));
    assert_eq!(perf.imap_pool_size, 128);
    assert_eq!(perf.smtp_pool_size, 256);
    assert_eq!(perf.read_buffer_kb, 32);
    assert_eq!(perf.write_buffer_kb, 16);
}

#[test]
fn test_performance_config_partial() {
    // Only overriding some fields should leave the rest at defaults.
    let extra = r#"
[performance]
worker_threads = 2
"#;
    let config: ServerConfig = toml::from_str(&minimal_toml_with(extra)).unwrap();
    let perf = &config.performance;

    assert_eq!(perf.worker_threads, Some(2));
    assert_eq!(perf.imap_pool_size, 64); // default
    assert_eq!(perf.smtp_pool_size, 64); // default
    assert_eq!(perf.read_buffer_kb, 64); // default
    assert_eq!(perf.write_buffer_kb, 64); // default
}

#[test]
fn test_performance_effective_worker_threads_none() {
    // When worker_threads is None, effective_worker_threads returns at least 1.
    let perf = PerformanceConfig::default();
    assert!(perf.effective_worker_threads() >= 1);
}

#[test]
fn test_performance_effective_worker_threads_explicit() {
    // When worker_threads is set, effective_worker_threads returns that value.
    let perf = PerformanceConfig {
        worker_threads: Some(8),
        ..PerformanceConfig::default()
    };
    assert_eq!(perf.effective_worker_threads(), 8);
}

#[test]
fn test_performance_validate_zero_pool() {
    // A pool size of 0 should fail validation.
    let perf = PerformanceConfig {
        imap_pool_size: 0,
        ..PerformanceConfig::default()
    };
    assert!(perf.validate().is_err());
}

#[test]
fn test_performance_validate_zero_worker_threads() {
    // An explicit worker_threads = 0 should fail validation.
    let perf = PerformanceConfig {
        worker_threads: Some(0),
        ..PerformanceConfig::default()
    };
    assert!(perf.validate().is_err());
}

#[test]
fn test_performance_validate_ok() {
    // Default config should validate successfully.
    PerformanceConfig::default().validate().unwrap();
}
