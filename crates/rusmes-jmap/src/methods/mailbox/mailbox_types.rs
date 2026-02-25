//! Types, structs, and helper utilities for JMAP Mailbox methods.
//!
//! Re-exported from the parent `mailbox` module.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Mailbox object as defined in RFC 8621
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mailbox {
    /// Unique identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Parent mailbox ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Special-use role
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<MailboxRole>,
    /// Sort order
    pub sort_order: u32,
    /// Total number of emails
    pub total_emails: u64,
    /// Number of unread emails
    pub unread_emails: u64,
    /// Total number of threads
    pub total_threads: u64,
    /// Number of unread threads
    pub unread_threads: u64,
    /// My rights
    pub my_rights: MailboxRights,
    /// Is subscribed
    pub is_subscribed: bool,
}

/// Mailbox role (special-use)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MailboxRole {
    Inbox,
    Archive,
    Drafts,
    Sent,
    Trash,
    Junk,
    #[serde(rename = "important")]
    Important,
}

impl MailboxRole {
    /// Convert JMAP role to IMAP special-use attribute
    pub fn to_special_use(&self) -> String {
        match self {
            MailboxRole::Inbox => "\\Inbox".to_string(),
            MailboxRole::Archive => "\\Archive".to_string(),
            MailboxRole::Drafts => "\\Drafts".to_string(),
            MailboxRole::Sent => "\\Sent".to_string(),
            MailboxRole::Trash => "\\Trash".to_string(),
            MailboxRole::Junk => "\\Junk".to_string(),
            MailboxRole::Important => "\\Important".to_string(),
        }
    }

    /// Try to parse from IMAP special-use attribute
    pub fn from_special_use(attr: &str) -> Option<Self> {
        match attr {
            "\\Inbox" => Some(MailboxRole::Inbox),
            "\\Archive" => Some(MailboxRole::Archive),
            "\\Drafts" => Some(MailboxRole::Drafts),
            "\\Sent" => Some(MailboxRole::Sent),
            "\\Trash" => Some(MailboxRole::Trash),
            "\\Junk" => Some(MailboxRole::Junk),
            "\\Important" => Some(MailboxRole::Important),
            _ => None,
        }
    }

    /// Detect role from mailbox name (auto-detection)
    pub fn detect_from_name(name: &str) -> Option<Self> {
        let name_lower = name.to_lowercase();
        match name_lower.as_str() {
            "inbox" => Some(MailboxRole::Inbox),
            "archive" => Some(MailboxRole::Archive),
            "drafts" => Some(MailboxRole::Drafts),
            "sent" => Some(MailboxRole::Sent),
            "sent items" | "sent mail" => Some(MailboxRole::Sent),
            "trash" | "deleted" | "deleted items" => Some(MailboxRole::Trash),
            "junk" | "spam" => Some(MailboxRole::Junk),
            "important" => Some(MailboxRole::Important),
            _ => None,
        }
    }
}

/// Mailbox rights
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxRights {
    pub may_read_items: bool,
    pub may_add_items: bool,
    pub may_remove_items: bool,
    pub may_set_seen: bool,
    pub may_set_keywords: bool,
    pub may_create_child: bool,
    pub may_rename: bool,
    pub may_delete: bool,
    pub may_submit: bool,
}

impl Default for MailboxRights {
    fn default() -> Self {
        Self {
            may_read_items: true,
            may_add_items: true,
            may_remove_items: true,
            may_set_seen: true,
            may_set_keywords: true,
            may_create_child: true,
            may_rename: true,
            may_delete: true,
            may_submit: true,
        }
    }
}

/// Mailbox/get request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxGetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
}

/// Mailbox/get response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxGetResponse {
    pub account_id: String,
    pub state: String,
    pub list: Vec<Mailbox>,
    pub not_found: Vec<String>,
}

/// Mailbox/set request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxSetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_in_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create: Option<HashMap<String, MailboxObject>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroy: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_destroy_remove_emails: Option<bool>,
}

/// Mailbox object for creation
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxObject {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<MailboxRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_subscribed: Option<bool>,
}

/// Mailbox/set response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxSetResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<HashMap<String, Mailbox>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<HashMap<String, Option<Mailbox>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_created: Option<HashMap<String, crate::types::JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_updated: Option<HashMap<String, crate::types::JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_destroyed: Option<HashMap<String, crate::types::JmapSetError>>,
}

/// Mailbox/query request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxQueryRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<MailboxFilterCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<MailboxSort>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculate_total: Option<bool>,
}

/// Mailbox filter condition
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxFilterCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<MailboxRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_any_role: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_subscribed: Option<bool>,
}

/// Mailbox sort comparator
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxSort {
    pub property: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_ascending: Option<bool>,
}

/// Mailbox/query response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxQueryResponse {
    pub account_id: String,
    pub query_state: String,
    pub can_calculate_changes: bool,
    pub position: i64,
    pub ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// Mailbox/changes request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxChangesRequest {
    pub account_id: String,
    pub since_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_changes: Option<u64>,
}

/// Mailbox/changes response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxChangesResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    pub has_more_changes: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}

/// Mailbox/queryChanges request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxQueryChangesRequest {
    pub account_id: String,
    pub since_query_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<MailboxFilterCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<MailboxSort>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_changes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_to_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculate_total: Option<bool>,
}

/// Mailbox/queryChanges response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxQueryChangesResponse {
    pub account_id: String,
    pub old_query_state: String,
    pub new_query_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    pub removed: Vec<String>,
    pub added: Vec<AddedItem>,
}

/// Added item in queryChanges
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedItem {
    pub id: String,
    pub index: u64,
}

// ─── Shared helper functions ───────────────────────────────────────────────

/// Create a default mailbox
pub(crate) fn create_default_mailbox(
    id: &str,
    name: &str,
    role: Option<MailboxRole>,
    sort_order: u32,
) -> Mailbox {
    Mailbox {
        id: id.to_string(),
        name: name.to_string(),
        parent_id: None,
        role,
        sort_order,
        total_emails: 0,
        unread_emails: 0,
        total_threads: 0,
        unread_threads: 0,
        my_rights: MailboxRights::default(),
        is_subscribed: true,
    }
}

/// Get default mailboxes as a HashMap
pub(crate) fn get_default_mailboxes() -> HashMap<String, Mailbox> {
    let mut mailboxes = HashMap::new();

    mailboxes.insert(
        "inbox".to_string(),
        create_default_mailbox("inbox", "Inbox", Some(MailboxRole::Inbox), 0),
    );
    mailboxes.insert(
        "sent".to_string(),
        create_default_mailbox("sent", "Sent", Some(MailboxRole::Sent), 10),
    );
    mailboxes.insert(
        "drafts".to_string(),
        create_default_mailbox("drafts", "Drafts", Some(MailboxRole::Drafts), 20),
    );
    mailboxes.insert(
        "trash".to_string(),
        create_default_mailbox("trash", "Trash", Some(MailboxRole::Trash), 30),
    );
    mailboxes.insert(
        "junk".to_string(),
        create_default_mailbox("junk", "Junk", Some(MailboxRole::Junk), 40),
    );
    mailboxes.insert(
        "archive".to_string(),
        create_default_mailbox("archive", "Archive", Some(MailboxRole::Archive), 50),
    );

    mailboxes
}

/// Generate a state string (timestamp-based)
pub(crate) fn generate_state() -> String {
    chrono::Utc::now().timestamp().to_string()
}

/// Filter mailbox properties based on requested properties
pub(crate) fn filter_mailbox_properties(mailbox: Mailbox, _properties: &[String]) -> Mailbox {
    // In a real implementation, we'd selectively include only requested properties.
    // JMAP spec allows returning more properties than requested.
    mailbox
}

/// Check if a mailbox ID represents a special-use mailbox
pub(crate) fn is_special_use_mailbox(id: &str) -> bool {
    matches!(
        id,
        "inbox" | "sent" | "drafts" | "trash" | "junk" | "archive"
    )
}

/// Validate mailbox name
pub(crate) fn validate_mailbox_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Mailbox name cannot be empty".to_string());
    }
    if name.len() > 255 {
        return Err("Mailbox name too long (max 255 characters)".to_string());
    }
    // Check for invalid characters (hierarchy separator, control characters)
    if name.contains('/') || name.contains('\0') {
        return Err("Mailbox name contains invalid characters".to_string());
    }
    Ok(())
}

/// Check for circular parent reference
pub(crate) fn would_create_cycle(
    parent_id: &str,
    child_id: &str,
    _mailboxes: &HashMap<String, Mailbox>,
) -> bool {
    // Simplified check: in real implementation, would traverse the hierarchy
    parent_id == child_id
}

/// Apply filter to a mailbox
pub(crate) fn apply_mailbox_filter(mailbox: &Mailbox, filter: &MailboxFilterCondition) -> bool {
    // Filter by parent_id
    if let Some(ref parent_id) = filter.parent_id {
        if mailbox.parent_id.as_ref() != Some(parent_id) {
            return false;
        }
    }

    // Filter by name (exact match)
    if let Some(ref name) = filter.name {
        if &mailbox.name != name {
            return false;
        }
    }

    // Filter by role
    if let Some(role) = filter.role {
        if mailbox.role != Some(role) {
            return false;
        }
    }

    // Filter by has_any_role
    if let Some(has_any_role) = filter.has_any_role {
        if has_any_role && mailbox.role.is_none() {
            return false;
        }
        if !has_any_role && mailbox.role.is_some() {
            return false;
        }
    }

    // Filter by is_subscribed
    if let Some(is_subscribed) = filter.is_subscribed {
        if mailbox.is_subscribed != is_subscribed {
            return false;
        }
    }

    true
}

/// Apply sort to mailbox list
pub(crate) fn apply_mailbox_sort(mailboxes: &mut [Mailbox], sort_comparators: &[MailboxSort]) {
    use std::cmp::Ordering;

    mailboxes.sort_by(|a, b| {
        for comparator in sort_comparators {
            let is_ascending = comparator.is_ascending.unwrap_or(true);

            let ordering = match comparator.property.as_str() {
                "sortOrder" => a.sort_order.cmp(&b.sort_order),
                "name" => a.name.cmp(&b.name),
                "totalEmails" => a.total_emails.cmp(&b.total_emails),
                "unreadEmails" => a.unread_emails.cmp(&b.unread_emails),
                "totalThreads" => a.total_threads.cmp(&b.total_threads),
                "unreadThreads" => a.unread_threads.cmp(&b.unread_threads),
                _ => Ordering::Equal,
            };

            let ordering = if is_ascending {
                ordering
            } else {
                ordering.reverse()
            };

            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    });
}
