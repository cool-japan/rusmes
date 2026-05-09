//! JMAP EmailSubmission public types

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Email submission object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmission {
    /// Unique identifier
    pub id: String,
    /// Identity ID for sender
    pub identity_id: String,
    /// Email ID being submitted
    pub email_id: String,
    /// Thread ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Envelope information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Envelope>,
    /// Send at time (for delayed send)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_at: Option<DateTime<Utc>>,
    /// Undo status
    pub undo_status: UndoStatus,
    /// Delivery status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_status: Option<HashMap<String, DeliveryStatus>>,
    /// DSN blob IDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsn_blob_ids: Option<Vec<String>>,
    /// MDN blob IDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mdn_blob_ids: Option<Vec<String>>,
}

/// Envelope for submission
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub mail_from: Address,
    pub rcpt_to: Vec<Address>,
}

/// Email address for envelope
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, Option<String>>>,
}

/// Undo status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UndoStatus {
    Pending,
    Final,
    Canceled,
}

/// Delivery status for a recipient
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryStatus {
    pub smtp_reply: String,
    pub delivered: DeliveryState,
    pub displayed: DisplayedState,
}

/// Delivery state enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryState {
    Queued,
    Yes,
    No,
    Unknown,
}

/// Displayed state enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayedState {
    Unknown,
    Yes,
    No,
}

/// EmailSubmission/get request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionGetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
}

/// EmailSubmission/get response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionGetResponse {
    pub account_id: String,
    pub state: String,
    pub list: Vec<EmailSubmission>,
    pub not_found: Vec<String>,
}

/// EmailSubmission/set request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionSetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_in_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create: Option<HashMap<String, EmailSubmissionObject>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroy: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_success_update_email: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_success_destroy_email: Option<Vec<String>>,
}

/// EmailSubmission object for creation
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionObject {
    pub identity_id: String,
    pub email_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Envelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_at: Option<DateTime<Utc>>,
}

/// EmailSubmission/set response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionSetResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<HashMap<String, EmailSubmission>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<HashMap<String, Option<EmailSubmission>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_created: Option<HashMap<String, crate::types::JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_updated: Option<HashMap<String, crate::types::JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_destroyed: Option<HashMap<String, crate::types::JmapSetError>>,
}

/// EmailSubmission/query request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionQueryRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<EmailSubmissionFilterCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<EmailSubmissionSort>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculate_total: Option<bool>,
}

/// EmailSubmission filter condition
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionFilterCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_status: Option<UndoStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<DateTime<Utc>>,
}

/// EmailSubmission sort comparator
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionSort {
    pub property: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_ascending: Option<bool>,
}

/// EmailSubmission/query response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionQueryResponse {
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

/// EmailSubmission/changes request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionChangesRequest {
    pub account_id: String,
    pub since_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_changes: Option<u64>,
}

/// EmailSubmission/changes response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionChangesResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    pub has_more_changes: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}
