//! Email address types

use crate::error::{MailError, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Represents a valid email address
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MailAddress {
    local_part: String,
    domain: Domain,
}

impl MailAddress {
    /// Create a new email address with validation
    pub fn new(local_part: impl Into<String>, domain: Domain) -> Result<Self> {
        let local_part = local_part.into();

        // Basic validation
        if local_part.is_empty() || local_part.len() > 64 {
            return Err(MailError::InvalidAddress(format!(
                "Local part length must be 1-64 characters, got {}",
                local_part.len()
            )));
        }

        if local_part.contains('@') {
            return Err(MailError::InvalidAddress(
                "Local part cannot contain '@'".to_string(),
            ));
        }

        Ok(Self { local_part, domain })
    }

    /// Get the local part (before @)
    pub fn local_part(&self) -> &str {
        &self.local_part
    }

    /// Get the domain part
    pub fn domain(&self) -> &Domain {
        &self.domain
    }

    /// Get the full email address as a string
    pub fn as_string(&self) -> String {
        format!("{}@{}", self.local_part, self.domain.as_str())
    }
}

impl FromStr for MailAddress {
    type Err = MailError;

    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.rsplitn(2, '@').collect();
        if parts.len() != 2 {
            return Err(MailError::InvalidAddress("Missing @ separator".to_string()));
        }

        let domain = Domain::new(parts[0])?;
        let local_part = parts[1];

        Self::new(local_part, domain)
    }
}

impl fmt::Display for MailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.local_part, self.domain.as_str())
    }
}

/// Represents a domain name
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Domain(String);

impl Domain {
    /// Create a new domain with validation
    pub fn new(domain: impl Into<String>) -> Result<Self> {
        let domain = domain.into();

        if domain.is_empty() || domain.len() > 255 {
            return Err(MailError::InvalidDomain(format!(
                "Domain length must be 1-255 characters, got {}",
                domain.len()
            )));
        }

        // Basic validation - check for valid characters
        if !domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Err(MailError::InvalidDomain(
                "Domain contains invalid characters".to_string(),
            ));
        }

        if domain.starts_with('.') || domain.ends_with('.') {
            return Err(MailError::InvalidDomain(
                "Domain cannot start or end with '.'".to_string(),
            ));
        }

        Ok(Self(domain.to_lowercase()))
    }

    /// Get the domain as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Domain {
    type Err = MailError;

    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents a username for authentication
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Username(String);

impl Username {
    /// Create a new username with validation
    pub fn new(username: impl Into<String>) -> Result<Self> {
        let username = username.into();

        if username.is_empty() || username.len() > 128 {
            return Err(MailError::InvalidUsername(format!(
                "Username length must be 1-128 characters, got {}",
                username.len()
            )));
        }

        Ok(Self(username))
    }

    /// Get the username as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Username {
    type Err = MailError;

    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_mail_address() {
        let domain = Domain::new("example.com").unwrap();
        let addr = MailAddress::new("user", domain).unwrap();
        assert_eq!(addr.as_string(), "user@example.com");
    }

    #[test]
    fn test_mail_address_from_str() {
        let addr: MailAddress = "user@example.com".parse().unwrap();
        assert_eq!(addr.local_part(), "user");
        assert_eq!(addr.domain().as_str(), "example.com");
    }

    #[test]
    fn test_invalid_address_no_at() {
        let result: Result<MailAddress> = "userexample.com".parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_domain_case_normalization() {
        let domain1 = Domain::new("Example.COM").unwrap();
        let domain2 = Domain::new("example.com").unwrap();
        assert_eq!(domain1, domain2);
    }
}
