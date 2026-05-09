//! Integration tests for basic configuration parsing (TOML and YAML).
//!
//! These tests were extracted from `lib.rs` to keep file sizes under the
//! 2000-line policy limit.

use rusmes_config::{
    AuthConfig, DomainsConfig, LoggingConfig, MetricsConfig, QueueConfig, RateLimitConfig,
    SecurityConfig, ServerConfig,
};

const MINIMAL_TOML: &str = r#"
domain = "example.com"
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 25
tls_port = 587
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
"#;

#[test]
fn test_parse_toml_config() {
    let config: ServerConfig = toml::from_str(MINIMAL_TOML).unwrap();
    assert_eq!(config.domain, "example.com");
    assert_eq!(config.smtp.port, 25);
    assert_eq!(config.processors.len(), 1);
    assert_eq!(config.processors[0].mailets.len(), 1);
}

#[test]
fn test_parse_auth_config() {
    let toml_str = r#"
        backend = "file"
        path = "/etc/rusmes/users.db"
    "#;

    let config: AuthConfig = toml::from_str(toml_str).unwrap();
    match config {
        AuthConfig::File { config } => {
            assert_eq!(config.path, "/etc/rusmes/users.db");
        }
        _ => panic!("Expected File auth backend"),
    }
}

#[test]
fn test_parse_logging_config() {
    let toml_str = r#"
        level = "info"
        format = "json"
        output = "stdout"
    "#;

    let config: LoggingConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.level, "info");
    assert_eq!(config.format, "json");
    assert_eq!(config.output, "stdout");
    config.validate_level().unwrap();
    config.validate_format().unwrap();
}

#[test]
fn test_parse_queue_config() {
    let toml_str = r#"
        initial_delay = "60s"
        max_delay = "3600s"
        backoff_multiplier = 2.0
        max_attempts = 5
        worker_threads = 5
        batch_size = 100
    "#;

    let config: QueueConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.initial_delay_seconds().unwrap(), 60);
    assert_eq!(config.max_delay_seconds().unwrap(), 3600);
    assert_eq!(config.backoff_multiplier, 2.0);
    assert_eq!(config.max_attempts, 5);
    assert_eq!(config.worker_threads, 5);
    assert_eq!(config.batch_size, 100);
    config.validate_backoff_multiplier().unwrap();
    config.validate_worker_threads().unwrap();
}

#[test]
fn test_parse_security_config() {
    let toml_str = r#"
        relay_networks = ["127.0.0.0/8", "10.0.0.0/8"]
        blocked_ips = ["192.0.2.1", "2001:db8::1"]
        check_recipient_exists = true
        reject_unknown_recipients = true
    "#;

    let config: SecurityConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.relay_networks.len(), 2);
    assert_eq!(config.blocked_ips.len(), 2);
    assert!(config.check_recipient_exists);
    assert!(config.reject_unknown_recipients);
    config.validate_relay_networks().unwrap();
    config.validate_blocked_ips().unwrap();
}

#[test]
fn test_parse_domains_config() {
    let toml_str = r#"
        local_domains = ["example.com", "mail.example.com"]

        [aliases]
        "abuse@example.com" = "postmaster@example.com"
        "webmaster@example.com" = "admin@example.com"
    "#;

    let config: DomainsConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.local_domains.len(), 2);
    assert_eq!(config.aliases.len(), 2);
    assert_eq!(
        config.aliases.get("abuse@example.com"),
        Some(&"postmaster@example.com".to_string())
    );
    config.validate_local_domains().unwrap();
    config.validate_aliases().unwrap();
}

#[test]
fn test_parse_metrics_config() {
    let toml_str = r#"
        enabled = true
        bind_address = "0.0.0.0:9090"
        path = "/metrics"
    "#;

    let config: MetricsConfig = toml::from_str(toml_str).unwrap();
    assert!(config.enabled);
    assert_eq!(config.bind_address, "0.0.0.0:9090");
    assert_eq!(config.path, "/metrics");
    config.validate_bind_address().unwrap();
    config.validate_path().unwrap();
}

#[test]
fn test_parse_rate_limit_config() {
    let toml_str = r#"
        max_messages_per_hour = 100
        window_duration = "1h"
    "#;

    let config: RateLimitConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.max_messages_per_hour, 100);
    assert_eq!(config.window_duration_seconds().unwrap(), 3600);
}

#[test]
fn test_parse_full_config_with_all_sections() {
    let toml_str = r#"
        domain = "mail.example.com"
        postmaster = "postmaster@example.com"

        [smtp]
        host = "0.0.0.0"
        port = 25
        tls_port = 587
        max_message_size = "50MB"

        [smtp.rate_limit]
        max_messages_per_hour = 100
        window_duration = "1h"

        [storage]
        backend = "filesystem"
        path = "/var/mail"

        [auth]
        backend = "file"
        path = "/etc/rusmes/users.db"

        [logging]
        level = "info"
        format = "json"
        output = "stdout"

        [queue]
        initial_delay = "60s"
        max_delay = "3600s"
        backoff_multiplier = 2.0
        max_attempts = 5
        worker_threads = 5
        batch_size = 100

        [security]
        relay_networks = ["127.0.0.0/8"]
        blocked_ips = []
        check_recipient_exists = true
        reject_unknown_recipients = true

        [domains]
        local_domains = ["example.com"]

        [domains.aliases]
        "abuse@example.com" = "postmaster@example.com"

        [metrics]
        enabled = true
        bind_address = "0.0.0.0:9090"
        path = "/metrics"

        [[processors]]
        name = "root"
        state = "root"

        [[processors.mailets]]
        matcher = "All"
        mailet = "LocalDelivery"
    "#;

    let config: ServerConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.domain, "mail.example.com");
    assert!(config.auth.is_some());
    assert!(config.logging.is_some());
    assert!(config.queue.is_some());
    assert!(config.security.is_some());
    assert!(config.domains.is_some());
    assert!(config.metrics.is_some());
    assert!(config.smtp.rate_limit.is_some());

    if let Some(logging) = &config.logging {
        logging.validate_level().unwrap();
        logging.validate_format().unwrap();
    }

    if let Some(queue) = &config.queue {
        queue.validate_backoff_multiplier().unwrap();
        queue.validate_worker_threads().unwrap();
    }

    if let Some(security) = &config.security {
        security.validate_relay_networks().unwrap();
        security.validate_blocked_ips().unwrap();
    }

    if let Some(domains) = &config.domains {
        domains.validate_local_domains().unwrap();
        domains.validate_aliases().unwrap();
    }

    if let Some(metrics) = &config.metrics {
        metrics.validate_bind_address().unwrap();
        metrics.validate_path().unwrap();
    }
}

#[test]
fn test_parse_yaml_config() {
    let yaml_str = r#"
domain: example.com
postmaster: postmaster@example.com

smtp:
  host: 0.0.0.0
  port: 25
  tls_port: 587
  max_message_size: 50MB

storage:
  backend: filesystem
  path: /var/mail

processors:
  - name: root
    state: root
    mailets:
      - matcher: All
        mailet: LocalDelivery
        params: {}
    "#;

    let config: ServerConfig = serde_yaml::from_str(yaml_str).unwrap();
    assert_eq!(config.domain, "example.com");
    assert_eq!(config.smtp.port, 25);
    assert_eq!(config.processors.len(), 1);
    assert_eq!(config.processors[0].mailets.len(), 1);
}

#[test]
fn test_yaml_equivalence_to_toml() {
    let yaml_str = r#"
domain: mail.example.com
postmaster: postmaster@example.com

smtp:
  host: 0.0.0.0
  port: 25
  tls_port: 587
  max_message_size: 50MB
  require_auth: true
  enable_starttls: true

storage:
  backend: filesystem
  path: /var/mail

processors:
  - name: root
    state: root
    mailets:
      - matcher: All
        mailet: LocalDelivery

auth:
  backend: file
  path: /etc/rusmes/users.db

logging:
  level: info
  format: json
  output: stdout

domains:
  local_domains:
    - example.com
    - mail.example.com
    "#;

    let toml_str = r#"
domain = "mail.example.com"
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 25
tls_port = 587
max_message_size = "50MB"
require_auth = true
enable_starttls = true

[storage]
backend = "filesystem"
path = "/var/mail"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"

[auth]
backend = "file"
path = "/etc/rusmes/users.db"

[logging]
level = "info"
format = "json"
output = "stdout"

[domains]
local_domains = ["example.com", "mail.example.com"]
    "#;

    let yaml_config: ServerConfig = serde_yaml::from_str(yaml_str).unwrap();
    let toml_config: ServerConfig = toml::from_str(toml_str).unwrap();

    assert_eq!(yaml_config.domain, toml_config.domain);
    assert_eq!(yaml_config.postmaster, toml_config.postmaster);
    assert_eq!(yaml_config.smtp.host, toml_config.smtp.host);
    assert_eq!(yaml_config.smtp.port, toml_config.smtp.port);
    assert_eq!(yaml_config.smtp.tls_port, toml_config.smtp.tls_port);
    assert_eq!(
        yaml_config.smtp.max_message_size,
        toml_config.smtp.max_message_size
    );
    assert_eq!(yaml_config.smtp.require_auth, toml_config.smtp.require_auth);
    assert_eq!(
        yaml_config.smtp.enable_starttls,
        toml_config.smtp.enable_starttls
    );
    assert_eq!(yaml_config.processors.len(), toml_config.processors.len());
    assert!(yaml_config.auth.is_some());
    assert!(toml_config.auth.is_some());

    if let (Some(yaml_log), Some(toml_log)) = (&yaml_config.logging, &toml_config.logging) {
        assert_eq!(yaml_log.level, toml_log.level);
        assert_eq!(yaml_log.format, toml_log.format);
        assert_eq!(yaml_log.output, toml_log.output);
    }

    if let (Some(yaml_domains), Some(toml_domains)) = (&yaml_config.domains, &toml_config.domains) {
        assert_eq!(
            yaml_domains.local_domains.len(),
            toml_domains.local_domains.len()
        );
        assert_eq!(yaml_domains.local_domains, toml_domains.local_domains);
    }
}
