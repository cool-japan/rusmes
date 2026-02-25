//! JMAP Session Object (RFC 8620 Section 2)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JMAP Session object returned by the session endpoint
/// RFC 8620 Section 2
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// The set of capabilities supported by the server
    pub capabilities: HashMap<String, Capability>,

    /// A map of account ID to account information
    pub accounts: HashMap<String, Account>,

    /// A map of capability URIs to the primary account IDs
    pub primary_accounts: HashMap<String, String>,

    /// The username of the authenticated user
    pub username: String,

    /// The base URL for the JMAP API
    pub api_url: String,

    /// The URL to download blobs
    pub download_url: String,

    /// The URL to upload blobs
    pub upload_url: String,

    /// The URL for event source connections (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_source_url: Option<String>,

    /// Current session state (opaque string)
    pub state: String,
}

/// Capability object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    /// Maximum number of concurrent requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_requests: Option<u32>,

    /// Maximum number of method calls per request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_calls_in_request: Option<u32>,

    /// Maximum size of a request in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size_request: Option<u64>,

    /// Maximum size of a blob upload in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size_upload: Option<u64>,

    /// Maximum number of objects to return in a single get/query call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_objects_in_get: Option<u32>,

    /// Maximum number of objects to return in a single set call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_objects_in_set: Option<u32>,

    /// Collation algorithms supported
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collation_algorithms: Option<Vec<String>>,
}

impl Default for Capability {
    fn default() -> Self {
        Self {
            max_concurrent_requests: Some(4),
            max_calls_in_request: Some(16),
            max_size_request: Some(10 * 1024 * 1024), // 10 MB
            max_size_upload: Some(50 * 1024 * 1024),  // 50 MB
            max_objects_in_get: Some(500),
            max_objects_in_set: Some(500),
            collation_algorithms: Some(vec![
                "i;ascii-numeric".to_string(),
                "i;ascii-casemap".to_string(),
            ]),
        }
    }
}

/// Account object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Display name for the account
    pub name: String,

    /// True if the user is the owner of the account
    pub is_personal: bool,

    /// True if the user has read-only access
    pub is_read_only: bool,

    /// The set of capability URIs supported for this account
    pub account_capabilities: HashMap<String, AccountCapability>,
}

/// Account-specific capability
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCapability {
    /// Maximum number of mailboxes allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_mailboxes_per_email: Option<u32>,

    /// Maximum depth of mailbox hierarchy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_mailbox_depth: Option<u32>,

    /// Maximum size of a single email in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size_mailbox_name: Option<u32>,

    /// Maximum number of emails in a mailbox
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size_attachments_per_email: Option<u64>,

    /// Email submission extensions supported
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_query_sort_options: Option<Vec<String>>,

    /// May upload script (for Sieve)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub may_upload_script: Option<bool>,
}

impl Default for AccountCapability {
    fn default() -> Self {
        Self {
            max_mailboxes_per_email: Some(10),
            max_mailbox_depth: Some(10),
            max_size_mailbox_name: Some(255),
            max_size_attachments_per_email: Some(50 * 1024 * 1024), // 50 MB
            email_query_sort_options: Some(vec![
                "receivedAt".to_string(),
                "from".to_string(),
                "subject".to_string(),
                "size".to_string(),
            ]),
            may_upload_script: Some(false),
        }
    }
}

impl Session {
    /// Create a new session for a user
    pub fn new(username: String, account_id: String, base_url: String) -> Self {
        let mut capabilities = HashMap::new();
        let mut accounts = HashMap::new();
        let mut primary_accounts = HashMap::new();

        // Core capability (RFC 8620)
        let core_cap_uri = "urn:ietf:params:jmap:core".to_string();
        capabilities.insert(core_cap_uri.clone(), Capability::default());

        // Mail capability (RFC 8621)
        let mail_cap_uri = "urn:ietf:params:jmap:mail".to_string();
        capabilities.insert(mail_cap_uri.clone(), Capability::default());

        // Submission capability (RFC 8621)
        let submission_cap_uri = "urn:ietf:params:jmap:submission".to_string();
        capabilities.insert(submission_cap_uri.clone(), Capability::default());

        // Create account capabilities
        let mut account_caps = HashMap::new();
        account_caps.insert(mail_cap_uri.clone(), AccountCapability::default());
        account_caps.insert(submission_cap_uri.clone(), AccountCapability::default());

        // Create the account
        let account = Account {
            name: username.clone(),
            is_personal: true,
            is_read_only: false,
            account_capabilities: account_caps,
        };

        accounts.insert(account_id.clone(), account);

        // Set primary accounts for each capability
        primary_accounts.insert(core_cap_uri, account_id.clone());
        primary_accounts.insert(mail_cap_uri, account_id.clone());
        primary_accounts.insert(submission_cap_uri, account_id);

        Self {
            capabilities,
            accounts,
            primary_accounts,
            username,
            api_url: format!("{}/jmap", base_url),
            download_url: format!("{}/download/{{accountId}}/{{blobId}}/{{name}}", base_url),
            upload_url: format!("{}/upload/{{accountId}}", base_url),
            event_source_url: Some(format!("{}/eventsource", base_url)),
            state: "session-state-1".to_string(),
        }
    }
}
