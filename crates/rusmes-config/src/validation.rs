//! Configuration validation module

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;

/// Validate domain name format
///
/// Validates that the domain:
/// - Is not empty
/// - Contains only valid characters (alphanumeric, hyphens, dots)
/// - Has valid structure (no consecutive dots, doesn't start/end with hyphen)
/// - Has a valid TLD
pub fn validate_domain(domain: &str) -> Result<()> {
    if domain.is_empty() {
        anyhow::bail!("Domain name cannot be empty");
    }

    // Check for consecutive dots
    if domain.contains("..") {
        anyhow::bail!("Invalid domain: consecutive dots not allowed: {}", domain);
    }

    // Check if domain starts or ends with dot
    if domain.starts_with('.') || domain.ends_with('.') {
        anyhow::bail!("Invalid domain: cannot start or end with dot: {}", domain);
    }

    // Split into labels and validate each
    let labels: Vec<&str> = domain.split('.').collect();

    if labels.is_empty() {
        anyhow::bail!("Invalid domain: no labels found: {}", domain);
    }

    for label in &labels {
        if label.is_empty() {
            anyhow::bail!("Invalid domain: empty label in: {}", domain);
        }

        // Check length (max 63 characters per label)
        if label.len() > 63 {
            anyhow::bail!("Invalid domain: label too long (max 63 chars): {}", label);
        }

        // Check if label starts or ends with hyphen
        if label.starts_with('-') || label.ends_with('-') {
            anyhow::bail!(
                "Invalid domain: label cannot start or end with hyphen: {}",
                label
            );
        }

        // Check if all characters are alphanumeric or hyphen
        for ch in label.chars() {
            if !ch.is_ascii_alphanumeric() && ch != '-' {
                anyhow::bail!(
                    "Invalid domain: invalid character '{}' in label: {}",
                    ch,
                    label
                );
            }
        }
    }

    // Validate TLD (last label)
    if let Some(tld) = labels.last() {
        if tld.chars().all(|c| c.is_ascii_digit()) {
            anyhow::bail!("Invalid domain: TLD cannot be all numeric: {}", tld);
        }
        if tld.len() < 2 {
            anyhow::bail!("Invalid domain: TLD too short: {}", tld);
        }
    }

    Ok(())
}

/// Validate email address format
///
/// Validates that the email:
/// - Contains exactly one '@' character
/// - Has non-empty local and domain parts
/// - Domain part is a valid domain
pub fn validate_email(email: &str) -> Result<()> {
    if email.is_empty() {
        anyhow::bail!("Email address cannot be empty");
    }

    let parts: Vec<&str> = email.split('@').collect();

    if parts.len() != 2 {
        anyhow::bail!(
            "Invalid email address: must contain exactly one '@': {}",
            email
        );
    }

    let local_part = parts[0];
    let domain_part = parts[1];

    // Validate local part
    if local_part.is_empty() {
        anyhow::bail!(
            "Invalid email address: local part cannot be empty: {}",
            email
        );
    }

    // Validate local part length (max 64 characters)
    if local_part.len() > 64 {
        anyhow::bail!(
            "Invalid email address: local part too long (max 64 chars): {}",
            email
        );
    }

    // Validate domain part
    validate_domain(domain_part)
        .with_context(|| format!("Invalid email address domain in: {}", email))?;

    Ok(())
}

/// Validate port number
///
/// Validates that the port:
/// - Is not 0
/// - Is in valid range (1-65535)
/// - Warns if privileged port (<1024)
pub fn validate_port(port: u16, name: &str) -> Result<()> {
    if port == 0 {
        anyhow::bail!("{} cannot be 0", name);
    }

    if port < 1024 {
        tracing::warn!("{} {} is privileged (requires root)", name, port);
    }

    Ok(())
}

/// Validate storage path
///
/// Validates that the path:
/// - Exists and is a directory, or can be created
/// - Is writable by current user
pub fn validate_storage_path(path: &str) -> Result<()> {
    let p = Path::new(path);

    if p.exists() {
        if !p.is_dir() {
            anyhow::bail!("Storage path is not a directory: {}", path);
        }

        // Check if writable by trying to create a test file
        let test_file = p.join(".rusmes_write_test");
        std::fs::write(&test_file, b"test")
            .with_context(|| format!("Storage path is not writable: {}", path))?;
        std::fs::remove_file(test_file)
            .with_context(|| format!("Failed to remove test file in storage path: {}", path))?;
    } else {
        // Try to create the directory
        std::fs::create_dir_all(p)
            .with_context(|| format!("Cannot create storage path: {}", path))?;
    }

    Ok(())
}

/// Validate processor configurations
///
/// Validates that:
/// - Processor names are unique
/// - State names are not empty
pub fn validate_processors(processors: &[crate::ProcessorConfig]) -> Result<()> {
    // Check unique names
    let mut names = HashSet::new();
    for proc in processors {
        if !names.insert(&proc.name) {
            anyhow::bail!("Duplicate processor name: {}", proc.name);
        }

        // Validate state name is not empty
        if proc.state.is_empty() {
            anyhow::bail!("Processor '{}' has empty state name", proc.name);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_domain_valid() {
        assert!(validate_domain("example.com").is_ok());
        assert!(validate_domain("mail.example.com").is_ok());
        assert!(validate_domain("mail-server.example.com").is_ok());
        assert!(validate_domain("a.b.c.d.e.co").is_ok());
        assert!(validate_domain("123.example.com").is_ok());
    }

    #[test]
    fn test_validate_domain_invalid() {
        assert!(validate_domain("").is_err());
        assert!(validate_domain(".example.com").is_err());
        assert!(validate_domain("example.com.").is_err());
        assert!(validate_domain("example..com").is_err());
        assert!(validate_domain("-example.com").is_err());
        assert!(validate_domain("example-.com").is_err());
        assert!(validate_domain("example.123").is_err());
        assert!(validate_domain("example.c").is_err());
        assert!(validate_domain("exa mple.com").is_err());
        assert!(validate_domain("example.c@m").is_err());
    }

    #[test]
    fn test_validate_email_valid() {
        assert!(validate_email("postmaster@example.com").is_ok());
        assert!(validate_email("user@mail.example.com").is_ok());
        assert!(validate_email("test.user@example.com").is_ok());
        assert!(validate_email("a@b.co").is_ok());
    }

    #[test]
    fn test_validate_email_invalid() {
        assert!(validate_email("").is_err());
        assert!(validate_email("invalid").is_err());
        assert!(validate_email("@example.com").is_err());
        assert!(validate_email("user@").is_err());
        assert!(validate_email("user@@example.com").is_err());
        assert!(validate_email("user@example..com").is_err());
        assert!(validate_email("user@.example.com").is_err());
    }

    #[test]
    fn test_validate_port() {
        assert!(validate_port(1, "Test port").is_ok());
        assert!(validate_port(80, "HTTP port").is_ok());
        assert!(validate_port(1024, "User port").is_ok());
        assert!(validate_port(8080, "App port").is_ok());
        assert!(validate_port(65535, "Max port").is_ok());
    }

    #[test]
    fn test_validate_port_invalid() {
        assert!(validate_port(0, "Invalid port").is_err());
    }
}
