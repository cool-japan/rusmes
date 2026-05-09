//! JMAP type definitions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JMAP request
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapRequest {
    pub using: Vec<String>,
    pub method_calls: Vec<JmapMethodCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_ids: Option<serde_json::Value>,
}

/// JMAP method call
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JmapMethodCall(pub String, pub serde_json::Value, pub String);

/// JMAP response
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapResponse {
    pub method_responses: Vec<JmapMethodResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_ids: Option<serde_json::Value>,
}

/// JMAP method response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JmapMethodResponse(pub String, pub serde_json::Value, pub String);

/// JMAP methods
#[derive(Debug, Clone)]
pub enum JmapMethod {
    /// Email/get
    EmailGet,
    /// Email/set
    EmailSet,
    /// Email/query
    EmailQuery,
    /// Mailbox/get
    MailboxGet,
    /// Mailbox/set
    MailboxSet,
    /// Mailbox/query
    MailboxQuery,
}

/// JMAP error types as defined in RFC 8620 Section 3.6
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JmapErrorType {
    /// The content type of the request was not "application/json" or the request did not parse as I-JSON.
    NotJson,
    /// The request parsed as JSON but did not match the structure defined in RFC 8620 Section 3.3.
    NotRequest,
    /// The server has a limit on the number of calls in a single request
    Limit,
    /// Unknown capability in "using" property
    UnknownCapability,
    /// Unknown method
    UnknownMethod,
    /// Invalid arguments to method
    InvalidArguments,
    /// Account not found or does not support this data type
    AccountNotFound,
    /// Account not supported by this method
    AccountNotSupportedByMethod,
    /// Account is read-only
    AccountReadOnly,
    /// Server error
    ServerFail,
    /// Server is unavailable
    ServerUnavailable,
    /// Server has a hard limit on the number of objects
    ServerPartialFailure,
    /// The authenticated principal is not permitted to access the requested account.
    ///
    /// Per RFC 8620 §3.6 there is no top-level `forbidden` error code (the closest
    /// standardized concept is the `forbidden` `setError` in §5.3); we use this
    /// dedicated method-level error so that ownership-mismatch responses are
    /// distinguishable from `accountNotFound` (which would also be RFC-defensible
    /// because it does not reveal whether the foreign account exists).
    Forbidden,
}

impl JmapErrorType {
    /// Get the string representation of the error type
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotJson => "urn:ietf:params:jmap:error:notJSON",
            Self::NotRequest => "urn:ietf:params:jmap:error:notRequest",
            Self::Limit => "urn:ietf:params:jmap:error:limit",
            Self::UnknownCapability => "urn:ietf:params:jmap:error:unknownCapability",
            Self::UnknownMethod => "urn:ietf:params:jmap:error:unknownMethod",
            Self::InvalidArguments => "urn:ietf:params:jmap:error:invalidArguments",
            Self::AccountNotFound => "urn:ietf:params:jmap:error:accountNotFound",
            Self::AccountNotSupportedByMethod => {
                "urn:ietf:params:jmap:error:accountNotSupportedByMethod"
            }
            Self::AccountReadOnly => "urn:ietf:params:jmap:error:accountReadOnly",
            Self::ServerFail => "urn:ietf:params:jmap:error:serverFail",
            Self::ServerUnavailable => "urn:ietf:params:jmap:error:serverUnavailable",
            Self::ServerPartialFailure => "urn:ietf:params:jmap:error:serverPartialFailure",
            Self::Forbidden => "urn:ietf:params:jmap:error:forbidden",
        }
    }
}

/// Authenticated principal — attached to every authorized JMAP request by the
/// auth middleware (`crate::auth::JmapAuthLayer`).
///
/// Method handlers receive `&Principal` and use it to enforce that the
/// `accountId` named in each JMAP request belongs to the authenticated
/// caller. See [`Principal::owns_account`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Username of the authenticated user (e.g. `alice@example.com`).
    pub username: String,
    /// Canonical account identifier this principal owns.
    ///
    /// JMAP requests carrying a different `accountId` are rejected with
    /// [`JmapErrorType::Forbidden`].
    pub account_id: String,
    /// Granted scopes (e.g. capability URIs the principal is allowed to use).
    /// An empty set means "all scopes" — refine when scope-based authorization
    /// is wired in (see follow-ups in `TODO.md`).
    pub scopes: Vec<String>,
}

/// Scope value granted to administrative principals — bypasses the per-account
/// ownership check in [`Principal::owns_account`]. Currently used by the
/// in-tree test fixtures that exercise the dispatch layer; production
/// administrative consoles will want it too.
pub const SCOPE_ADMIN: &str = "rusmes:admin:any-account";

/// Build an admin-scoped principal for use in unit tests.
///
/// The returned principal owns every `accountId` because it carries the
/// [`SCOPE_ADMIN`] scope. Test fixtures that want to specifically exercise
/// ownership enforcement should construct their own [`Principal`] manually
/// instead of using this helper.
#[doc(hidden)]
pub fn admin_principal_for_tests() -> Principal {
    Principal {
        username: "test-admin".to_string(),
        account_id: "test-admin-account".to_string(),
        scopes: vec![SCOPE_ADMIN.to_string()],
    }
}

impl Principal {
    /// Build a principal from a username, deriving the canonical `account_id`
    /// the same way the session endpoint does.
    pub fn from_username(username: impl Into<String>) -> Self {
        let username = username.into();
        let account_id = derive_account_id(&username);
        Self {
            username,
            account_id,
            scopes: Vec::new(),
        }
    }

    /// True iff `requested_account_id` equals this principal's owned account
    /// OR this principal has been granted the [`SCOPE_ADMIN`] scope.
    pub fn owns_account(&self, requested_account_id: &str) -> bool {
        self.account_id == requested_account_id || self.scopes.iter().any(|s| s == SCOPE_ADMIN)
    }
}

/// Canonical mapping from a username to the account ID exposed in the JMAP
/// session. Centralized here so the session endpoint, auth middleware and
/// tests all agree on the same scheme.
pub fn derive_account_id(username: &str) -> String {
    format!("account-{}", username.replace('@', "-"))
}

/// JMAP error response
#[derive(Debug, Clone, Serialize)]
pub struct JmapError {
    #[serde(rename = "type")]
    pub error_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
}

impl JmapError {
    /// Create a new JMAP error
    pub fn new(error_type: JmapErrorType) -> Self {
        Self {
            error_type: error_type.as_str().to_string(),
            status: None,
            detail: None,
            limit: None,
        }
    }

    /// Set the status code
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Set the detail message
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Set the limit information
    pub fn with_limit(mut self, limit: impl Into<String>) -> Self {
        self.limit = Some(limit.into());
        self
    }
}

/// RFC 8620 §5.1 PushSubscription — a registered WebPush endpoint that the
/// server notifies whenever the principal's data changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSubscription {
    /// Server-assigned unique identifier.
    pub id: String,

    /// Opaque client-chosen identifier used to deduplicate registrations from
    /// the same device across sessions.
    #[serde(rename = "deviceClientId")]
    pub device_client_id: String,

    /// HTTPS URL of the push endpoint (RFC 8030).
    pub url: String,

    /// Optional Web Crypto key material for encrypted push (RFC 8291).
    /// When `None`, the server sends an unencrypted "tickle" (empty body).
    pub keys: Option<PushKeys>,

    /// Short-lived secret the server sends to the push endpoint for out-of-band
    /// verification.  Held in memory only; never serialized to API responses.
    #[serde(skip)]
    pub verification_code: Option<String>,

    /// RFC 3339 expiry.  When `Some` and in the past the subscription is
    /// silently dropped on the next delivery attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<chrono::DateTime<chrono::Utc>>,

    /// Data-type names (e.g. `"Email"`, `"Mailbox"`) this subscription monitors.
    /// An empty list means "all types".
    pub types: Vec<String>,

    /// Whether the subscription has been verified via the out-of-band
    /// verification push.  Unverified subscriptions are never used for delivery.
    /// Not serialized — internal state only.
    #[serde(skip)]
    pub verified: bool,

    /// The `account_id` of the principal that owns this subscription.
    /// Not serialized — internal state only.
    #[serde(skip)]
    pub principal_id: String,
}

/// Web Crypto key material for RFC 8291 encrypted push.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushKeys {
    /// Base64url-encoded client public key (P-256 uncompressed point).
    pub p256dh: String,
    /// Base64url-encoded 16-byte auth secret.
    pub auth: String,
}

/// Email address in JMAP format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
}

impl EmailAddress {
    /// Create a new email address
    pub fn new(email: String) -> Self {
        Self { name: None, email }
    }

    /// Create a new email address with name
    pub fn with_name(email: String, name: String) -> Self {
        Self {
            name: Some(name),
            email,
        }
    }
}

/// Email object as defined in RFC 8621
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Email {
    /// Unique identifier for the email
    pub id: String,
    /// Blob ID for the raw RFC 5322 message
    pub blob_id: String,
    /// Thread ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Mailbox IDs (map of mailbox ID to boolean)
    pub mailbox_ids: HashMap<String, bool>,
    /// Keywords/flags (e.g., $seen, $flagged, $draft)
    pub keywords: HashMap<String, bool>,
    /// Size in bytes
    pub size: u64,
    /// Time email was received at the server
    pub received_at: DateTime<Utc>,
    /// Message-ID header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Vec<String>>,
    /// In-Reply-To header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<Vec<String>>,
    /// References header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<Vec<String>>,
    /// Sender header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Vec<EmailAddress>>,
    /// From header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Vec<EmailAddress>>,
    /// To header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Vec<EmailAddress>>,
    /// Cc header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<Vec<EmailAddress>>,
    /// Bcc header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc: Option<Vec<EmailAddress>>,
    /// Reply-To header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Vec<EmailAddress>>,
    /// Subject header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Sent-At date from Date header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<DateTime<Utc>>,
    /// Has attachment
    pub has_attachment: bool,
    /// Preview text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Body values (for body parts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_values: Option<HashMap<String, EmailBodyValue>>,
    /// Text body parts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_body: Option<Vec<EmailBodyPart>>,
    /// HTML body parts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_body: Option<Vec<EmailBodyPart>>,
    /// Attachments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<EmailBodyPart>>,
}

/// Email body value
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailBodyValue {
    pub value: String,
    pub is_encoding_problem: bool,
    pub is_truncated: bool,
}

/// Email body part
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailBodyPart {
    pub part_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// Email/get request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailGetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_properties: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch_text_body_values: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch_html_body_values: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch_all_body_values: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_body_value_bytes: Option<u64>,
}

/// Email/get response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailGetResponse {
    pub account_id: String,
    pub state: String,
    pub list: Vec<Email>,
    pub not_found: Vec<String>,
}

/// Email/set request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_in_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create: Option<HashMap<String, EmailSetObject>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroy: Option<Vec<String>>,
}

/// Email object for Email/set create (RFC 8621 §5.2)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSetObject {
    /// Mailboxes to file this message into (mailbox ID → true)
    pub mailbox_ids: HashMap<String, bool>,
    /// Initial keywords/flags
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<HashMap<String, bool>>,
    /// Override for the received-at timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_at: Option<DateTime<Utc>>,
    /// From header addresses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Vec<EmailAddress>>,
    /// To header addresses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Vec<EmailAddress>>,
    /// Cc header addresses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<Vec<EmailAddress>>,
    /// Bcc header addresses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc: Option<Vec<EmailAddress>>,
    /// Reply-To header addresses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Vec<EmailAddress>>,
    /// Sender header addresses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Vec<EmailAddress>>,
    /// Subject header
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Date sent (encoded into the Date header)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<DateTime<Utc>>,
    /// In-Reply-To message ID header values
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<Vec<String>>,
    /// References header values
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<Vec<String>>,
    /// Message-ID header values
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<Vec<String>>,
    /// Body part values (keyed by part ID)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_values: Option<HashMap<String, EmailBodyValue>>,
    /// Ordered text-body part references
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_body: Option<Vec<EmailBodyPart>>,
    /// Ordered HTML-body part references
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_body: Option<Vec<EmailBodyPart>>,
    /// Attachments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<EmailBodyPart>>,
}

/// Email/set response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSetResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<HashMap<String, Email>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<HashMap<String, Option<Email>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_created: Option<HashMap<String, JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_updated: Option<HashMap<String, JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_destroyed: Option<HashMap<String, JmapSetError>>,
}

/// JMAP set error
#[derive(Debug, Clone, Serialize)]
pub struct JmapSetError {
    #[serde(rename = "type")]
    pub error_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Email/query request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailQueryRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<EmailFilterCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<EmailSort>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_offset: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculate_total: Option<bool>,
}

/// Email filter condition
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailFilterCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_mailbox: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_mailbox_other_than: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_in_thread_have_keyword: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub some_in_thread_have_keyword: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub none_in_thread_have_keyword: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_keyword: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_keyword: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_attachment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<String>>,
}

/// Email sort comparator
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSort {
    pub property: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_ascending: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collation: Option<String>,
}

/// Email/query response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailQueryResponse {
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
