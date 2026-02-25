//! Validate configuration file

use anyhow::{Context, Result};
use rusmes_config::ServerConfig;
use std::path::Path;

/// Check and validate configuration file
pub fn run(config_path: &str) -> Result<()> {
    println!("Checking configuration: {}", config_path);

    // Check if file exists
    let path = Path::new(config_path);
    if !path.exists() {
        anyhow::bail!("Configuration file not found: {}", config_path);
    }

    // Load configuration
    let config = ServerConfig::from_file(config_path).context("Failed to load configuration")?;

    println!("Configuration loaded successfully");
    println!();

    // Validate basic settings
    println!("Basic settings:");
    println!("  Domain: {}", config.domain);
    println!("  Postmaster: {}", config.postmaster);

    // Validate postmaster address
    config
        .postmaster_address()
        .context("Invalid postmaster address")?;
    println!("  Postmaster address: valid");

    // Validate SMTP settings
    println!();
    println!("SMTP server:");
    println!("  Host: {}", config.smtp.host);
    println!("  Port: {}", config.smtp.port);
    println!("  Max message size: {}", config.smtp.max_message_size);

    let max_size = config
        .smtp
        .max_message_size_bytes()
        .context("Invalid max_message_size format")?;
    println!("  Max message size (bytes): {}", max_size);

    if let Some(tls_port) = config.smtp.tls_port {
        println!("  TLS port: {}", tls_port);
    }

    if let Some(rate_limit) = &config.smtp.rate_limit {
        println!(
            "  Rate limit: {} msgs/hour",
            rate_limit.max_messages_per_hour
        );
        let window = rate_limit
            .window_duration_seconds()
            .context("Invalid rate_limit window_duration")?;
        println!("  Rate limit window: {}s", window);
    }

    // Validate IMAP settings
    if let Some(imap) = &config.imap {
        println!();
        println!("IMAP server:");
        println!("  Host: {}", imap.host);
        println!("  Port: {}", imap.port);
        if let Some(tls_port) = imap.tls_port {
            println!("  TLS port: {}", tls_port);
        }
    }

    // Validate JMAP settings
    if let Some(jmap) = &config.jmap {
        println!();
        println!("JMAP server:");
        println!("  Host: {}", jmap.host);
        println!("  Port: {}", jmap.port);
        println!("  Base URL: {}", jmap.base_url);
    }

    // Validate storage settings
    println!();
    println!("Storage:");
    match &config.storage {
        rusmes_config::StorageConfig::Filesystem { path } => {
            println!("  Backend: filesystem");
            println!("  Path: {}", path);
        }
        rusmes_config::StorageConfig::Postgres { connection_string } => {
            println!("  Backend: postgres");
            println!("  Connection: {}", connection_string);
        }
        rusmes_config::StorageConfig::AmateRS {
            endpoints,
            replication_factor,
        } => {
            println!("  Backend: AmateRS");
            println!("  Endpoints: {:?}", endpoints);
            println!("  Replication factor: {}", replication_factor);
        }
    }

    // Validate auth settings
    if let Some(auth) = &config.auth {
        println!();
        println!("Authentication:");
        match auth {
            rusmes_config::AuthConfig::File {
                config: file_config,
            } => {
                println!("  Backend: file");
                println!("  Path: {}", file_config.path);
            }
            rusmes_config::AuthConfig::Ldap {
                config: ldap_config,
            } => {
                println!("  Backend: LDAP");
                println!("  URL: {}", ldap_config.url);
                println!("  Base DN: {}", ldap_config.base_dn);
            }
            rusmes_config::AuthConfig::Sql { config: sql_config } => {
                println!("  Backend: SQL");
                println!("  Connection: {}", sql_config.connection_string);
            }
            rusmes_config::AuthConfig::OAuth2 {
                config: oauth_config,
            } => {
                println!("  Backend: OAuth2");
                println!("  Token URL: {}", oauth_config.token_url);
            }
        }
    }

    // Validate logging settings
    if let Some(logging) = &config.logging {
        println!();
        println!("Logging:");
        println!("  Level: {}", logging.level);
        println!("  Format: {}", logging.format);
        println!("  Output: {}", logging.output);

        logging.validate_level().context("Invalid log level")?;
        logging.validate_format().context("Invalid log format")?;

        if let Some(file) = &logging.file {
            println!("  File path: {}", file.path);
            println!("  Max size: {}", file.max_size);
            println!("  Max backups: {}", file.max_backups);
            println!("  Compress: {}", file.compress);

            let max_size = file
                .max_size_bytes()
                .context("Invalid log file max_size format")?;
            println!("  Max size (bytes): {}", max_size);
        }
    }

    // Validate queue settings
    if let Some(queue) = &config.queue {
        println!();
        println!("Queue:");
        println!("  Initial delay: {}", queue.initial_delay);
        println!("  Max delay: {}", queue.max_delay);
        println!("  Backoff multiplier: {}", queue.backoff_multiplier);
        println!("  Max attempts: {}", queue.max_attempts);
        println!("  Worker threads: {}", queue.worker_threads);
        println!("  Batch size: {}", queue.batch_size);

        let initial_delay = queue
            .initial_delay_seconds()
            .context("Invalid queue initial_delay format")?;
        let max_delay = queue
            .max_delay_seconds()
            .context("Invalid queue max_delay format")?;
        println!("  Initial delay (seconds): {}", initial_delay);
        println!("  Max delay (seconds): {}", max_delay);

        queue
            .validate_backoff_multiplier()
            .context("Invalid backoff_multiplier")?;
        queue
            .validate_worker_threads()
            .context("Invalid worker_threads")?;
    }

    // Validate security settings
    if let Some(security) = &config.security {
        println!();
        println!("Security:");
        println!("  Relay networks: {:?}", security.relay_networks);
        println!("  Blocked IPs: {:?}", security.blocked_ips);
        println!(
            "  Check recipient exists: {}",
            security.check_recipient_exists
        );
        println!(
            "  Reject unknown recipients: {}",
            security.reject_unknown_recipients
        );

        security
            .validate_relay_networks()
            .context("Invalid relay_networks")?;
        security
            .validate_blocked_ips()
            .context("Invalid blocked_ips")?;
    }

    // Validate domains settings
    if let Some(domains) = &config.domains {
        println!();
        println!("Domains:");
        println!("  Local domains: {:?}", domains.local_domains);

        domains
            .validate_local_domains()
            .context("Invalid local_domains")?;

        if !domains.aliases.is_empty() {
            println!("  Aliases: {} configured", domains.aliases.len());
            domains.validate_aliases().context("Invalid aliases")?;
        }
    }

    // Validate metrics settings
    if let Some(metrics) = &config.metrics {
        println!();
        println!("Metrics:");
        println!("  Enabled: {}", metrics.enabled);
        println!("  Bind address: {}", metrics.bind_address);
        println!("  Path: {}", metrics.path);

        metrics
            .validate_bind_address()
            .context("Invalid metrics bind_address")?;
        metrics.validate_path().context("Invalid metrics path")?;
    }

    // Validate processors
    println!();
    println!("Processors:");
    println!("  Total processors: {}", config.processors.len());
    for processor in &config.processors {
        println!("  - {} (state: {})", processor.name, processor.state);
        println!("    Mailets: {}", processor.mailets.len());
    }

    // All checks passed
    println!();
    println!("Configuration is valid!");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::Builder;

    #[test]
    fn test_valid_minimal_config() {
        let config = r#"
domain = "example.com"
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 25
max_message_size = "50MB"

[storage]
backend = "filesystem"
path = "./data/mailboxes"

[[processors]]
name = "root"
state = "root"
mailets = []
"#;

        let mut file = Builder::new().suffix(".toml").tempfile().unwrap();
        file.write_all(config.as_bytes()).unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let result = run(&path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_config_file() {
        let result = run("/nonexistent/path/to/config.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_config_with_all_backends() {
        let config = r#"
domain = "example.com"
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 25
max_message_size = "50MB"

[storage]
backend = "postgres"
connection_string = "postgresql://localhost/rusmes"

[auth]
backend = "file"
path = "./data/users.db"

[[processors]]
name = "root"
state = "root"
mailets = []
"#;

        let mut file = Builder::new().suffix(".toml").tempfile().unwrap();
        file.write_all(config.as_bytes()).unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let result = run(&path);
        assert!(result.is_ok());
    }
}
