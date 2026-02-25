//! IMAP SPECIAL-USE extension (RFC 6154)
//!
//! This module implements the SPECIAL-USE extension which allows clients to
//! identify special-use mailboxes such as Drafts, Sent, Trash, etc.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

/// Special-use attributes defined in RFC 6154
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpecialUse {
    /// \All - Virtual mailbox containing all messages
    All,
    /// \Archive - Archive mailbox
    Archive,
    /// \Drafts - Draft messages
    Drafts,
    /// \Flagged - Messages with \Flagged flag
    Flagged,
    /// \Junk - Spam/junk messages
    Junk,
    /// \Sent - Sent messages
    Sent,
    /// \Trash - Deleted messages
    Trash,
}

impl SpecialUse {
    /// Get the IMAP attribute string for this special use
    pub fn as_str(&self) -> &'static str {
        match self {
            SpecialUse::All => "\\All",
            SpecialUse::Archive => "\\Archive",
            SpecialUse::Drafts => "\\Drafts",
            SpecialUse::Flagged => "\\Flagged",
            SpecialUse::Junk => "\\Junk",
            SpecialUse::Sent => "\\Sent",
            SpecialUse::Trash => "\\Trash",
        }
    }

    /// Get all special use attributes
    pub fn all() -> Vec<SpecialUse> {
        vec![
            SpecialUse::All,
            SpecialUse::Archive,
            SpecialUse::Drafts,
            SpecialUse::Flagged,
            SpecialUse::Junk,
            SpecialUse::Sent,
            SpecialUse::Trash,
        ]
    }
}

impl fmt::Display for SpecialUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for SpecialUse {
    type Err = SpecialUseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "\\All" => Ok(SpecialUse::All),
            "\\Archive" => Ok(SpecialUse::Archive),
            "\\Drafts" => Ok(SpecialUse::Drafts),
            "\\Flagged" => Ok(SpecialUse::Flagged),
            "\\Junk" => Ok(SpecialUse::Junk),
            "\\Sent" => Ok(SpecialUse::Sent),
            "\\Trash" => Ok(SpecialUse::Trash),
            _ => Err(SpecialUseError::InvalidAttribute(s.to_string())),
        }
    }
}

/// Error type for special-use operations
#[derive(Debug, Clone, PartialEq)]
pub enum SpecialUseError {
    /// Invalid special-use attribute
    InvalidAttribute(String),
    /// Multiple special-use attributes where only one is allowed
    MultipleAttributes,
    /// Conflicting special-use attributes
    ConflictingAttributes,
}

impl fmt::Display for SpecialUseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpecialUseError::InvalidAttribute(s) => {
                write!(f, "Invalid special-use attribute: {}", s)
            }
            SpecialUseError::MultipleAttributes => {
                write!(f, "Multiple special-use attributes not allowed")
            }
            SpecialUseError::ConflictingAttributes => {
                write!(f, "Conflicting special-use attributes")
            }
        }
    }
}

impl std::error::Error for SpecialUseError {}

/// Collection of special-use flags for a mailbox
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialUseFlags {
    flags: HashSet<SpecialUse>,
}

impl SpecialUseFlags {
    /// Create a new empty set of special-use flags
    pub fn new() -> Self {
        Self {
            flags: HashSet::new(),
        }
    }

    /// Create flags with a single special-use attribute
    pub fn single(special_use: SpecialUse) -> Self {
        let mut flags = HashSet::new();
        flags.insert(special_use);
        Self { flags }
    }

    /// Add a special-use flag
    pub fn add(&mut self, special_use: SpecialUse) {
        self.flags.insert(special_use);
    }

    /// Remove a special-use flag
    pub fn remove(&mut self, special_use: &SpecialUse) -> bool {
        self.flags.remove(special_use)
    }

    /// Check if a specific special-use flag is set
    pub fn has(&self, special_use: &SpecialUse) -> bool {
        self.flags.contains(special_use)
    }

    /// Check if any special-use flags are set
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    /// Get the number of special-use flags
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    /// Get an iterator over the flags
    pub fn iter(&self) -> impl Iterator<Item = &SpecialUse> {
        self.flags.iter()
    }

    /// Convert to a vector of flags
    pub fn to_vec(&self) -> Vec<SpecialUse> {
        self.flags.iter().copied().collect()
    }

    /// Parse special-use flags from a string list
    pub fn parse(attributes: &[String]) -> Result<Self, SpecialUseError> {
        let mut flags = HashSet::new();
        for attr in attributes {
            if let Ok(special_use) = attr.parse::<SpecialUse>() {
                flags.insert(special_use);
            }
        }
        Ok(Self { flags })
    }

    /// Format as IMAP LIST response attributes
    pub fn format_list_attributes(&self) -> Vec<String> {
        self.flags.iter().map(|f| f.to_string()).collect()
    }

    /// Check if this mailbox is a drafts mailbox
    pub fn is_drafts(&self) -> bool {
        self.has(&SpecialUse::Drafts)
    }

    /// Check if this mailbox is a sent mailbox
    pub fn is_sent(&self) -> bool {
        self.has(&SpecialUse::Sent)
    }

    /// Check if this mailbox is a trash mailbox
    pub fn is_trash(&self) -> bool {
        self.has(&SpecialUse::Trash)
    }

    /// Check if this mailbox is a junk mailbox
    pub fn is_junk(&self) -> bool {
        self.has(&SpecialUse::Junk)
    }

    /// Check if this mailbox is an archive mailbox
    pub fn is_archive(&self) -> bool {
        self.has(&SpecialUse::Archive)
    }

    /// Check if this mailbox is an all mailbox
    pub fn is_all(&self) -> bool {
        self.has(&SpecialUse::All)
    }

    /// Check if this mailbox is a flagged mailbox
    pub fn is_flagged(&self) -> bool {
        self.has(&SpecialUse::Flagged)
    }
}

impl FromIterator<SpecialUse> for SpecialUseFlags {
    fn from_iter<T: IntoIterator<Item = SpecialUse>>(iter: T) -> Self {
        Self {
            flags: iter.into_iter().collect(),
        }
    }
}

impl fmt::Display for SpecialUseFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let attrs: Vec<String> = self.format_list_attributes();
        write!(f, "{}", attrs.join(" "))
    }
}

/// Parse CREATE-SPECIAL-USE command parameters
pub fn parse_create_special_use(
    params: &[String],
) -> Result<(String, SpecialUse), SpecialUseError> {
    if params.len() < 2 {
        return Err(SpecialUseError::InvalidAttribute(
            "Missing mailbox name or special-use attribute".to_string(),
        ));
    }

    let mailbox = params[0].clone();
    let special_use = params[1].parse::<SpecialUse>()?;

    Ok((mailbox, special_use))
}

/// Format LIST-EXTENDED response with special-use attributes
pub fn format_list_extended(
    mailbox_name: &str,
    delimiter: char,
    attributes: &[String],
    special_use: &SpecialUseFlags,
) -> String {
    let mut all_attrs = attributes.to_vec();
    all_attrs.extend(special_use.format_list_attributes());

    let attrs_str = all_attrs.join(" ");
    format!(
        r#"* LIST ({}) "{}" "{}""#,
        attrs_str, delimiter, mailbox_name
    )
}

/// SPECIAL-USE capability string
pub const SPECIAL_USE_CAPABILITY: &str = "SPECIAL-USE";

/// LIST-EXTENDED capability string
pub const LIST_EXTENDED_CAPABILITY: &str = "LIST-EXTENDED";

/// Check if a mailbox name suggests a special use
pub fn suggest_special_use(mailbox_name: &str) -> Option<SpecialUse> {
    let lower = mailbox_name.to_lowercase();

    if lower.contains("draft") {
        Some(SpecialUse::Drafts)
    } else if lower.contains("sent") {
        Some(SpecialUse::Sent)
    } else if lower.contains("trash") || lower.contains("delete") {
        Some(SpecialUse::Trash)
    } else if lower.contains("junk") || lower.contains("spam") {
        Some(SpecialUse::Junk)
    } else if lower.contains("archive") {
        Some(SpecialUse::Archive)
    } else {
        None
    }
}

/// Validate that special-use attributes are not conflicting
pub fn validate_special_use_flags(flags: &SpecialUseFlags) -> Result<(), SpecialUseError> {
    // Virtual mailboxes (\All, \Flagged) should not be combined with other uses
    if flags.has(&SpecialUse::All) && flags.len() > 1 {
        return Err(SpecialUseError::ConflictingAttributes);
    }
    if flags.has(&SpecialUse::Flagged) && flags.len() > 1 {
        return Err(SpecialUseError::ConflictingAttributes);
    }

    Ok(())
}

/// Format CAPABILITY response including SPECIAL-USE and LIST-EXTENDED
pub fn format_capability_response() -> String {
    format!("{} {}", SPECIAL_USE_CAPABILITY, LIST_EXTENDED_CAPABILITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_special_use_as_str() {
        assert_eq!(SpecialUse::All.as_str(), "\\All");
        assert_eq!(SpecialUse::Archive.as_str(), "\\Archive");
        assert_eq!(SpecialUse::Drafts.as_str(), "\\Drafts");
        assert_eq!(SpecialUse::Flagged.as_str(), "\\Flagged");
        assert_eq!(SpecialUse::Junk.as_str(), "\\Junk");
        assert_eq!(SpecialUse::Sent.as_str(), "\\Sent");
        assert_eq!(SpecialUse::Trash.as_str(), "\\Trash");
    }

    #[test]
    fn test_special_use_from_str() {
        assert_eq!(
            "\\All"
                .parse::<SpecialUse>()
                .expect("\\All should parse to SpecialUse::All"),
            SpecialUse::All
        );
        assert_eq!(
            "\\Archive"
                .parse::<SpecialUse>()
                .expect("\\Archive should parse to SpecialUse::Archive"),
            SpecialUse::Archive
        );
        assert_eq!(
            "\\Drafts"
                .parse::<SpecialUse>()
                .expect("\\Drafts should parse to SpecialUse::Drafts"),
            SpecialUse::Drafts
        );
        assert_eq!(
            "\\Flagged"
                .parse::<SpecialUse>()
                .expect("\\Flagged should parse to SpecialUse::Flagged"),
            SpecialUse::Flagged
        );
        assert_eq!(
            "\\Junk"
                .parse::<SpecialUse>()
                .expect("\\Junk should parse to SpecialUse::Junk"),
            SpecialUse::Junk
        );
        assert_eq!(
            "\\Sent"
                .parse::<SpecialUse>()
                .expect("\\Sent should parse to SpecialUse::Sent"),
            SpecialUse::Sent
        );
        assert_eq!(
            "\\Trash"
                .parse::<SpecialUse>()
                .expect("\\Trash should parse to SpecialUse::Trash"),
            SpecialUse::Trash
        );
    }

    #[test]
    fn test_special_use_from_str_invalid() {
        assert!("\\Invalid".parse::<SpecialUse>().is_err());
        assert!("All".parse::<SpecialUse>().is_err());
        assert!("".parse::<SpecialUse>().is_err());
    }

    #[test]
    fn test_special_use_display() {
        assert_eq!(SpecialUse::Drafts.to_string(), "\\Drafts");
        assert_eq!(SpecialUse::Sent.to_string(), "\\Sent");
    }

    #[test]
    fn test_special_use_all() {
        let all = SpecialUse::all();
        assert_eq!(all.len(), 7);
        assert!(all.contains(&SpecialUse::All));
        assert!(all.contains(&SpecialUse::Archive));
        assert!(all.contains(&SpecialUse::Drafts));
        assert!(all.contains(&SpecialUse::Flagged));
        assert!(all.contains(&SpecialUse::Junk));
        assert!(all.contains(&SpecialUse::Sent));
        assert!(all.contains(&SpecialUse::Trash));
    }

    #[test]
    fn test_special_use_flags_new() {
        let flags = SpecialUseFlags::new();
        assert!(flags.is_empty());
        assert_eq!(flags.len(), 0);
    }

    #[test]
    fn test_special_use_flags_single() {
        let flags = SpecialUseFlags::single(SpecialUse::Drafts);
        assert!(!flags.is_empty());
        assert_eq!(flags.len(), 1);
        assert!(flags.has(&SpecialUse::Drafts));
        assert!(!flags.has(&SpecialUse::Sent));
    }

    #[test]
    fn test_special_use_flags_add_remove() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Drafts);
        assert!(flags.has(&SpecialUse::Drafts));
        assert_eq!(flags.len(), 1);

        flags.add(SpecialUse::Sent);
        assert!(flags.has(&SpecialUse::Sent));
        assert_eq!(flags.len(), 2);

        assert!(flags.remove(&SpecialUse::Drafts));
        assert!(!flags.has(&SpecialUse::Drafts));
        assert_eq!(flags.len(), 1);

        assert!(!flags.remove(&SpecialUse::Drafts));
    }

    #[test]
    fn test_special_use_flags_parse() {
        let attrs = vec![
            "\\Drafts".to_string(),
            "\\NoSelect".to_string(),
            "\\Sent".to_string(),
        ];
        let flags = SpecialUseFlags::parse(&attrs)
            .expect("SpecialUseFlags::parse with valid attrs should succeed");
        assert!(flags.has(&SpecialUse::Drafts));
        assert!(flags.has(&SpecialUse::Sent));
        assert_eq!(flags.len(), 2);
    }

    #[test]
    fn test_special_use_flags_format_list_attributes() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Drafts);
        flags.add(SpecialUse::Sent);

        let attrs = flags.format_list_attributes();
        assert_eq!(attrs.len(), 2);
        assert!(attrs.contains(&"\\Drafts".to_string()));
        assert!(attrs.contains(&"\\Sent".to_string()));
    }

    #[test]
    fn test_special_use_flags_is_methods() {
        let mut flags = SpecialUseFlags::new();

        flags.add(SpecialUse::Drafts);
        assert!(flags.is_drafts());
        assert!(!flags.is_sent());

        flags.add(SpecialUse::Sent);
        assert!(flags.is_sent());

        flags.add(SpecialUse::Trash);
        assert!(flags.is_trash());

        flags.add(SpecialUse::Junk);
        assert!(flags.is_junk());

        flags.add(SpecialUse::Archive);
        assert!(flags.is_archive());

        flags.add(SpecialUse::All);
        assert!(flags.is_all());

        flags.add(SpecialUse::Flagged);
        assert!(flags.is_flagged());
    }

    #[test]
    fn test_special_use_flags_from_iter() {
        let flags: SpecialUseFlags = vec![SpecialUse::Drafts, SpecialUse::Sent]
            .into_iter()
            .collect();
        assert_eq!(flags.len(), 2);
        assert!(flags.has(&SpecialUse::Drafts));
        assert!(flags.has(&SpecialUse::Sent));
    }

    #[test]
    fn test_parse_create_special_use() {
        let params = vec!["INBOX/Drafts".to_string(), "\\Drafts".to_string()];
        let (mailbox, special_use) = parse_create_special_use(&params)
            .expect("parse_create_special_use with valid params should succeed");
        assert_eq!(mailbox, "INBOX/Drafts");
        assert_eq!(special_use, SpecialUse::Drafts);
    }

    #[test]
    fn test_parse_create_special_use_invalid() {
        let params = vec!["INBOX/Drafts".to_string()];
        assert!(parse_create_special_use(&params).is_err());

        let params = vec!["INBOX/Drafts".to_string(), "\\Invalid".to_string()];
        assert!(parse_create_special_use(&params).is_err());
    }

    #[test]
    fn test_format_list_extended() {
        let mut special_use = SpecialUseFlags::new();
        special_use.add(SpecialUse::Drafts);

        let result = format_list_extended(
            "INBOX/Drafts",
            '/',
            &["\\HasNoChildren".to_string()],
            &special_use,
        );

        assert!(result.contains("\\HasNoChildren"));
        assert!(result.contains("\\Drafts"));
        assert!(result.contains("INBOX/Drafts"));
    }

    #[test]
    fn test_format_list_extended_multiple() {
        let mut special_use = SpecialUseFlags::new();
        special_use.add(SpecialUse::Sent);
        special_use.add(SpecialUse::Archive);

        let result = format_list_extended(
            "INBOX/Sent",
            '/',
            &["\\HasChildren".to_string()],
            &special_use,
        );

        assert!(result.contains("\\HasChildren"));
        assert!(result.contains("\\Sent"));
        assert!(result.contains("\\Archive"));
    }

    #[test]
    fn test_special_use_flags_to_vec() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Drafts);
        flags.add(SpecialUse::Sent);

        let vec = flags.to_vec();
        assert_eq!(vec.len(), 2);
        assert!(vec.contains(&SpecialUse::Drafts));
        assert!(vec.contains(&SpecialUse::Sent));
    }

    #[test]
    fn test_special_use_flags_iter() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Drafts);
        flags.add(SpecialUse::Sent);

        let mut count = 0;
        for _flag in flags.iter() {
            count += 1;
        }
        assert_eq!(count, 2);
    }

    #[test]
    fn test_special_use_error_display() {
        let err = SpecialUseError::InvalidAttribute("\\Bad".to_string());
        assert_eq!(err.to_string(), "Invalid special-use attribute: \\Bad");

        let err = SpecialUseError::MultipleAttributes;
        assert_eq!(
            err.to_string(),
            "Multiple special-use attributes not allowed"
        );

        let err = SpecialUseError::ConflictingAttributes;
        assert_eq!(err.to_string(), "Conflicting special-use attributes");
    }

    #[test]
    fn test_special_use_flags_display() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Drafts);

        let display = flags.to_string();
        assert!(display.contains("\\Drafts"));
    }

    #[test]
    fn test_suggest_special_use_drafts() {
        assert_eq!(suggest_special_use("Drafts"), Some(SpecialUse::Drafts));
        assert_eq!(
            suggest_special_use("INBOX/Drafts"),
            Some(SpecialUse::Drafts)
        );
        assert_eq!(suggest_special_use("draft"), Some(SpecialUse::Drafts));
    }

    #[test]
    fn test_suggest_special_use_sent() {
        assert_eq!(suggest_special_use("Sent"), Some(SpecialUse::Sent));
        assert_eq!(suggest_special_use("Sent Items"), Some(SpecialUse::Sent));
        assert_eq!(suggest_special_use("INBOX/sent"), Some(SpecialUse::Sent));
    }

    #[test]
    fn test_suggest_special_use_trash() {
        assert_eq!(suggest_special_use("Trash"), Some(SpecialUse::Trash));
        assert_eq!(
            suggest_special_use("Deleted Items"),
            Some(SpecialUse::Trash)
        );
        assert_eq!(suggest_special_use("trash"), Some(SpecialUse::Trash));
    }

    #[test]
    fn test_suggest_special_use_junk() {
        assert_eq!(suggest_special_use("Junk"), Some(SpecialUse::Junk));
        assert_eq!(suggest_special_use("Spam"), Some(SpecialUse::Junk));
        assert_eq!(suggest_special_use("INBOX/junk"), Some(SpecialUse::Junk));
    }

    #[test]
    fn test_suggest_special_use_archive() {
        assert_eq!(suggest_special_use("Archive"), Some(SpecialUse::Archive));
        assert_eq!(
            suggest_special_use("INBOX/Archive"),
            Some(SpecialUse::Archive)
        );
        assert_eq!(suggest_special_use("archive"), Some(SpecialUse::Archive));
    }

    #[test]
    fn test_suggest_special_use_none() {
        assert_eq!(suggest_special_use("INBOX"), None);
        assert_eq!(suggest_special_use("Work"), None);
        assert_eq!(suggest_special_use("Projects"), None);
    }

    #[test]
    fn test_validate_special_use_flags_ok() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Drafts);
        flags.add(SpecialUse::Archive);
        assert!(validate_special_use_flags(&flags).is_ok());

        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Sent);
        assert!(validate_special_use_flags(&flags).is_ok());
    }

    #[test]
    fn test_validate_special_use_flags_all_conflict() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::All);
        flags.add(SpecialUse::Drafts);
        assert!(matches!(
            validate_special_use_flags(&flags),
            Err(SpecialUseError::ConflictingAttributes)
        ));
    }

    #[test]
    fn test_validate_special_use_flags_flagged_conflict() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Flagged);
        flags.add(SpecialUse::Sent);
        assert!(matches!(
            validate_special_use_flags(&flags),
            Err(SpecialUseError::ConflictingAttributes)
        ));
    }

    #[test]
    fn test_validate_special_use_flags_all_alone_ok() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::All);
        assert!(validate_special_use_flags(&flags).is_ok());
    }

    #[test]
    fn test_validate_special_use_flags_flagged_alone_ok() {
        let mut flags = SpecialUseFlags::new();
        flags.add(SpecialUse::Flagged);
        assert!(validate_special_use_flags(&flags).is_ok());
    }

    #[test]
    fn test_format_capability_response() {
        let response = format_capability_response();
        assert!(response.contains("SPECIAL-USE"));
        assert!(response.contains("LIST-EXTENDED"));
    }

    #[test]
    fn test_special_use_capability_const() {
        assert_eq!(SPECIAL_USE_CAPABILITY, "SPECIAL-USE");
        assert_eq!(LIST_EXTENDED_CAPABILITY, "LIST-EXTENDED");
    }

    #[test]
    fn test_special_use_flags_equality() {
        let mut flags1 = SpecialUseFlags::new();
        flags1.add(SpecialUse::Drafts);
        flags1.add(SpecialUse::Sent);

        let mut flags2 = SpecialUseFlags::new();
        flags2.add(SpecialUse::Sent);
        flags2.add(SpecialUse::Drafts);

        assert_eq!(flags1, flags2);
    }

    #[test]
    fn test_special_use_flags_default() {
        let flags = SpecialUseFlags::default();
        assert!(flags.is_empty());
    }

    #[test]
    fn test_special_use_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(SpecialUse::Drafts);
        set.insert(SpecialUse::Drafts); // duplicate
        assert_eq!(set.len(), 1);
    }
}
