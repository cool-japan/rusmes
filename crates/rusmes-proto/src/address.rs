//! Email address types

use crate::error::{MailError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

/// Represents a valid email address
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MailAddress {
    local_part: String,
    domain: Domain,
}

impl MailAddress {
    /// Create a new email address with validation (ASCII-only local-part).
    ///
    /// This is the default, RFC 5321-compliant constructor. The local-part must be
    /// pure 7-bit ASCII. If the local-part contains bytes ≥ 0x80 (i.e. multi-byte
    /// UTF-8 sequences), use [`MailAddress::new_smtputf8`] instead and ensure
    /// SMTPUTF8 was negotiated for the SMTP session (RFC 6531).
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

        // Reject C0 control characters (0x00–0x1F) and DEL (0x7F).
        if local_part.bytes().any(|b| b < 0x20 || b == 0x7F) {
            return Err(MailError::InvalidAddress(
                "local-part contains control character".to_string(),
            ));
        }

        // Reject non-ASCII bytes — caller must use new_smtputf8 instead.
        if local_part.bytes().any(|b| b >= 0x80) {
            return Err(MailError::NonAsciiLocalPartRequiresSMTPUTF8);
        }

        Ok(Self { local_part, domain })
    }

    /// Create a new email address allowing a non-ASCII (UTF-8) local-part.
    ///
    /// This constructor is used when SMTPUTF8 has been negotiated for the SMTP
    /// session (RFC 6531). The local-part is validated according to RFC 6531 rules:
    ///
    /// - Octet length must be 1–64 bytes (same RFC 5321 limit, measured in bytes).
    /// - The local-part must not contain `'@'`.
    /// - C0 (U+0000–U+001F), DEL (U+007F) and C1 (U+0080–U+009F) control
    ///   codepoints are rejected.
    /// - All other Unicode codepoints are accepted.
    pub fn new_smtputf8(local_part: impl Into<String>, domain: Domain) -> Result<Self> {
        let local_part = local_part.into();

        // Octet-length limit per RFC 5321 §4.5.3.1.1 — applies to UTF-8 too.
        if local_part.is_empty() || local_part.len() > 64 {
            return Err(MailError::InvalidAddress(format!(
                "local-part octet length must be 1-64, got {}",
                local_part.len()
            )));
        }

        if local_part.contains('@') {
            return Err(MailError::InvalidAddress(
                "local-part cannot contain '@'".to_string(),
            ));
        }

        // Reject C0/C1 control codepoints per RFC 6531.
        for ch in local_part.chars() {
            let cp = ch as u32;
            if cp < 0x20 || (0x7F..=0x9F).contains(&cp) {
                return Err(MailError::InvalidAddress(format!(
                    "local-part contains disallowed control codepoint U+{:04X}",
                    cp
                )));
            }
        }

        Ok(Self { local_part, domain })
    }

    /// Parse an email address string allowing a non-ASCII (UTF-8) local-part.
    ///
    /// Equivalent to [`FromStr`] but calls [`MailAddress::new_smtputf8`] for the
    /// local-part so that internationalized addresses (RFC 6531) are accepted.
    /// The domain portion is still validated with [`Domain::new`] which requires
    /// ASCII / Punycode-encoded domain labels.
    ///
    /// Splits on the **last** `'@'` in the string so that a literal `'@'` in the
    /// local-part (rare but syntactically possible when quoted per RFC 5321) would
    /// never accidentally eat part of the domain.
    pub fn from_str_smtputf8(s: &str) -> Result<Self> {
        let at_pos = s
            .rfind('@')
            .ok_or_else(|| MailError::InvalidAddress("missing '@'".to_string()))?;
        let local = &s[..at_pos];
        let domain_str = &s[at_pos + 1..];
        let domain = Domain::new(domain_str)?;
        Self::new_smtputf8(local, domain)
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

    /// Returns `true` if the address's domain is in the supplied local-domain set.
    ///
    /// Comparison is case-insensitive and IDN-aware:
    ///
    /// * Both the address's own domain and every entry in `domain_set` are
    ///   normalized to ASCII Punycode via [`idna::domain_to_ascii`] before
    ///   comparison, so a Unicode domain such as `münchen.de` will match an
    ///   ASCII Punycode entry such as `xn--mnchen-3ya.de` (and vice versa).
    /// * ASCII case differences are folded to lowercase.
    /// * Entries in `domain_set` that fail IDN normalization (malformed domains)
    ///   are silently skipped — they cannot match any valid address.
    /// * If this address's own domain fails IDN normalization the function
    ///   returns `false` (no match).
    ///
    /// The membership check is `O(n)` over `domain_set` because every entry
    /// must be IDN-normalized before lookup. Callers that need repeated lookups
    /// should pre-normalize their set once.
    pub fn is_local(&self, domain_set: &HashSet<String>) -> bool {
        // Normalize this address's domain once.
        let own_ascii = match idna::domain_to_ascii(self.domain.as_str()) {
            Ok(ascii) => ascii.to_ascii_lowercase(),
            Err(_) => return false,
        };

        // Walk the set, normalizing each entry on the fly. We deliberately
        // accept either Unicode or Punycode entries — this is the contract
        // the trait was specified with.
        for entry in domain_set {
            if let Ok(entry_ascii) = idna::domain_to_ascii(entry) {
                if entry_ascii.to_ascii_lowercase() == own_ascii {
                    return true;
                }
            }
        }
        false
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

    #[test]
    fn test_mailaddress_ascii_only_rejects_unicode() {
        // FromStr calls MailAddress::new which is ASCII-only.
        let result = "münchen@example.com".parse::<MailAddress>();
        assert!(
            matches!(result, Err(MailError::NonAsciiLocalPartRequiresSMTPUTF8)),
            "expected NonAsciiLocalPartRequiresSMTPUTF8, got {:?}",
            result
        );
    }

    #[test]
    fn test_mailaddress_smtputf8_roundtrip() {
        let addr = MailAddress::from_str_smtputf8("münchen@example.com")
            .expect("SMTPUTF8 address must be accepted");
        assert_eq!(addr.local_part(), "münchen");
        assert_eq!(addr.as_string(), "münchen@example.com");
    }

    #[test]
    fn test_mailaddress_64_byte_limit_unicode() {
        // Each CJK character encodes as 3 UTF-8 bytes.
        // 22 × 3 = 66 bytes → should be rejected.
        let long_local: String = "中".repeat(22);
        assert!(
            MailAddress::new_smtputf8(long_local, Domain::new("example.com").unwrap()).is_err(),
            "66-byte local-part must be rejected"
        );

        // 16 × 3 = 48 bytes → should be accepted.
        let ok_local: String = "中".repeat(16);
        assert!(
            MailAddress::new_smtputf8(ok_local, Domain::new("example.com").unwrap()).is_ok(),
            "48-byte local-part must be accepted"
        );
    }

    #[test]
    fn test_mailaddress_smtputf8_rejects_control_chars() {
        // U+0001 (C0 control) — rejected.
        let result = MailAddress::new_smtputf8("\x01user", Domain::new("example.com").unwrap());
        assert!(result.is_err(), "C0 control must be rejected");

        // U+0085 (NEXT LINE, C1 control) — rejected.
        let result = MailAddress::new_smtputf8("\u{0085}user", Domain::new("example.com").unwrap());
        assert!(result.is_err(), "C1 control must be rejected");
    }

    #[test]
    fn test_mailaddress_new_rejects_control_chars() {
        // DEL (0x7F) should be rejected by MailAddress::new.
        let result = MailAddress::new("user\x7f", Domain::new("example.com").unwrap());
        assert!(result.is_err(), "DEL character must be rejected by new()");
    }

    #[test]
    fn is_local_case_insensitive_and_idn() {
        // -- Case insensitivity --
        // Address constructed with mixed-case domain (Domain::new lowercases on
        // construction, so this only tests one half; the more interesting
        // direction is mixed-case in the *set*).
        let addr: MailAddress = "user@Example.COM".parse().expect("parse address");
        let mut set = HashSet::new();
        set.insert("EXAMPLE.com".to_string());
        assert!(
            addr.is_local(&set),
            "case-insensitive match against mixed-case set entry"
        );

        // Empty set never matches.
        let empty: HashSet<String> = HashSet::new();
        assert!(!addr.is_local(&empty), "empty set must never match");

        // Different domain doesn't match.
        let mut other = HashSet::new();
        other.insert("example.org".to_string());
        assert!(!addr.is_local(&other));

        // -- IDN normalization --
        // Address with Punycode-encoded German "münchen.de"; set with the
        // Unicode form. They must still compare as equal after IDN normalization.
        let punycode_addr: MailAddress = "user@xn--mnchen-3ya.de"
            .parse()
            .expect("parse punycode address");
        let mut idn_set = HashSet::new();
        idn_set.insert("münchen.de".to_string());
        assert!(
            punycode_addr.is_local(&idn_set),
            "Punycode address must match Unicode set entry"
        );

        // -- Reverse direction: Unicode address (constructed via raw Domain)
        // is unsupported because Domain::new() rejects non-ASCII characters;
        // we still verify the set itself can hold both forms and one matches.
        let mut both_forms = HashSet::new();
        both_forms.insert("xn--mnchen-3ya.de".to_string());
        both_forms.insert("example.com".to_string());
        assert!(punycode_addr.is_local(&both_forms));

        // -- Malformed entry in set is silently skipped, not propagated.
        let mut with_bad = HashSet::new();
        with_bad.insert(String::new()); // empty string - idna typically errors
        with_bad.insert("example.com".to_string());
        assert!(addr.is_local(&with_bad));
    }
}
