//! Storage-related types

use rusmes_proto::{MessageId, Username};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use uuid::Uuid;

/// Unique identifier for a mailbox
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MailboxId(Uuid);

impl MailboxId {
    /// Create a new unique mailbox ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a MailboxId from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the UUID value
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for MailboxId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MailboxId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Mailbox path (hierarchical)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MailboxPath {
    user: Username,
    path: Vec<String>,
}

impl MailboxPath {
    /// Create a new mailbox path
    pub fn new(user: Username, path: Vec<String>) -> Self {
        Self { user, path }
    }

    /// Get the user
    pub fn user(&self) -> &Username {
        &self.user
    }

    /// Get the path components
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// Get the mailbox name (last component)
    pub fn name(&self) -> Option<&str> {
        self.path.last().map(|s| s.as_str())
    }
}

impl std::fmt::Display for MailboxPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.user, self.path.join("/"))
    }
}

/// Mailbox metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mailbox {
    id: MailboxId,
    path: MailboxPath,
    uid_validity: u32,
    uid_next: u32,
    special_use: Option<String>,
}

impl Mailbox {
    /// Create a new mailbox
    pub fn new(path: MailboxPath) -> Self {
        Self {
            id: MailboxId::new(),
            path,
            uid_validity: 1, // Would be more sophisticated in production
            uid_next: 1,
            special_use: None,
        }
    }

    /// Create a new mailbox with special-use attribute
    pub fn new_with_special_use(path: MailboxPath, special_use: String) -> Self {
        Self {
            id: MailboxId::new(),
            path,
            uid_validity: 1,
            uid_next: 1,
            special_use: Some(special_use),
        }
    }

    /// Get the mailbox ID
    pub fn id(&self) -> &MailboxId {
        &self.id
    }

    /// Get the mailbox path
    pub fn path(&self) -> &MailboxPath {
        &self.path
    }

    /// Set the mailbox path (for rename operations)
    pub fn set_path(&mut self, path: MailboxPath) {
        self.path = path;
    }

    /// Get UID validity
    pub fn uid_validity(&self) -> u32 {
        self.uid_validity
    }

    /// Get next UID
    pub fn uid_next(&self) -> u32 {
        self.uid_next
    }

    /// Get special-use attribute
    pub fn special_use(&self) -> Option<&str> {
        self.special_use.as_deref()
    }

    /// Set special-use attribute
    pub fn set_special_use(&mut self, special_use: Option<String>) {
        self.special_use = special_use;
    }
}

/// Message metadata in a mailbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMetadata {
    message_id: MessageId,
    mailbox_id: MailboxId,
    uid: u32,
    flags: MessageFlags,
    size: usize,
    /// RFC 5256 thread identifier assigned at delivery time.
    /// `None` for messages stored before threading was introduced, or for
    /// backends that do not implement threading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl MessageMetadata {
    /// Create new message metadata
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        message_id: MessageId,
        mailbox_id: MailboxId,
        uid: u32,
        flags: MessageFlags,
        size: usize,
    ) -> Self {
        Self {
            message_id,
            mailbox_id,
            uid,
            flags,
            size,
            thread_id: None,
        }
    }

    /// Create new message metadata with a thread ID.
    pub fn new_with_thread_id(
        message_id: MessageId,
        mailbox_id: MailboxId,
        uid: u32,
        flags: MessageFlags,
        size: usize,
        thread_id: Option<String>,
    ) -> Self {
        Self {
            message_id,
            mailbox_id,
            uid,
            flags,
            size,
            thread_id,
        }
    }

    /// Get message ID
    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }

    /// Get mailbox ID
    pub fn mailbox_id(&self) -> &MailboxId {
        &self.mailbox_id
    }

    /// Get UID
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Get flags
    pub fn flags(&self) -> &MessageFlags {
        &self.flags
    }

    /// Get size
    pub fn size(&self) -> usize {
        self.size
    }
}

/// Message flags (IMAP standard flags)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageFlags {
    seen: bool,
    answered: bool,
    flagged: bool,
    deleted: bool,
    draft: bool,
    recent: bool,
    custom: HashSet<String>,
}

impl MessageFlags {
    /// Create new empty flags
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if message is seen
    pub fn is_seen(&self) -> bool {
        self.seen
    }

    /// Set seen flag
    pub fn set_seen(&mut self, value: bool) {
        self.seen = value;
    }

    /// Check if message is answered
    pub fn is_answered(&self) -> bool {
        self.answered
    }

    /// Set answered flag
    pub fn set_answered(&mut self, value: bool) {
        self.answered = value;
    }

    /// Check if message is flagged
    pub fn is_flagged(&self) -> bool {
        self.flagged
    }

    /// Set flagged flag
    pub fn set_flagged(&mut self, value: bool) {
        self.flagged = value;
    }

    /// Check if message is deleted
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    /// Set deleted flag
    pub fn set_deleted(&mut self, value: bool) {
        self.deleted = value;
    }

    /// Check if message is draft
    pub fn is_draft(&self) -> bool {
        self.draft
    }

    /// Set draft flag
    pub fn set_draft(&mut self, value: bool) {
        self.draft = value;
    }

    /// Check if message is recent
    pub fn is_recent(&self) -> bool {
        self.recent
    }

    /// Set recent flag
    pub fn set_recent(&mut self, value: bool) {
        self.recent = value;
    }

    /// Add custom flag
    pub fn add_custom(&mut self, flag: String) {
        self.custom.insert(flag);
    }

    /// Remove custom flag
    pub fn remove_custom(&mut self, flag: &str) -> bool {
        self.custom.remove(flag)
    }

    /// Get custom flags
    pub fn custom(&self) -> &HashSet<String> {
        &self.custom
    }
}

/// Search criteria for messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SearchCriteria {
    All,
    Unseen,
    Seen,
    Flagged,
    Unflagged,
    Deleted,
    Undeleted,
    From(String),
    To(String),
    Subject(String),
    Body(String),
    And(Vec<SearchCriteria>),
    Or(Vec<SearchCriteria>),
    Not(Box<SearchCriteria>),
}

/// Mailbox counters
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct MailboxCounters {
    pub exists: u32,
    pub recent: u32,
    pub unseen: u32,
}

/// User quota information
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Quota {
    pub used: u64,
    pub limit: u64,
}

impl Quota {
    /// Create new quota
    pub fn new(used: u64, limit: u64) -> Self {
        Self { used, limit }
    }

    /// Check if quota is exceeded
    pub fn is_exceeded(&self) -> bool {
        self.used >= self.limit
    }

    /// Get remaining quota
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }
}

/// Special-use mailbox attributes (RFC 6154)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialUseAttributes {
    attributes: HashSet<String>,
}

impl SpecialUseAttributes {
    /// Create new empty special-use attributes
    pub fn new() -> Self {
        Self {
            attributes: HashSet::new(),
        }
    }

    /// Create with a single attribute
    pub fn single(attribute: String) -> Self {
        let mut attrs = HashSet::new();
        attrs.insert(attribute);
        Self { attributes: attrs }
    }

    /// Add an attribute
    pub fn add(&mut self, attribute: String) {
        self.attributes.insert(attribute);
    }

    /// Remove an attribute
    pub fn remove(&mut self, attribute: &str) -> bool {
        self.attributes.remove(attribute)
    }

    /// Check if a specific attribute is set
    pub fn has_attribute(&self, attribute: &str) -> bool {
        self.attributes.contains(attribute)
    }

    /// Check if any attributes are set
    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }

    /// Get the number of attributes
    pub fn len(&self) -> usize {
        self.attributes.len()
    }

    /// Get an iterator over the attributes
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.attributes.iter()
    }

    /// Convert to a vector of attributes
    pub fn to_vec(&self) -> Vec<String> {
        self.attributes.iter().cloned().collect()
    }

    /// Parse from a list of attribute strings
    pub fn from_vec(attributes: Vec<String>) -> Self {
        Self {
            attributes: attributes.into_iter().collect(),
        }
    }

    /// Common special-use attributes
    pub fn drafts() -> Self {
        Self::single("\\Drafts".to_string())
    }

    pub fn sent() -> Self {
        Self::single("\\Sent".to_string())
    }

    pub fn trash() -> Self {
        Self::single("\\Trash".to_string())
    }

    pub fn junk() -> Self {
        Self::single("\\Junk".to_string())
    }

    pub fn archive() -> Self {
        Self::single("\\Archive".to_string())
    }

    pub fn all() -> Self {
        Self::single("\\All".to_string())
    }

    pub fn flagged() -> Self {
        Self::single("\\Flagged".to_string())
    }
}

impl FromIterator<String> for SpecialUseAttributes {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        Self {
            attributes: iter.into_iter().collect(),
        }
    }
}

impl fmt::Display for SpecialUseAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let attrs: Vec<&String> = self.attributes.iter().collect();
        write!(
            f,
            "{}",
            attrs
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_special_use_attributes_new() {
        let attrs = SpecialUseAttributes::new();
        assert!(attrs.is_empty());
        assert_eq!(attrs.len(), 0);
    }

    #[test]
    fn test_special_use_attributes_single() {
        let attrs = SpecialUseAttributes::single("\\Drafts".to_string());
        assert!(!attrs.is_empty());
        assert_eq!(attrs.len(), 1);
        assert!(attrs.has_attribute("\\Drafts"));
    }

    #[test]
    fn test_special_use_attributes_add_remove() {
        let mut attrs = SpecialUseAttributes::new();
        attrs.add("\\Drafts".to_string());
        assert!(attrs.has_attribute("\\Drafts"));
        assert_eq!(attrs.len(), 1);

        attrs.add("\\Sent".to_string());
        assert!(attrs.has_attribute("\\Sent"));
        assert_eq!(attrs.len(), 2);

        assert!(attrs.remove("\\Drafts"));
        assert!(!attrs.has_attribute("\\Drafts"));
        assert_eq!(attrs.len(), 1);

        assert!(!attrs.remove("\\Drafts"));
    }

    #[test]
    fn test_special_use_attributes_from_vec() {
        let vec = vec!["\\Drafts".to_string(), "\\Sent".to_string()];
        let attrs = SpecialUseAttributes::from_vec(vec);
        assert_eq!(attrs.len(), 2);
        assert!(attrs.has_attribute("\\Drafts"));
        assert!(attrs.has_attribute("\\Sent"));
    }

    #[test]
    fn test_special_use_attributes_to_vec() {
        let mut attrs = SpecialUseAttributes::new();
        attrs.add("\\Drafts".to_string());
        attrs.add("\\Sent".to_string());

        let vec = attrs.to_vec();
        assert_eq!(vec.len(), 2);
        assert!(vec.contains(&"\\Drafts".to_string()));
        assert!(vec.contains(&"\\Sent".to_string()));
    }

    #[test]
    fn test_special_use_attributes_iter() {
        let mut attrs = SpecialUseAttributes::new();
        attrs.add("\\Drafts".to_string());
        attrs.add("\\Sent".to_string());

        let mut count = 0;
        for _attr in attrs.iter() {
            count += 1;
        }
        assert_eq!(count, 2);
    }

    #[test]
    fn test_special_use_attributes_from_iter() {
        let attrs: SpecialUseAttributes = vec!["\\Drafts".to_string(), "\\Sent".to_string()]
            .into_iter()
            .collect();
        assert_eq!(attrs.len(), 2);
        assert!(attrs.has_attribute("\\Drafts"));
        assert!(attrs.has_attribute("\\Sent"));
    }

    #[test]
    fn test_special_use_attributes_display() {
        let mut attrs = SpecialUseAttributes::new();
        attrs.add("\\Drafts".to_string());

        let display = attrs.to_string();
        assert!(display.contains("\\Drafts"));
    }

    #[test]
    fn test_special_use_attributes_drafts() {
        let attrs = SpecialUseAttributes::drafts();
        assert!(attrs.has_attribute("\\Drafts"));
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_special_use_attributes_sent() {
        let attrs = SpecialUseAttributes::sent();
        assert!(attrs.has_attribute("\\Sent"));
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_special_use_attributes_trash() {
        let attrs = SpecialUseAttributes::trash();
        assert!(attrs.has_attribute("\\Trash"));
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_special_use_attributes_junk() {
        let attrs = SpecialUseAttributes::junk();
        assert!(attrs.has_attribute("\\Junk"));
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_special_use_attributes_archive() {
        let attrs = SpecialUseAttributes::archive();
        assert!(attrs.has_attribute("\\Archive"));
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_special_use_attributes_all() {
        let attrs = SpecialUseAttributes::all();
        assert!(attrs.has_attribute("\\All"));
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_special_use_attributes_flagged() {
        let attrs = SpecialUseAttributes::flagged();
        assert!(attrs.has_attribute("\\Flagged"));
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_special_use_attributes_default() {
        let attrs = SpecialUseAttributes::default();
        assert!(attrs.is_empty());
    }

    #[test]
    fn test_special_use_attributes_equality() {
        let mut attrs1 = SpecialUseAttributes::new();
        attrs1.add("\\Drafts".to_string());
        attrs1.add("\\Sent".to_string());

        let mut attrs2 = SpecialUseAttributes::new();
        attrs2.add("\\Sent".to_string());
        attrs2.add("\\Drafts".to_string());

        assert_eq!(attrs1, attrs2);
    }
}
