//! Mail queue management with retry logic

use crate::queue::priority::Priority;
use dashmap::DashMap;
use rusmes_proto::{Mail, MailId};
use rusmes_storage::StorageBackend;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

/// Serializable queue entry metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEntryData {
    pub mail_id: MailId,
    pub attempts: u32,
    pub max_attempts: u32,
    #[serde(with = "systemtime_serde")]
    pub next_retry: SystemTime,
    pub last_error: Option<String>,
    #[serde(default)]
    pub priority: Priority,
}

/// Queue entry with retry information
#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub mail: Mail,
    pub attempts: u32,
    pub max_attempts: u32,
    pub next_retry: SystemTime,
    pub last_error: Option<String>,
    pub priority: Priority,
}

/// Serialization helper for SystemTime
mod systemtime_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = time
            .duration_since(UNIX_EPOCH)
            .map_err(serde::ser::Error::custom)?;
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(UNIX_EPOCH + Duration::from_secs(secs))
    }
}

impl QueueEntry {
    /// Create a new queue entry with default priority
    pub fn new(mail: Mail) -> Self {
        Self::new_with_priority(mail, Priority::default())
    }

    /// Create a new queue entry with specified priority
    pub fn new_with_priority(mail: Mail, priority: Priority) -> Self {
        Self {
            mail,
            attempts: 0,
            max_attempts: 5,
            next_retry: SystemTime::now(),
            last_error: None,
            priority,
        }
    }

    /// Get the priority of this entry
    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// Set the priority of this entry
    pub fn set_priority(&mut self, priority: Priority) {
        self.priority = priority;
    }

    /// Calculate next retry time with exponential backoff
    pub fn calculate_next_retry(&mut self) {
        let backoff_secs = 2u64.pow(self.attempts.min(10)) * 60; // 1min, 2min, 4min, 8min...
        self.next_retry = SystemTime::now() + Duration::from_secs(backoff_secs);
        self.attempts += 1;
    }

    /// Check if entry should be retried
    pub fn should_retry(&self) -> bool {
        self.attempts < self.max_attempts && SystemTime::now() >= self.next_retry
    }

    /// Check if entry has exceeded max attempts
    pub fn is_bounced(&self) -> bool {
        self.attempts >= self.max_attempts
    }

    /// Get mail ID
    pub fn mail_id(&self) -> &MailId {
        self.mail.id()
    }

    /// Convert to serializable data
    pub fn to_data(&self) -> QueueEntryData {
        QueueEntryData {
            mail_id: *self.mail.id(),
            attempts: self.attempts,
            max_attempts: self.max_attempts,
            next_retry: self.next_retry,
            last_error: self.last_error.clone(),
            priority: self.priority,
        }
    }

    /// Create from data and mail
    pub fn from_data(data: QueueEntryData, mail: Mail) -> Self {
        Self {
            mail,
            attempts: data.attempts,
            max_attempts: data.max_attempts,
            next_retry: data.next_retry,
            last_error: data.last_error,
            priority: data.priority,
        }
    }
}

/// Queue storage operations
#[async_trait::async_trait]
pub trait QueueStore: Send + Sync {
    /// Save a queue entry to persistent storage
    async fn save_entry(&self, entry: &QueueEntry) -> anyhow::Result<()>;

    /// Load a queue entry from persistent storage
    async fn load_entry(&self, mail_id: &MailId) -> anyhow::Result<Option<QueueEntry>>;

    /// Remove a queue entry from persistent storage
    async fn remove_entry(&self, mail_id: &MailId) -> anyhow::Result<()>;

    /// Load all pending queue entries on startup
    async fn load_all_entries(&self) -> anyhow::Result<Vec<QueueEntry>>;

    /// Save to dead letter queue
    async fn save_to_dlq(&self, entry: &QueueEntry) -> anyhow::Result<()>;

    /// List all dead letter queue entries
    async fn list_dlq(&self) -> anyhow::Result<Vec<QueueEntry>>;

    /// Remove from dead letter queue
    async fn remove_from_dlq(&self, mail_id: &MailId) -> anyhow::Result<()>;
}

/// Mail queue with retry logic and priority support
pub struct MailQueue {
    entries: Arc<RwLock<HashMap<MailId, QueueEntry>>>,
    store: Option<Arc<dyn QueueStore>>,
    priority_config: Arc<RwLock<crate::queue::priority::PriorityConfig>>,
    /// Per-recipient-domain message counter (domain → count)
    domain_stats: Arc<DashMap<String, AtomicU64>>,
}

impl MailQueue {
    /// Create a new mail queue without persistent storage
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            store: None,
            priority_config: Arc::new(RwLock::new(
                crate::queue::priority::PriorityConfig::default(),
            )),
            domain_stats: Arc::new(DashMap::new()),
        }
    }

    /// Create a new mail queue with persistent storage
    pub fn new_with_store(store: Arc<dyn QueueStore>) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            store: Some(store),
            priority_config: Arc::new(RwLock::new(
                crate::queue::priority::PriorityConfig::default(),
            )),
            domain_stats: Arc::new(DashMap::new()),
        }
    }

    /// Create a new mail queue with priority configuration
    pub fn new_with_priority_config(
        priority_config: crate::queue::priority::PriorityConfig,
    ) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            store: None,
            priority_config: Arc::new(RwLock::new(priority_config)),
            domain_stats: Arc::new(DashMap::new()),
        }
    }

    /// Create a new mail queue with storage and priority configuration
    pub fn new_with_store_and_priority(
        store: Arc<dyn QueueStore>,
        priority_config: crate::queue::priority::PriorityConfig,
    ) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            store: Some(store),
            priority_config: Arc::new(RwLock::new(priority_config)),
            domain_stats: Arc::new(DashMap::new()),
        }
    }

    /// Update priority configuration
    pub fn update_priority_config(&self, config: crate::queue::priority::PriorityConfig) {
        if let Ok(mut guard) = self.priority_config.write() {
            *guard = config;
        }
    }

    /// Get current priority configuration
    pub fn get_priority_config(&self) -> crate::queue::priority::PriorityConfig {
        self.priority_config
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Get a handle to the domain stats map (for metrics consumers such as rusmes-metrics)
    pub fn domain_stats_map(&self) -> Arc<DashMap<String, AtomicU64>> {
        Arc::clone(&self.domain_stats)
    }

    /// Return a snapshot of per-recipient-domain message counts.
    ///
    /// This is the primary API consumed by `rusmes-metrics` (Cluster 7) for Prometheus exposition.
    pub fn queue_stats_per_domain(&self) -> HashMap<String, u64> {
        self.domain_stats
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().load(Ordering::Relaxed)))
            .collect()
    }

    /// Increment domain counters for all recipient domains in the given mail.
    fn record_domain_stats(&self, mail: &Mail) {
        for recipient in mail.recipients() {
            let domain = recipient.domain().as_str().to_owned();
            if let Some(counter) = self.domain_stats.get(&domain) {
                counter.fetch_add(1, Ordering::Relaxed);
            } else {
                // Use entry API to avoid races
                self.domain_stats
                    .entry(domain)
                    .or_insert_with(|| AtomicU64::new(0))
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Load all pending entries from storage on startup
    pub async fn load_from_storage(&self) -> anyhow::Result<()> {
        if let Some(store) = &self.store {
            let entries = store.load_all_entries().await?;
            let mut queue_entries = self
                .entries
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            for entry in entries {
                let mail_id = *entry.mail.id();
                tracing::info!("Loaded mail {} from storage", mail_id);
                queue_entries.insert(mail_id, entry);
            }
        }
        Ok(())
    }

    /// Enqueue a mail for delivery (atomic operation with persistence)
    pub async fn enqueue(&self, mail: Mail) -> anyhow::Result<()> {
        let mail_id = *mail.id();

        // Calculate priority based on configuration
        let priority = {
            let config = self
                .priority_config
                .read()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            config.calculate_priority(&mail, 0)
        };

        // Record domain statistics before moving mail into entry
        self.record_domain_stats(&mail);

        let entry = QueueEntry::new_with_priority(mail, priority);

        // Save to storage first (if available)
        if let Some(store) = &self.store {
            store.save_entry(&entry).await?;
        }

        // Then add to in-memory queue
        self.entries
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .insert(mail_id, entry);
        tracing::info!(
            "Enqueued mail {} for delivery with priority {}",
            mail_id,
            priority
        );
        Ok(())
    }

    /// Enqueue a mail with explicit priority
    pub async fn enqueue_with_priority(
        &self,
        mail: Mail,
        priority: Priority,
    ) -> anyhow::Result<()> {
        let mail_id = *mail.id();

        // Record domain statistics
        self.record_domain_stats(&mail);

        let entry = QueueEntry::new_with_priority(mail, priority);

        // Save to storage first (if available)
        if let Some(store) = &self.store {
            store.save_entry(&entry).await?;
        }

        // Then add to in-memory queue
        self.entries
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .insert(mail_id, entry);
        tracing::info!(
            "Enqueued mail {} for delivery with priority {}",
            mail_id,
            priority
        );
        Ok(())
    }

    /// Get next batch of mails ready for retry, ordered by priority
    pub fn get_ready_for_retry(&self, limit: usize) -> Vec<QueueEntry> {
        let entries = match self.entries.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut ready: Vec<QueueEntry> = entries
            .values()
            .filter(|e| e.should_retry())
            .cloned()
            .collect();

        // Sort by priority (highest first), then by next_retry time
        ready.sort_by(|a, b| match b.priority.cmp(&a.priority) {
            std::cmp::Ordering::Equal => a.next_retry.cmp(&b.next_retry),
            other => other,
        });

        ready.into_iter().take(limit).collect()
    }

    /// Get mails ready for retry for a specific priority
    pub fn get_ready_for_retry_by_priority(
        &self,
        priority: Priority,
        limit: usize,
    ) -> Vec<QueueEntry> {
        let entries = match self.entries.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries
            .values()
            .filter(|e| e.should_retry() && e.priority == priority)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Mark delivery attempt as failed (atomic operation with persistence)
    pub async fn mark_failed(&self, mail_id: &MailId, error: String) -> anyhow::Result<()> {
        let (should_move_to_dlq, entry_to_save) = {
            let mut entries = self
                .entries
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            if let Some(entry) = entries.get_mut(mail_id) {
                entry.last_error = Some(error.clone());
                entry.calculate_next_retry();

                // Update priority based on retry attempts (priority inheritance/boost)
                let new_priority = {
                    let config = self
                        .priority_config
                        .read()
                        .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
                    if config.inherit_priority_on_retry {
                        config.calculate_priority(&entry.mail, entry.attempts)
                    } else {
                        entry.priority
                    }
                };

                if new_priority != entry.priority {
                    tracing::info!(
                        "Mail {} priority boosted from {} to {} after {} attempts",
                        mail_id,
                        entry.priority,
                        new_priority,
                        entry.attempts
                    );
                    entry.priority = new_priority;
                }

                if entry.is_bounced() {
                    tracing::warn!(
                        "Mail {} exceeded max delivery attempts ({}), moving to DLQ",
                        mail_id,
                        entry.max_attempts
                    );
                    (true, None)
                } else {
                    tracing::info!(
                        "Mail {} delivery failed (attempt {}/{}), priority {}, retry at {:?}",
                        mail_id,
                        entry.attempts,
                        entry.max_attempts,
                        entry.priority,
                        entry.next_retry
                    );
                    (false, Some(entry.clone()))
                }
            } else {
                (false, None)
            }
        }; // Lock dropped here

        // Update in storage (outside lock)
        if let Some(entry) = entry_to_save {
            if let Some(store) = &self.store {
                store.save_entry(&entry).await?;
            }
        }

        // Move to DLQ if bounced
        if should_move_to_dlq {
            self.move_to_dlq(mail_id).await?;
        }

        Ok(())
    }

    /// Move entry to dead letter queue
    async fn move_to_dlq(&self, mail_id: &MailId) -> anyhow::Result<()> {
        let entry = self
            .entries
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .remove(mail_id);

        if let Some(entry) = entry {
            if let Some(store) = &self.store {
                // Save to DLQ
                store.save_to_dlq(&entry).await?;
                // Remove from main queue storage
                store.remove_entry(mail_id).await?;
                tracing::info!("Moved mail {} to dead letter queue", mail_id);
            } else {
                // Without storage, just keep in memory as bounced
                tracing::warn!("Mail {} bounced but no DLQ storage available", mail_id);
            }
        }
        Ok(())
    }

    /// Mark delivery as successful and remove from queue (atomic operation)
    pub async fn mark_delivered(&self, mail_id: &MailId) -> anyhow::Result<()> {
        // Remove from storage first
        if let Some(store) = &self.store {
            store.remove_entry(mail_id).await?;
        }

        // Then remove from in-memory queue
        self.entries
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .remove(mail_id);
        tracing::info!(
            "Mail {} successfully delivered and removed from queue",
            mail_id
        );
        Ok(())
    }

    /// Get all bounced messages (exceeded retry limit) - legacy in-memory only
    pub fn get_bounced(&self) -> Vec<QueueEntry> {
        let entries = match self.entries.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries
            .values()
            .filter(|e| e.is_bounced())
            .cloned()
            .collect()
    }

    /// List all entries in dead letter queue
    pub async fn list_dlq(&self) -> anyhow::Result<Vec<QueueEntry>> {
        if let Some(store) = &self.store {
            store.list_dlq().await
        } else {
            Ok(Vec::new())
        }
    }

    /// Remove a mail from dead letter queue
    pub async fn remove_from_dlq(&self, mail_id: &MailId) -> anyhow::Result<()> {
        if let Some(store) = &self.store {
            store.remove_from_dlq(mail_id).await?;
        }
        Ok(())
    }

    /// Retry a message from dead letter queue
    pub async fn retry_from_dlq(&self, mail_id: &MailId) -> anyhow::Result<()> {
        if let Some(store) = &self.store {
            // Load from DLQ
            let dlq_entries = store.list_dlq().await?;
            if let Some(mut entry) = dlq_entries.into_iter().find(|e| e.mail.id() == mail_id) {
                // Reset retry count
                entry.attempts = 0;
                entry.next_retry = SystemTime::now();
                entry.last_error = None;

                // Save to main queue
                store.save_entry(&entry).await?;
                self.entries
                    .write()
                    .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
                    .insert(*mail_id, entry);

                // Remove from DLQ
                store.remove_from_dlq(mail_id).await?;

                tracing::info!("Retrying mail {} from dead letter queue", mail_id);
            }
        }
        Ok(())
    }

    /// Remove a mail from queue (atomic operation)
    pub async fn remove(&self, mail_id: &MailId) -> anyhow::Result<Option<QueueEntry>> {
        if let Some(store) = &self.store {
            store.remove_entry(mail_id).await?;
        }
        Ok(self
            .entries
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .remove(mail_id))
    }

    /// Get queue statistics
    pub fn stats(&self) -> QueueStats {
        let entries = match self.entries.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let total = entries.len();
        let ready = entries.values().filter(|e| e.should_retry()).count();
        let bounced = entries.values().filter(|e| e.is_bounced()).count();

        QueueStats {
            total,
            ready,
            bounced,
            delayed: total - ready - bounced,
        }
    }

    /// Get statistics for a specific priority level
    pub fn stats_for_priority(&self, priority: Priority) -> QueueStats {
        let entries = match self.entries.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let priority_entries: Vec<_> = entries
            .values()
            .filter(|e| e.priority == priority)
            .collect();

        let total = priority_entries.len();
        let ready = priority_entries.iter().filter(|e| e.should_retry()).count();
        let bounced = priority_entries.iter().filter(|e| e.is_bounced()).count();

        QueueStats {
            total,
            ready,
            bounced,
            delayed: total - ready - bounced,
        }
    }

    /// Get statistics grouped by priority
    pub fn stats_by_priority(&self) -> HashMap<Priority, QueueStats> {
        let mut stats_map = HashMap::new();

        for &priority in Priority::all() {
            stats_map.insert(priority, self.stats_for_priority(priority));
        }

        stats_map
    }

    /// Get all queue entries (for inspection)
    pub fn list_all(&self) -> Vec<QueueEntry> {
        match self.entries.read() {
            Ok(guard) => guard.values().cloned().collect(),
            Err(poisoned) => poisoned.into_inner().values().cloned().collect(),
        }
    }

    /// Get queue entries for a specific priority
    pub fn list_by_priority(&self, priority: Priority) -> Vec<QueueEntry> {
        match self.entries.read() {
            Ok(guard) => guard
                .values()
                .filter(|e| e.priority == priority)
                .cloned()
                .collect(),
            Err(poisoned) => poisoned
                .into_inner()
                .values()
                .filter(|e| e.priority == priority)
                .cloned()
                .collect(),
        }
    }
}

impl Default for MailQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Queue statistics
#[derive(Debug, Clone)]
pub struct QueueStats {
    pub total: usize,
    pub ready: usize,
    pub bounced: usize,
    pub delayed: usize,
}

/// Filesystem-based queue store implementation
pub struct FilesystemQueueStore {
    queue_dir: PathBuf,
    dlq_dir: PathBuf,
    storage: Arc<dyn StorageBackend>,
}

impl FilesystemQueueStore {
    /// Create a new filesystem queue store
    pub fn new(base_path: impl Into<PathBuf>, storage: Arc<dyn StorageBackend>) -> Self {
        let base_path: PathBuf = base_path.into();
        let queue_dir = base_path.join("queue");
        let dlq_dir = base_path.join("dlq");

        Self {
            queue_dir,
            dlq_dir,
            storage,
        }
    }

    /// Ensure directories exist
    async fn ensure_dirs(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.queue_dir).await?;
        tokio::fs::create_dir_all(&self.dlq_dir).await?;
        Ok(())
    }

    /// Get path for queue entry metadata
    fn entry_metadata_path(&self, mail_id: &MailId) -> PathBuf {
        self.queue_dir.join(format!("{}.json", mail_id))
    }

    /// Get path for DLQ entry metadata
    fn dlq_metadata_path(&self, mail_id: &MailId) -> PathBuf {
        self.dlq_dir.join(format!("{}.json", mail_id))
    }
}

#[async_trait::async_trait]
impl QueueStore for FilesystemQueueStore {
    async fn save_entry(&self, entry: &QueueEntry) -> anyhow::Result<()> {
        self.ensure_dirs().await?;

        // Serialize and save metadata
        let data = entry.to_data();
        let json = serde_json::to_string_pretty(&data)?;
        let metadata_path = self.entry_metadata_path(entry.mail_id());
        tokio::fs::write(&metadata_path, json).await?;

        // Save mail to storage (using message store)
        let message_store = self.storage.message_store();
        let mailbox_id = rusmes_storage::MailboxId::new(); // Queue mailbox
        message_store
            .append_message(&mailbox_id, entry.mail.clone())
            .await?;

        Ok(())
    }

    async fn load_entry(&self, mail_id: &MailId) -> anyhow::Result<Option<QueueEntry>> {
        let metadata_path = self.entry_metadata_path(mail_id);

        // Check if file exists
        if !tokio::fs::try_exists(&metadata_path).await? {
            return Ok(None);
        }

        // Load metadata
        let json = tokio::fs::read_to_string(&metadata_path).await?;
        let data: QueueEntryData = serde_json::from_str(&json)?;

        // Load mail from storage
        let message_store = self.storage.message_store();
        let mail_msg_id = rusmes_proto::MessageId::new(); // Would need to store this
        if let Some(mail) = message_store.get_message(&mail_msg_id).await? {
            Ok(Some(QueueEntry::from_data(data, mail)))
        } else {
            Ok(None)
        }
    }

    async fn remove_entry(&self, mail_id: &MailId) -> anyhow::Result<()> {
        let metadata_path = self.entry_metadata_path(mail_id);

        // Remove metadata file
        if tokio::fs::try_exists(&metadata_path).await? {
            tokio::fs::remove_file(&metadata_path).await?;
        }

        // Note: We keep the mail in storage as it might be referenced elsewhere

        Ok(())
    }

    async fn load_all_entries(&self) -> anyhow::Result<Vec<QueueEntry>> {
        self.ensure_dirs().await?;

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&self.queue_dir).await?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                // Load metadata
                let json = tokio::fs::read_to_string(&path).await?;
                if let Ok(data) = serde_json::from_str::<QueueEntryData>(&json) {
                    // Load mail from storage
                    let message_store = self.storage.message_store();
                    let mail_msg_id = rusmes_proto::MessageId::new(); // Would need proper mapping
                    if let Ok(Some(mail)) = message_store.get_message(&mail_msg_id).await {
                        entries.push(QueueEntry::from_data(data, mail));
                    }
                }
            }
        }

        Ok(entries)
    }

    async fn save_to_dlq(&self, entry: &QueueEntry) -> anyhow::Result<()> {
        self.ensure_dirs().await?;

        // Serialize and save metadata to DLQ
        let data = entry.to_data();
        let json = serde_json::to_string_pretty(&data)?;
        let metadata_path = self.dlq_metadata_path(entry.mail_id());
        tokio::fs::write(&metadata_path, json).await?;

        // Mail is already in storage

        Ok(())
    }

    async fn list_dlq(&self) -> anyhow::Result<Vec<QueueEntry>> {
        self.ensure_dirs().await?;

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&self.dlq_dir).await?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                // Load metadata
                let json = tokio::fs::read_to_string(&path).await?;
                if let Ok(data) = serde_json::from_str::<QueueEntryData>(&json) {
                    // Load mail from storage
                    let message_store = self.storage.message_store();
                    let mail_msg_id = rusmes_proto::MessageId::new(); // Would need proper mapping
                    if let Ok(Some(mail)) = message_store.get_message(&mail_msg_id).await {
                        entries.push(QueueEntry::from_data(data, mail));
                    }
                }
            }
        }

        Ok(entries)
    }

    async fn remove_from_dlq(&self, mail_id: &MailId) -> anyhow::Result<()> {
        let metadata_path = self.dlq_metadata_path(mail_id);

        // Remove metadata file
        if tokio::fs::try_exists(&metadata_path).await? {
            tokio::fs::remove_file(&metadata_path).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

    fn make_mail(sender: Option<&str>, recipients: Vec<&str>) -> Mail {
        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));
        Mail::new(
            sender.and_then(|s| s.parse().ok()),
            recipients.iter().filter_map(|r| r.parse().ok()).collect(),
            message,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn test_queue_enqueue_dequeue() {
        let queue = MailQueue::new();
        let mail = make_mail(Some("sender@example.com"), vec!["recipient@example.com"]);

        let mail_id = *mail.id();
        queue.enqueue(mail).await.unwrap();

        let stats = queue.stats();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.ready, 1);

        queue.mark_delivered(&mail_id).await.unwrap();
        let stats = queue.stats();
        assert_eq!(stats.total, 0);
    }

    #[test]
    fn test_retry_backoff() {
        let mut entry = QueueEntry::new(make_mail(None, vec![]));

        entry.calculate_next_retry();
        assert_eq!(entry.attempts, 1);

        entry.calculate_next_retry();
        assert_eq!(entry.attempts, 2);

        // After max attempts, should be bounced
        entry.attempts = 5;
        assert!(entry.is_bounced());
    }

    #[tokio::test]
    async fn queue_priority_ordering() {
        // Enqueue High, Low, Low, Normal → dequeue order: High, Normal, Low, Low
        use crate::queue::priority::{Priority, PriorityQueue};
        use rusmes_proto::MailId;

        let mut queue = PriorityQueue::<&str>::with_default_config();

        queue.enqueue(MailId::new(), "High msg", Priority::High);
        queue.enqueue(MailId::new(), "Low msg 1", Priority::Low);
        queue.enqueue(MailId::new(), "Low msg 2", Priority::Low);
        queue.enqueue(MailId::new(), "Normal msg", Priority::Normal);

        let (_, item1, p1) = queue.dequeue().unwrap();
        assert_eq!(p1, Priority::High, "First dequeued should be High");
        assert_eq!(item1, "High msg");

        let (_, item2, p2) = queue.dequeue().unwrap();
        assert_eq!(p2, Priority::Normal, "Second dequeued should be Normal");
        assert_eq!(item2, "Normal msg");

        let (_, _, p3) = queue.dequeue().unwrap();
        assert_eq!(p3, Priority::Low, "Third dequeued should be Low");

        let (_, _, p4) = queue.dequeue().unwrap();
        assert_eq!(p4, Priority::Low, "Fourth dequeued should be Low");

        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn queue_stats_per_domain_counts() {
        let queue = MailQueue::new();

        // Enqueue 5 messages to example.com
        for _ in 0..5 {
            let mail = make_mail(Some("sender@x.com"), vec!["a@example.com"]);
            queue.enqueue(mail).await.unwrap();
        }

        // Enqueue 3 messages to example.org
        for _ in 0..3 {
            let mail = make_mail(Some("sender@x.com"), vec!["b@example.org"]);
            queue.enqueue(mail).await.unwrap();
        }

        let stats = queue.queue_stats_per_domain();
        assert_eq!(
            stats.get("example.com").copied().unwrap_or(0),
            5,
            "example.com should have 5 messages"
        );
        assert_eq!(
            stats.get("example.org").copied().unwrap_or(0),
            3,
            "example.org should have 3 messages"
        );
    }
}
