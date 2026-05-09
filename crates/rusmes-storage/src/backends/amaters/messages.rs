//! AmateRS message store implementation.
//!
//! # Bug fixes (2026-05-05)
//!
//! 1. **Body serialization**: the blob now stores the full RFC 822 form
//!    (serialized headers + `\r\n` separator + body bytes) so `get_message`
//!    can reconstruct a faithful `Mail` via `MimeMessage::parse_from_bytes`.
//!    The previous stub stored `vec![]`, silently losing every message body.
//!
//! 2. **UID allocation** (Strategy B — in-process mutex, no CAS in SDK v0.2):
//!    Each mailbox gets a dedicated `Arc<Mutex<u32>>` entry in `uid_locks`.
//!    The mutex is held across the **entire** append operation — UID reservation,
//!    blob write, metadata write, and counter update — so two concurrent appends
//!    to the same mailbox cannot corrupt each other's counter or steal a UID.
//!
//! 3. **Mailbox counters**: `append_message` and `delete_messages` update
//!    the `counters:{mailbox_id}` key so `MetadataStore::get_mailbox_counters`
//!    returns real data instead of zeros.
//!
//! # Concurrency model
//!
//! The outer `uid_locks` map is locked only long enough to get-or-create the
//! per-mailbox `Arc<Mutex<u32>>`.  The inner per-mailbox mutex is then held for
//! the full duration of `append_message` (UID reservation → blob write →
//! metadata write → counter read-modify-write).  This prevents:
//!
//! - UID reuse (two tasks cannot both read the same `nextuid`)
//! - Counter corruption (two tasks cannot both do `exists += 1` and write the
//!   same result)
//!
//! `delete_messages` acquires the per-mailbox mutex once per affected mailbox
//! before doing the counter decrement, for the same reason.

use super::client::AmatersClient;
use super::records::{MessageBlob, MessageRecord};
use super::UidLockMap;
use crate::traits::MessageStore;
use crate::types::{MailboxCounters, MailboxId, MessageFlags, MessageMetadata, SearchCriteria};
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress, MessageId, MimeMessage};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// RFC 822 serialization helpers
// ---------------------------------------------------------------------------

/// Serialize a `MimeMessage` to its full RFC 822 wire form:
/// `Name: Value\r\n` for every header, `\r\n` separator, then body bytes.
///
/// This is intentionally simple and lossless for storage purposes.  The
/// header values stored in `HeaderMap` are already unfolded; re-folding is
/// not needed for our key–value store.
async fn mime_to_rfc822(mime: &MimeMessage) -> anyhow::Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();

    for (name, values) in mime.headers().iter() {
        for value in values {
            buf.extend_from_slice(name.as_bytes());
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(value.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
    }

    // Header / body separator.
    buf.extend_from_slice(b"\r\n");

    // Body bytes.
    let body_bytes: Vec<u8> = match mime.body() {
        rusmes_proto::MessageBody::Small(bytes) => bytes.to_vec(),
        rusmes_proto::MessageBody::Large(large) => large
            .read_to_bytes()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read large body: {e}"))?
            .to_vec(),
    };
    buf.extend_from_slice(&body_bytes);

    Ok(buf)
}

// ---------------------------------------------------------------------------
// Counter key helper
// ---------------------------------------------------------------------------

/// Key used to store `MailboxCounters` JSON for a mailbox.
fn counters_key(mailbox_id: &MailboxId) -> String {
    format!("counters:{}", mailbox_id)
}

/// Read the persisted `MailboxCounters` from AmateRS, defaulting to zeros.
async fn read_counters(
    client: &AmatersClient,
    keyspace: &str,
    mailbox_id: &MailboxId,
) -> anyhow::Result<MailboxCounters> {
    let key = counters_key(mailbox_id);
    match client.get(keyspace, &key).await? {
        Some(data) => Ok(serde_json::from_slice(&data)?),
        None => Ok(MailboxCounters::default()),
    }
}

/// Write `MailboxCounters` back to AmateRS.
async fn write_counters(
    client: &AmatersClient,
    keyspace: &str,
    mailbox_id: &MailboxId,
    counters: &MailboxCounters,
) -> anyhow::Result<()> {
    let key = counters_key(mailbox_id);
    let data = serde_json::to_vec(counters)?;
    client.put(keyspace, key, data).await
}

// ---------------------------------------------------------------------------
// Per-mailbox mutex helpers
// ---------------------------------------------------------------------------

/// Key used to persist the next-UID counter for a mailbox.
fn nextuid_key(mailbox_id: &MailboxId) -> String {
    format!("nextuid:{}", mailbox_id)
}

/// Get-or-create the per-mailbox `Arc<Mutex<u32>>` from the shared map.
///
/// The outer map lock is held only for this brief insertion/lookup.  Callers
/// then lock the returned per-mailbox mutex for the full duration of their
/// operation.
async fn get_or_create_mailbox_mutex(
    uid_locks: &UidLockMap,
    mailbox_id: &MailboxId,
) -> Arc<Mutex<u32>> {
    let mut map = uid_locks.lock().await;
    map.entry(mailbox_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(1u32)))
        .clone()
}

/// Synchronise the in-process `nextuid_guard` value with the persisted AmateRS
/// value, then allocate and persist the next UID.
///
/// # Precondition
/// The caller must already hold the per-mailbox mutex (i.e., hold a lock guard
/// wrapping `nextuid_guard`).  This function only touches AmateRS — it does
/// **not** acquire the mutex itself, so it can be called while the guard is
/// live without deadlocking.
///
/// # Persistence guarantee
/// The new `nextuid` is persisted to AmateRS *before* this function returns.
/// If the caller subsequently fails to write the message blob, the UID is
/// "wasted" but never reused — which is the safer invariant for IMAP.
async fn sync_and_advance_uid(
    client: &AmatersClient,
    keyspace: &str,
    mailbox_id: &MailboxId,
    nextuid_guard: &mut u32,
) -> anyhow::Result<u32> {
    // Sync in-process counter with persisted value (handles process restart).
    let key = nextuid_key(mailbox_id);
    if let Some(bytes) = client.get(keyspace, &key).await? {
        if bytes.len() >= 4 {
            let arr: [u8; 4] = bytes[..4]
                .try_into()
                .map_err(|_| anyhow::anyhow!("nextuid key has invalid length"))?;
            let stored = u32::from_be_bytes(arr);
            if stored > *nextuid_guard {
                *nextuid_guard = stored;
            }
        }
    }

    // Allocate and advance.
    let uid = *nextuid_guard;
    *nextuid_guard = uid
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("UID counter overflow for mailbox {}", mailbox_id))?;

    // Persist the new nextuid BEFORE returning (crash-safe reservation).
    let new_nextuid = *nextuid_guard;
    client
        .put(keyspace, key, new_nextuid.to_be_bytes().to_vec())
        .await?;

    Ok(uid)
}

// ---------------------------------------------------------------------------
// AmatersMessageStore
// ---------------------------------------------------------------------------

/// AmateRS message store with blob separation and real UID allocation.
pub(super) struct AmatersMessageStore {
    pub(super) client: Arc<AmatersClient>,
    pub(super) metadata_keyspace: String,
    pub(super) blob_keyspace: String,
    /// Per-mailbox next-UID mutexes (Strategy B).
    pub(super) uid_locks: UidLockMap,
}

#[async_trait]
impl MessageStore for AmatersMessageStore {
    async fn append_message(
        &self,
        mailbox_id: &MailboxId,
        message: Mail,
    ) -> anyhow::Result<MessageMetadata> {
        let message_id = *message.message_id();
        let message_size = message.size();

        // Serialize the full RFC 822 wire form before acquiring the lock,
        // since `read_to_bytes()` on a Large body is one-shot and must not
        // be called while holding the mutex (it's I/O but safe here).
        let rfc822_bytes = mime_to_rfc822(message.message()).await?;

        // Acquire the per-mailbox mutex.  Hold it for the entire append so
        // that concurrent appends to the same mailbox see strictly
        // serialised UID allocation AND counter updates.
        let per_mailbox_mutex = get_or_create_mailbox_mutex(&self.uid_locks, mailbox_id).await;
        let mut nextuid_guard = per_mailbox_mutex.lock().await;

        // Allocate UID — syncs with AmateRS and persists the advanced counter.
        let uid = sync_and_advance_uid(
            &self.client,
            &self.metadata_keyspace,
            mailbox_id,
            &mut nextuid_guard,
        )
        .await?;

        // Store the blob.
        let blob = MessageBlob {
            message_id: message_id.to_string(),
            body: rfc822_bytes,
            compressed: false,
        };
        let blob_key = format!("blob:{}", message_id);
        let blob_value = serde_json::to_vec(&blob)?;
        self.client
            .put(&self.blob_keyspace, blob_key.clone(), blob_value)
            .await?;

        // Store message metadata.
        let record = MessageRecord {
            id: message_id.to_string(),
            mailbox_id: mailbox_id.to_string(),
            uid,
            sender: message.sender().map(|s| s.to_string()),
            recipients: message.recipients().iter().map(|r| r.to_string()).collect(),
            headers: HashMap::new(),
            size: message_size,
            blob_key,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        };
        let metadata_key = format!("message:{}", message_id);
        let metadata_value = serde_json::to_vec(&record)?;
        self.client
            .put(&self.metadata_keyspace, metadata_key, metadata_value)
            .await?;

        // Index by mailbox so list/search can enumerate messages.
        let mailbox_index_key = format!("mailbox:{}:message:{}", mailbox_id, message_id);
        self.client
            .put(&self.metadata_keyspace, mailbox_index_key, vec![])
            .await?;

        // Update mailbox counters — under the same per-mailbox mutex so the
        // RMW is serialised with concurrent appends.
        let mut counters = read_counters(&self.client, &self.metadata_keyspace, mailbox_id).await?;
        counters.exists = counters.exists.saturating_add(1);
        counters.recent = counters.recent.saturating_add(1);
        counters.unseen = counters.unseen.saturating_add(1);
        write_counters(&self.client, &self.metadata_keyspace, mailbox_id, &counters).await?;

        // Release the per-mailbox mutex.
        drop(nextuid_guard);

        let mut flags = MessageFlags::new();
        flags.set_recent(true);

        let metadata = MessageMetadata::new(message_id, *mailbox_id, uid, flags, message_size);

        Ok(metadata)
    }

    async fn get_message(&self, message_id: &MessageId) -> anyhow::Result<Option<Mail>> {
        // Fetch metadata record.
        let metadata_key = format!("message:{}", message_id);
        let record_bytes = match self
            .client
            .get(&self.metadata_keyspace, &metadata_key)
            .await?
        {
            Some(b) => b,
            None => return Ok(None),
        };
        let record: MessageRecord = serde_json::from_slice(&record_bytes)?;

        // Fetch blob.
        let blob_bytes = match self
            .client
            .get(&self.blob_keyspace, &record.blob_key)
            .await?
        {
            Some(b) => b,
            None => {
                tracing::warn!(
                    "Blob {} for message {} not found",
                    record.blob_key,
                    message_id
                );
                return Ok(None);
            }
        };
        let blob: MessageBlob = serde_json::from_slice(&blob_bytes)?;

        // Reconstruct MimeMessage from stored RFC 822 bytes.
        let mime = MimeMessage::parse_from_bytes(&blob.body)
            .map_err(|e| anyhow::anyhow!("Failed to parse stored RFC 822 blob: {e}"))?;

        // Reconstruct envelope fields from stored record.
        let sender: Option<MailAddress> = record
            .sender
            .as_deref()
            .map(|s| {
                s.parse::<MailAddress>()
                    .map_err(|e| anyhow::anyhow!("Invalid stored sender '{}': {}", s, e))
            })
            .transpose()?;

        let recipients: Vec<MailAddress> = record
            .recipients
            .iter()
            .map(|r| {
                r.parse::<MailAddress>()
                    .map_err(|e| anyhow::anyhow!("Invalid stored recipient '{}': {}", r, e))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let mail = Mail::with_message_id(sender, recipients, mime, None, None, *message_id);
        Ok(Some(mail))
    }

    async fn delete_messages(&self, message_ids: &[MessageId]) -> anyhow::Result<()> {
        // Group by mailbox so we can update each mailbox's counter once.
        // We accumulate the full list of (mailbox_id, record) pairs first,
        // then acquire each per-mailbox mutex in turn to do the counter RMW.
        let mut mailbox_deletes: HashMap<String, u32> = HashMap::new();

        for message_id in message_ids {
            let key = format!("message:{}", message_id);

            if let Some(data) = self.client.get(&self.metadata_keyspace, &key).await? {
                if let Ok(record) = serde_json::from_slice::<MessageRecord>(&data) {
                    // Track how many messages are removed per mailbox.
                    *mailbox_deletes
                        .entry(record.mailbox_id.clone())
                        .or_insert(0) += 1;

                    self.client
                        .delete(&self.blob_keyspace, &record.blob_key)
                        .await?;

                    // Remove mailbox index entry.
                    let index_key = format!("mailbox:{}:message:{}", record.mailbox_id, message_id);
                    self.client
                        .delete(&self.metadata_keyspace, &index_key)
                        .await?;
                }
            }

            self.client.delete(&self.metadata_keyspace, &key).await?;
        }

        // Decrement counters under each mailbox's per-mailbox mutex to
        // serialise against concurrent appends.
        for (mailbox_id_str, count) in mailbox_deletes {
            if let Ok(uuid) = uuid::Uuid::parse_str(&mailbox_id_str) {
                let mailbox_id = MailboxId::from_uuid(uuid);

                let per_mailbox_mutex =
                    get_or_create_mailbox_mutex(&self.uid_locks, &mailbox_id).await;
                let _guard = per_mailbox_mutex.lock().await;

                let mut counters =
                    read_counters(&self.client, &self.metadata_keyspace, &mailbox_id).await?;
                counters.exists = counters.exists.saturating_sub(count);
                // recent/unseen may already be 0 if flags were cleared; saturating_sub is safe.
                write_counters(
                    &self.client,
                    &self.metadata_keyspace,
                    &mailbox_id,
                    &counters,
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn set_flags(
        &self,
        message_ids: &[MessageId],
        flags: MessageFlags,
    ) -> anyhow::Result<()> {
        for message_id in message_ids {
            let key = format!("flags:{}", message_id);
            let value = serde_json::to_vec(&flags)?;
            self.client.put(&self.metadata_keyspace, key, value).await?;
        }
        Ok(())
    }

    async fn search(
        &self,
        mailbox_id: &MailboxId,
        _criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>> {
        let prefix = format!("mailbox:{}:message:", mailbox_id);
        let keys = self
            .client
            .list_prefix(&self.metadata_keyspace, &prefix)
            .await?;

        let message_ids = keys
            .into_iter()
            .filter_map(|k| {
                k.strip_prefix(&prefix)
                    .and_then(|id_str| uuid::Uuid::parse_str(id_str).ok().map(MessageId::from_uuid))
            })
            .collect();

        Ok(message_ids)
    }

    async fn copy_messages(
        &self,
        message_ids: &[MessageId],
        dest_mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        let mut metadata_list = Vec::new();

        for message_id in message_ids {
            if let Some(message) = self.get_message(message_id).await? {
                let metadata = self.append_message(dest_mailbox_id, message).await?;
                metadata_list.push(metadata);
            }
        }

        Ok(metadata_list)
    }

    async fn get_mailbox_messages(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        let prefix = format!("mailbox:{}:message:", mailbox_id);
        let keys = self
            .client
            .list_prefix(&self.metadata_keyspace, &prefix)
            .await?;

        let mut metadata_list = Vec::new();
        for key in keys {
            if let Some(id_str) = key.strip_prefix(&prefix) {
                if let Ok(uuid) = uuid::Uuid::parse_str(id_str) {
                    let message_id = MessageId::from_uuid(uuid);
                    let metadata_key = format!("message:{}", message_id);

                    if let Some(data) = self
                        .client
                        .get(&self.metadata_keyspace, &metadata_key)
                        .await?
                    {
                        if let Ok(record) = serde_json::from_slice::<MessageRecord>(&data) {
                            let metadata = MessageMetadata::new(
                                message_id,
                                *mailbox_id,
                                record.uid,
                                MessageFlags::new(),
                                record.size,
                            );
                            metadata_list.push(metadata);
                        }
                    }
                }
            }
        }

        Ok(metadata_list)
    }
}
