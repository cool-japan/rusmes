//! Pure data types for AmateRS serializable records.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Serializable mailbox metadata for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MailboxRecord {
    pub(super) id: String,
    pub(super) username: String,
    pub(super) path: Vec<String>,
    pub(super) uid_validity: u32,
    pub(super) uid_next: u32,
    pub(super) special_use: Option<String>,
    pub(super) created_at: i64,
}

/// Serializable message metadata for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MessageRecord {
    pub(super) id: String,
    pub(super) mailbox_id: String,
    pub(super) uid: u32,
    pub(super) sender: Option<String>,
    pub(super) recipients: Vec<String>,
    pub(super) headers: HashMap<String, String>,
    pub(super) size: usize,
    pub(super) blob_key: String,
    pub(super) created_at: i64,
}

/// Message blob stored separately
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MessageBlob {
    pub(super) message_id: String,
    pub(super) body: Vec<u8>,
    pub(super) compressed: bool,
}
