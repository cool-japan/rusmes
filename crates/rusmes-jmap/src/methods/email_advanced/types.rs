//! Shared request/response types for the `email_advanced` module.

use crate::types::{Email, JmapSetError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Email/changes request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailChangesRequest {
    pub account_id: String,
    pub since_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_changes: Option<u64>,
}

/// Email/changes response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailChangesResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    pub has_more_changes: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}

/// Email/queryChanges request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailQueryChangesRequest {
    pub account_id: String,
    pub since_query_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<crate::types::EmailFilterCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<crate::types::EmailSort>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_changes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_to_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculate_total: Option<bool>,
}

/// Email/queryChanges response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailQueryChangesResponse {
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

/// Email/copy request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailCopyRequest {
    pub from_account_id: String,
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_from_in_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_in_state: Option<String>,
    pub create: HashMap<String, EmailCopyObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_success_destroy_original: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroy_from_if_in_state: Option<String>,
}

/// Email object for copy operation
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailCopyObject {
    pub id: String,
    pub mailbox_ids: HashMap<String, bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<HashMap<String, bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_at: Option<DateTime<Utc>>,
}

/// Email/copy response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailCopyResponse {
    pub from_account_id: String,
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<HashMap<String, Email>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_created: Option<HashMap<String, JmapSetError>>,
}

/// Email/import request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailImportRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_in_state: Option<String>,
    pub emails: HashMap<String, EmailImportObject>,
}

/// Email import object
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailImportObject {
    pub blob_id: String,
    pub mailbox_ids: HashMap<String, bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<HashMap<String, bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_at: Option<DateTime<Utc>>,
}

/// Email/import response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailImportResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<HashMap<String, Email>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_created: Option<HashMap<String, JmapSetError>>,
}

/// Email/parse request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailParseRequest {
    pub account_id: String,
    pub blob_ids: Vec<String>,
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

/// Email/parse response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailParseResponse {
    pub account_id: String,
    pub parsed: HashMap<String, Email>,
    pub not_parsable: Vec<String>,
    pub not_found: Vec<String>,
}
