//! Filesystem-based storage backend using maildir format

pub mod backup;
pub mod compaction;
pub mod events;
pub mod locking;
mod message_helpers;
mod thread_ops;
pub mod threading;

use crate::traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria,
};
use crate::StorageEvent;
use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId, Username};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use threading::ThreadingEngine;
use tokio::sync::{broadcast, Mutex as TokioMutex};

/// Filesystem storage backend
pub struct FilesystemBackend {
    base_path: PathBuf,
    mailboxes: Arc<RwLock<HashMap<MailboxId, Mailbox>>>,
    messages: Arc<RwLock<HashMap<MessageId, Mail>>>,
    quotas: Arc<RwLock<HashMap<Username, Quota>>>,
    subscriptions: Arc<RwLock<HashMap<Username, HashSet<String>>>>,
    hostname: String,
    pid: u32,
    delivery_counter: Arc<RwLock<u64>>,
    /// Broadcast sender for storage events (capacity = 256).
    event_tx: broadcast::Sender<StorageEvent>,
    /// Per-MailboxId in-process mutex map.
    ///
    /// Serialises concurrent in-process operations on the same mailbox at the
    /// Tokio level BEFORE the filesystem lock is attempted.  This prevents all
    /// N in-process tasks from racing to acquire the same `fs2` file lock
    /// simultaneously and exhausting the retry budget.
    mailbox_locks: Arc<TokioMutex<HashMap<MailboxId, Arc<TokioMutex<()>>>>>,
}

impl FilesystemBackend {
    /// Create a new filesystem backend
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "localhost".to_string());

        let pid = std::process::id();

        let base_path_buf = base_path.into();

        // Load existing mailbox metadata from disk
        let mailboxes = Self::load_all_mailbox_metadata(&base_path_buf).unwrap_or_else(|e| {
            tracing::warn!("Failed to load mailbox metadata: {}", e);
            HashMap::new()
        });

        let (event_tx, _) = events::new_event_channel();

        Self {
            base_path: base_path_buf,
            mailboxes: Arc::new(RwLock::new(mailboxes)),
            messages: Arc::new(RwLock::new(HashMap::new())),
            quotas: Arc::new(RwLock::new(HashMap::new())),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            hostname,
            pid,
            delivery_counter: Arc::new(RwLock::new(0)),
            event_tx,
            mailbox_locks: Arc::new(TokioMutex::new(HashMap::new())),
        }
    }

    /// Subscribe to storage events from this backend.
    pub fn event_stream_direct(&self) -> broadcast::Receiver<StorageEvent> {
        self.event_tx.subscribe()
    }

    /// Return the base path of the filesystem backend (used by compaction).
    pub fn base_path(&self) -> &std::path::Path {
        &self.base_path
    }

    /// Get the path for a mailbox
    #[allow(dead_code)]
    fn mailbox_path(&self, mailbox_id: &MailboxId) -> PathBuf {
        self.base_path
            .join("mailboxes")
            .join(mailbox_id.to_string())
    }

    /// Get the metadata file path for a user
    fn metadata_file_path(base_path: &std::path::Path, user: &Username) -> PathBuf {
        base_path
            .join("users")
            .join(user.as_str())
            .join("mailboxes.json")
    }

    /// Load mailbox metadata for a specific user from disk
    fn load_user_mailbox_metadata(
        base_path: &std::path::Path,
        user: &Username,
    ) -> anyhow::Result<HashMap<MailboxId, Mailbox>> {
        let metadata_file = Self::metadata_file_path(base_path, user);

        if !metadata_file.exists() {
            return Ok(HashMap::new());
        }

        let content = std::fs::read_to_string(&metadata_file)?;
        let mailboxes: Vec<Mailbox> = serde_json::from_str(&content)?;

        let mut map = HashMap::new();
        for mailbox in mailboxes {
            map.insert(*mailbox.id(), mailbox);
        }

        tracing::info!("Loaded {} mailboxes for user {}", map.len(), user);
        Ok(map)
    }

    /// Load all mailbox metadata from disk (all users)
    fn load_all_mailbox_metadata(
        base_path: &std::path::Path,
    ) -> anyhow::Result<HashMap<MailboxId, Mailbox>> {
        let users_dir = base_path.join("users");

        if !users_dir.exists() {
            return Ok(HashMap::new());
        }

        let mut all_mailboxes = HashMap::new();

        // Iterate through all user directories
        for entry in std::fs::read_dir(&users_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                if let Some(username) = path.file_name().and_then(|n| n.to_str()) {
                    if let Ok(user) = username.parse::<Username>() {
                        match Self::load_user_mailbox_metadata(base_path, &user) {
                            Ok(mailboxes) => {
                                all_mailboxes.extend(mailboxes);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to load mailboxes for user {}: {}", user, e);
                            }
                        }
                    }
                }
            }
        }

        tracing::info!("Loaded {} total mailboxes from disk", all_mailboxes.len());
        Ok(all_mailboxes)
    }

    /// Save mailbox metadata for a specific user to disk
    fn save_user_mailbox_metadata(
        base_path: &std::path::Path,
        user: &Username,
        mailboxes: &[Mailbox],
    ) -> anyhow::Result<()> {
        let metadata_file = Self::metadata_file_path(base_path, user);

        // Create parent directory if it doesn't exist
        if let Some(parent) = metadata_file.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Serialize mailboxes to JSON
        let json = serde_json::to_string_pretty(mailboxes)?;

        // Write to disk
        std::fs::write(&metadata_file, json)?;

        tracing::debug!("Saved {} mailboxes for user {}", mailboxes.len(), user);
        Ok(())
    }
}

/// Maildir filename utilities
struct MaildirFilename;

impl MaildirFilename {
    /// Generate a unique maildir filename
    /// Format: timestamp.M<microseconds>P<pid>Q<counter>.<hostname>
    fn generate(hostname: &str, pid: u32, counter: u64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let micros = now.subsec_micros();

        format!("{}.M{}P{}Q{}.{}", secs, micros, pid, counter, hostname)
    }

    /// Encode flags into maildir format
    /// Format: :2,<flags> where flags are sorted: DFPRST (and 'd' for draft in some implementations)
    /// D=Draft, F=Flagged, P=Passed, R=Replied, S=Seen, T=Trashed
    fn encode_flags(flags: &MessageFlags) -> String {
        let mut flag_chars = String::new();

        if flags.is_draft() {
            flag_chars.push('D');
        }
        if flags.is_flagged() {
            flag_chars.push('F');
        }
        // P (Passed) - forwarded/redirected - not in standard IMAP flags
        if flags.is_answered() {
            flag_chars.push('R');
        }
        if flags.is_seen() {
            flag_chars.push('S');
        }
        if flags.is_deleted() {
            flag_chars.push('T');
        }

        if flag_chars.is_empty() {
            ":2,".to_string()
        } else {
            format!(":2,{}", flag_chars)
        }
    }

    /// Decode flags from maildir filename
    fn decode_flags(filename: &str) -> MessageFlags {
        let mut flags = MessageFlags::new();

        if let Some(flag_part) = filename.split(":2,").nth(1) {
            for ch in flag_part.chars() {
                match ch {
                    'D' => flags.set_draft(true),
                    'F' => flags.set_flagged(true),
                    'R' => flags.set_answered(true),
                    'S' => flags.set_seen(true),
                    'T' => flags.set_deleted(true),
                    _ => {}
                }
            }
        }

        flags
    }

    /// Extract the base filename without flags
    fn base_name(filename: &str) -> &str {
        filename.split(":2,").next().unwrap_or(filename)
    }

    /// Update flags in a filename
    fn with_flags(filename: &str, flags: &MessageFlags) -> String {
        let base = Self::base_name(filename);
        format!("{}{}", base, Self::encode_flags(flags))
    }
}

#[async_trait]
impl StorageBackend for FilesystemBackend {
    fn mailbox_store(&self) -> Arc<dyn MailboxStore> {
        Arc::new(FilesystemMailboxStore {
            base_path: self.base_path.clone(),
            mailboxes: self.mailboxes.clone(),
            subscriptions: self.subscriptions.clone(),
        })
    }

    fn message_store(&self) -> Arc<dyn MessageStore> {
        Arc::new(FilesystemMessageStore {
            base_path: self.base_path.clone(),
            messages: self.messages.clone(),
            mailboxes: self.mailboxes.clone(),
            quotas: self.quotas.clone(),
            hostname: self.hostname.clone(),
            pid: self.pid,
            delivery_counter: self.delivery_counter.clone(),
            event_tx: self.event_tx.clone(),
            mailbox_locks: self.mailbox_locks.clone(),
        })
    }

    fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        Arc::new(FilesystemMetadataStore {
            base_path: self.base_path.clone(),
            quotas: self.quotas.clone(),
        })
    }

    fn event_stream(&self) -> broadcast::Receiver<StorageEvent> {
        self.event_tx.subscribe()
    }

    async fn compact_expunged(&self, older_than: std::time::Duration) -> anyhow::Result<usize> {
        compaction::compact_trash(&self.base_path, older_than).await
    }

    fn as_filesystem_path(&self) -> Option<&std::path::Path> {
        Some(&self.base_path)
    }

    async fn list_all_users(&self) -> anyhow::Result<Vec<Username>> {
        let users_dir = self.base_path.join("users");
        if !tokio::fs::try_exists(&users_dir).await.unwrap_or(false) {
            // Fall back to enumerating users from in-memory mailbox metadata.
            let mailboxes = self
                .mailboxes
                .read()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            let mut seen: HashSet<Username> = HashSet::new();
            for mailbox in mailboxes.values() {
                seen.insert(mailbox.path().user().clone());
            }
            return Ok(seen.into_iter().collect());
        }

        let mut users = Vec::new();
        let mut entries = tokio::fs::read_dir(&users_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = match entry.file_type().await {
                Ok(ft) => ft,
                Err(e) => {
                    tracing::debug!("skipping unreadable entry under users/: {}", e);
                    continue;
                }
            };
            if !file_type.is_dir() {
                continue;
            }
            let name = match entry.file_name().into_string() {
                Ok(n) => n,
                Err(_) => {
                    tracing::debug!("skipping non-UTF8 user directory under users/");
                    continue;
                }
            };
            match Username::new(name) {
                Ok(u) => users.push(u),
                Err(e) => {
                    tracing::debug!("skipping invalid username under users/: {}", e);
                }
            }
        }
        Ok(users)
    }
}

/// Filesystem mailbox store
struct FilesystemMailboxStore {
    base_path: PathBuf,
    mailboxes: Arc<RwLock<HashMap<MailboxId, Mailbox>>>,
    subscriptions: Arc<RwLock<HashMap<Username, HashSet<String>>>>,
}

impl FilesystemMailboxStore {
    /// Persist mailbox metadata for a user to disk
    fn persist_user_mailboxes(&self, user: &Username) -> anyhow::Result<()> {
        let mailboxes = self
            .mailboxes
            .read()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
        let user_mailboxes: Vec<Mailbox> = mailboxes
            .values()
            .filter(|m| m.path().user() == user)
            .cloned()
            .collect();

        FilesystemBackend::save_user_mailbox_metadata(&self.base_path, user, &user_mailboxes)
    }
}

#[async_trait]
impl MailboxStore for FilesystemMailboxStore {
    async fn create_mailbox(&self, path: &MailboxPath) -> anyhow::Result<MailboxId> {
        let mailbox = Mailbox::new(path.clone());
        let id = *mailbox.id();
        let user = path.user().clone();

        // Create directory
        let mailbox_dir = self.base_path.join("mailboxes").join(id.to_string());
        tokio::fs::create_dir_all(&mailbox_dir).await?;

        // Create maildir subdirectories
        tokio::fs::create_dir_all(mailbox_dir.join("cur")).await?;
        tokio::fs::create_dir_all(mailbox_dir.join("new")).await?;
        tokio::fs::create_dir_all(mailbox_dir.join("tmp")).await?;

        // Store in memory
        self.mailboxes
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .insert(id, mailbox);

        // Persist metadata to disk
        self.persist_user_mailboxes(&user)?;

        Ok(id)
    }

    async fn delete_mailbox(&self, id: &MailboxId) -> anyhow::Result<()> {
        // Get user before deleting from memory
        let user = {
            let mailboxes = self
                .mailboxes
                .read()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            mailboxes
                .get(id)
                .map(|m| m.path().user().clone())
                .ok_or_else(|| anyhow::anyhow!("Mailbox not found"))?
        };

        let mailbox_dir = self.base_path.join("mailboxes").join(id.to_string());
        tokio::fs::remove_dir_all(mailbox_dir).await?;

        self.mailboxes
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .remove(id);

        // Persist updated metadata to disk
        self.persist_user_mailboxes(&user)?;

        Ok(())
    }

    async fn rename_mailbox(&self, id: &MailboxId, new_path: &MailboxPath) -> anyhow::Result<()> {
        let user = new_path.user().clone();

        let mut mailboxes = self
            .mailboxes
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
        if let Some(mailbox) = mailboxes.get_mut(id) {
            // Update the mailbox path in memory while preserving the ID
            mailbox.set_path(new_path.clone());
            drop(mailboxes); // Release the lock before persisting

            // Persist updated metadata to disk
            self.persist_user_mailboxes(&user)?;

            Ok(())
        } else {
            Err(anyhow::anyhow!("Mailbox not found"))
        }
    }

    async fn get_mailbox(&self, id: &MailboxId) -> anyhow::Result<Option<Mailbox>> {
        let mailboxes = self
            .mailboxes
            .read()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
        Ok(mailboxes.get(id).cloned())
    }

    async fn list_mailboxes(&self, user: &Username) -> anyhow::Result<Vec<Mailbox>> {
        let mailboxes: Vec<Mailbox> = self
            .mailboxes
            .read()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .values()
            .filter(|m| m.path().user() == user)
            .cloned()
            .collect();
        Ok(mailboxes)
    }

    async fn get_user_inbox(&self, user: &Username) -> anyhow::Result<Option<MailboxId>> {
        let mailbox_id = self
            .mailboxes
            .read()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .values()
            .find(|m| {
                m.path().user() == user
                    && m.path().name().map(|name| name == "INBOX").unwrap_or(false)
            })
            .map(|m| *m.id());

        Ok(mailbox_id)
    }

    async fn subscribe_mailbox(&self, user: &Username, mailbox_name: String) -> anyhow::Result<()> {
        let user_subs = {
            let mut subs = self
                .subscriptions
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            subs.entry(user.clone()).or_default().insert(mailbox_name);

            subs.get(user)
                .map(|s| s.iter().cloned().collect::<Vec<String>>())
                .unwrap_or_default()
        };

        // Persist subscriptions to disk
        let subs_dir = self.base_path.join("users").join(user.as_str());
        tokio::fs::create_dir_all(&subs_dir).await?;
        let subs_file = subs_dir.join("subscriptions");

        let content = user_subs.join("\n");
        tokio::fs::write(subs_file, content).await?;

        Ok(())
    }

    async fn unsubscribe_mailbox(&self, user: &Username, mailbox_name: &str) -> anyhow::Result<()> {
        let user_subs = {
            let mut subs = self
                .subscriptions
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            if let Some(user_subs) = subs.get_mut(user) {
                user_subs.remove(mailbox_name);
            }

            subs.get(user)
                .map(|s| s.iter().cloned().collect::<Vec<String>>())
                .unwrap_or_default()
        };

        // Persist subscriptions to disk
        let subs_dir = self.base_path.join("users").join(user.as_str());
        tokio::fs::create_dir_all(&subs_dir).await?;
        let subs_file = subs_dir.join("subscriptions");

        let content = user_subs.join("\n");
        tokio::fs::write(subs_file, content).await?;

        Ok(())
    }

    async fn list_subscriptions(&self, user: &Username) -> anyhow::Result<Vec<String>> {
        // Try to load from disk first
        let subs_file = self
            .base_path
            .join("users")
            .join(user.as_str())
            .join("subscriptions");

        if tokio::fs::try_exists(&subs_file).await.unwrap_or(false) {
            let content = tokio::fs::read_to_string(&subs_file).await?;
            let subs: Vec<String> = content.lines().map(|s| s.to_string()).collect();

            // Update in-memory cache
            let mut cache = self
                .subscriptions
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            cache.insert(user.clone(), subs.iter().cloned().collect());

            Ok(subs)
        } else {
            // Fall back to in-memory
            let subs = self
                .subscriptions
                .read()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            Ok(subs
                .get(user)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default())
        }
    }
}

/// Filesystem message store
struct FilesystemMessageStore {
    base_path: PathBuf,
    messages: Arc<RwLock<HashMap<MessageId, Mail>>>,
    mailboxes: Arc<RwLock<HashMap<MailboxId, Mailbox>>>,
    quotas: Arc<RwLock<HashMap<Username, Quota>>>,
    hostname: String,
    pid: u32,
    delivery_counter: Arc<RwLock<u64>>,
    /// Broadcast sender for storage events.
    event_tx: broadcast::Sender<StorageEvent>,
    /// Shared per-MailboxId in-process lock map (same Arc as `FilesystemBackend`).
    mailbox_locks: Arc<TokioMutex<HashMap<MailboxId, Arc<TokioMutex<()>>>>>,
}

impl FilesystemMessageStore {
    /// Return (or lazily create) the in-process per-mailbox `Arc<TokioMutex<()>>`.
    ///
    /// This is used in `append_message` to serialise concurrent in-process
    /// operations at the Tokio level *before* the `fs2` file lock is attempted,
    /// preventing N tasks from all racing against the filesystem lock at once.
    async fn per_mailbox_mutex(&self, mailbox_id: &MailboxId) -> Arc<TokioMutex<()>> {
        let mut map = self.mailbox_locks.lock().await;
        map.entry(*mailbox_id)
            .or_insert_with(|| Arc::new(TokioMutex::new(())))
            .clone()
    }
}

#[async_trait]
impl MessageStore for FilesystemMessageStore {
    async fn append_message(
        &self,
        mailbox_id: &MailboxId,
        message: Mail,
    ) -> anyhow::Result<MessageMetadata> {
        let message_id = *message.message_id();
        let message_size = message.size();

        // Get the mailbox to determine the user and mailbox name for events.
        let (user, mailbox_name) = {
            let mailboxes = self
                .mailboxes
                .read()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            let mailbox = mailboxes
                .get(mailbox_id)
                .ok_or_else(|| anyhow::anyhow!("Mailbox not found"))?;
            let name = mailbox.path().name().unwrap_or("INBOX").to_string();
            (mailbox.path().user().clone(), name)
        };

        // Check quota before appending
        let quota = {
            let quotas = self
                .quotas
                .read()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            quotas
                .get(&user)
                .cloned()
                .unwrap_or(Quota::new(0, 1024 * 1024 * 1024)) // Default 1GB
        };

        // Check if adding this message would exceed the quota
        if quota.used + (message_size as u64) > quota.limit {
            return Err(anyhow::anyhow!("Quota exceeded: cannot append message"));
        }

        // Get unique counter for this delivery — done before acquiring the dir
        // lock so the atomic counter is always advancing even on lock failure.
        let counter = {
            let mut c = self
                .delivery_counter
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            *c += 1;
            *c
        };

        // Generate unique maildir filename
        let filename = MaildirFilename::generate(&self.hostname, self.pid, counter);

        let mailbox_dir = self
            .base_path
            .join("mailboxes")
            .join(mailbox_id.to_string());

        // Acquire the in-process per-mailbox Tokio mutex FIRST.
        // This serialises concurrent in-process tasks at the Tokio level so that
        // only one task at a time tries to acquire the underlying fs2 file lock.
        // Without this, N simultaneous tokio::spawn tasks all race against the
        // same lockfile and exhaust the 2s retry budget.
        let _inproc_guard = self.per_mailbox_mutex(mailbox_id).await.lock_owned().await;

        // Acquire exclusive directory lock before any filesystem mutation.
        let _lock = locking::acquire_dir_lock(&mailbox_dir).await?;

        // Step 1: Write to tmp/ directory (atomic delivery part 1)
        let tmp_path = mailbox_dir.join("tmp").join(&filename);
        tokio::fs::create_dir_all(mailbox_dir.join("tmp")).await?;

        // Serialize message to disk
        let message_data = serialize_message_to_bytes(&message).await?;
        tokio::fs::write(&tmp_path, &message_data).await?;

        // Step 2: Rename to new/ directory (atomic delivery part 2)
        // This is atomic on most filesystems
        let new_path = mailbox_dir.join("new").join(&filename);
        tokio::fs::create_dir_all(mailbox_dir.join("new")).await?;
        tokio::fs::rename(&tmp_path, &new_path).await?;

        // Store message in memory
        self.messages
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .insert(message_id, message.clone());

        // Update quota after successful append
        let uid = {
            let mut quotas = self
                .quotas
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            let user_quota = quotas
                .entry(user.clone())
                .or_insert(Quota::new(0, 1024 * 1024 * 1024));
            user_quota.used += message_size as u64;
            // Use counter as a stable UID proxy (monotonically increasing).
            counter as u32
        };

        // Assign RFC 5256 thread ID while the mailbox directory lock is still held.
        let thread_id: Option<String> = {
            let engine = ThreadingEngine::new(&mailbox_dir);
            match engine.assign_thread_id(&message).await {
                Ok(tid) => Some(tid),
                Err(e) => {
                    tracing::warn!("Threading engine failed for message {}: {}", message_id, e);
                    None
                }
            }
        };

        // Create metadata
        let metadata = MessageMetadata::new_with_thread_id(
            message_id,
            *mailbox_id,
            uid,
            MessageFlags::new(),
            message.size(),
            thread_id,
        );

        // Fire MessageStored event after the write commits.
        // Lock is still held here (dropped at end of scope), which is correct.
        events::fire_stored(&self.event_tx, user.to_string(), mailbox_name, uid);

        Ok(metadata)
    }

    async fn get_message(&self, message_id: &MessageId) -> anyhow::Result<Option<Mail>> {
        // First check in-memory cache
        if let Some(mail) = self
            .messages
            .read()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .get(message_id)
            .cloned()
        {
            return Ok(Some(mail));
        }

        // If not in cache, search on disk
        // We need to scan all mailboxes since MessageId doesn't contain mailbox info
        tracing::debug!("Message {} not in cache, scanning disk", message_id);

        let mailboxes_dir = self.base_path.join("mailboxes");
        if !tokio::fs::try_exists(&mailboxes_dir).await.unwrap_or(false) {
            tracing::debug!("Mailboxes directory doesn't exist: {:?}", mailboxes_dir);
            return Ok(None);
        }

        // Scan all mailbox directories
        let mut entries = tokio::fs::read_dir(&mailboxes_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Ok(file_type) = entry.file_type().await {
                if file_type.is_dir() {
                    // This is a mailbox directory - check its messages
                    let mailbox_dir = entry.path();

                    // Try both new/ and cur/ subdirectories
                    for subdir in ["new", "cur"] {
                        let msg_dir = mailbox_dir.join(subdir);
                        if !tokio::fs::try_exists(&msg_dir).await.unwrap_or(false) {
                            continue;
                        }

                        let mut msg_entries = tokio::fs::read_dir(&msg_dir).await?;
                        while let Some(msg_entry) = msg_entries.next_entry().await? {
                            if let Ok(msg_file_type) = msg_entry.file_type().await {
                                if msg_file_type.is_file() {
                                    let file_path = msg_entry.path();

                                    // Read and parse the message to get its MessageId
                                    match tokio::fs::read(&file_path).await {
                                        Ok(data) => {
                                            match rusmes_proto::MimeMessage::parse_from_bytes(&data)
                                            {
                                                Ok(mime_message) => {
                                                    // Extract the stored MessageId from X-Rusmes-Message-Id header
                                                    let stored_message_id = mime_message
                                                        .headers()
                                                        .get_first("x-rusmes-message-id")
                                                        .and_then(|id_str| {
                                                            let trimmed = id_str.trim();
                                                            uuid::Uuid::from_str(trimmed)
                                                                .ok()
                                                                .map(MessageId::from_uuid)
                                                        });

                                                    // Check if this is the message we're looking for
                                                    if let Some(stored_id) = stored_message_id {
                                                        if &stored_id == message_id {
                                                            tracing::debug!(
                                                                "Found message {} on disk at {:?}",
                                                                message_id,
                                                                file_path
                                                            );

                                                            // Create a Mail object with the correct MessageId
                                                            let mail =
                                                                rusmes_proto::Mail::with_message_id(
                                                                    None,
                                                                    Vec::new(),
                                                                    mime_message,
                                                                    None,
                                                                    None,
                                                                    stored_id,
                                                                );

                                                            // Cache it for future lookups
                                                            if let Ok(mut msgs) =
                                                                self.messages.write()
                                                            {
                                                                msgs.insert(
                                                                    *message_id,
                                                                    mail.clone(),
                                                                );
                                                            }

                                                            return Ok(Some(mail));
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        "Failed to parse message file {:?}: {}",
                                                        file_path,
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to read message file {:?}: {}",
                                                file_path,
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        tracing::debug!("Message {} not found on disk", message_id);
        Ok(None)
    }

    async fn delete_messages(&self, message_ids: &[MessageId]) -> anyhow::Result<()> {
        // Remove from in-memory cache.
        {
            let mut messages = self
                .messages
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            for id in message_ids {
                messages.remove(id);
            }
        }

        // Remove from disk: scan all mailbox directories for files carrying
        // an `X-Rusmes-Message-Id` header matching one of the supplied IDs.
        let mailboxes_dir = self.base_path.join("mailboxes");
        if !tokio::fs::try_exists(&mailboxes_dir).await.unwrap_or(false) {
            // Fire expunged events even when no disk work was needed.
            for _ in message_ids {
                events::fire_expunged(&self.event_tx, String::new(), String::new(), 0);
            }
            return Ok(());
        }

        // Build a set of IDs to delete for quick membership testing.
        let ids_to_delete: std::collections::HashSet<MessageId> =
            message_ids.iter().cloned().collect();

        let mut mailbox_entries = tokio::fs::read_dir(&mailboxes_dir).await?;
        while let Some(mbx_entry) = mailbox_entries.next_entry().await? {
            let mbx_file_type = match mbx_entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !mbx_file_type.is_dir() {
                continue;
            }
            let mailbox_dir = mbx_entry.path();

            // Derive the MailboxId from the directory name (UUID string) so we
            // can acquire the in-process per-mailbox Tokio mutex before taking
            // the filesystem lock, matching `append_message` semantics.
            let dir_name = mailbox_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let maybe_inproc_guard = if let Ok(uuid) = uuid::Uuid::parse_str(dir_name) {
                let mid = MailboxId::from_uuid(uuid);
                let mutex = self.per_mailbox_mutex(&mid).await;
                Some(mutex.lock_owned().await)
            } else {
                None
            };

            // Acquire the exclusive directory lock once per mailbox directory
            // (not per file) so that the entire expunge scan+delete is atomic
            // with respect to concurrent deliveries into the same folder.
            let _mailbox_lock = match locking::acquire_dir_lock(&mailbox_dir).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(
                        "Could not acquire dir lock for {:?} during expunge: {}",
                        mailbox_dir,
                        e
                    );
                    drop(maybe_inproc_guard);
                    continue;
                }
            };

            for subdir in ["new", "cur"] {
                let msg_dir = mailbox_dir.join(subdir);
                if !tokio::fs::try_exists(&msg_dir).await.unwrap_or(false) {
                    continue;
                }

                let mut msg_entries = match tokio::fs::read_dir(&msg_dir).await {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                while let Some(msg_entry) = msg_entries.next_entry().await? {
                    let msg_ft = match msg_entry.file_type().await {
                        Ok(ft) => ft,
                        Err(_) => continue,
                    };
                    if !msg_ft.is_file() {
                        continue;
                    }

                    let file_path = msg_entry.path();
                    let data = match tokio::fs::read(&file_path).await {
                        Ok(d) => d,
                        Err(_) => continue,
                    };

                    let mime = match rusmes_proto::MimeMessage::parse_from_bytes(&data) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };

                    let stored_id =
                        mime.headers()
                            .get_first("x-rusmes-message-id")
                            .and_then(|id_str| {
                                uuid::Uuid::from_str(id_str.trim())
                                    .ok()
                                    .map(MessageId::from_uuid)
                            });

                    if let Some(sid) = stored_id {
                        if ids_to_delete.contains(&sid) {
                            // Remove the file. The mailbox-dir lock is already held
                            // above; no inner re-acquire needed.
                            if let Err(e) = tokio::fs::remove_file(&file_path).await {
                                tracing::warn!(
                                    "Failed to delete message file {:?}: {}",
                                    file_path,
                                    e
                                );
                            } else {
                                tracing::debug!(
                                    "Deleted message file {:?} for message {}",
                                    file_path,
                                    sid
                                );
                            }
                            events::fire_expunged(&self.event_tx, String::new(), String::new(), 0);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn set_flags(
        &self,
        message_ids: &[MessageId],
        flags: MessageFlags,
    ) -> anyhow::Result<()> {
        let mailboxes_dir = self.base_path.join("mailboxes");
        if !tokio::fs::try_exists(&mailboxes_dir).await.unwrap_or(false) {
            return Ok(());
        }

        let ids_to_flag: std::collections::HashSet<MessageId> =
            message_ids.iter().cloned().collect();

        let new_flag_suffix = MaildirFilename::encode_flags(&flags);

        let mut mailbox_entries = tokio::fs::read_dir(&mailboxes_dir).await?;
        while let Some(mbx_entry) = mailbox_entries.next_entry().await? {
            let mbx_file_type = match mbx_entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !mbx_file_type.is_dir() {
                continue;
            }
            let mailbox_dir = mbx_entry.path();

            // Messages in new/ have no flags; mark-as-seen moves them to cur/.
            // Messages in cur/ already carry flag suffixes; rename in place.
            for subdir in ["new", "cur"] {
                let msg_dir = mailbox_dir.join(subdir);
                if !tokio::fs::try_exists(&msg_dir).await.unwrap_or(false) {
                    continue;
                }

                let mut msg_entries = match tokio::fs::read_dir(&msg_dir).await {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                while let Some(msg_entry) = msg_entries.next_entry().await? {
                    let msg_ft = match msg_entry.file_type().await {
                        Ok(ft) => ft,
                        Err(_) => continue,
                    };
                    if !msg_ft.is_file() {
                        continue;
                    }

                    let file_path = msg_entry.path();
                    let data = match tokio::fs::read(&file_path).await {
                        Ok(d) => d,
                        Err(_) => continue,
                    };

                    let mime = match rusmes_proto::MimeMessage::parse_from_bytes(&data) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };

                    let stored_id =
                        mime.headers()
                            .get_first("x-rusmes-message-id")
                            .and_then(|id_str| {
                                uuid::Uuid::from_str(id_str.trim())
                                    .ok()
                                    .map(MessageId::from_uuid)
                            });

                    if let Some(sid) = stored_id {
                        if !ids_to_flag.contains(&sid) {
                            continue;
                        }

                        // Determine target directory: always cur/ when flags are set.
                        let target_dir = mailbox_dir.join("cur");
                        if let Err(e) = tokio::fs::create_dir_all(&target_dir).await {
                            tracing::warn!("Failed to create cur/ dir {:?}: {}", target_dir, e);
                            continue;
                        }

                        let old_filename = match file_path.file_name().and_then(|n| n.to_str()) {
                            Some(n) => n.to_string(),
                            None => continue,
                        };

                        let base = MaildirFilename::base_name(&old_filename).to_string();
                        let new_filename = format!("{}{}", base, new_flag_suffix);
                        let new_path = target_dir.join(&new_filename);

                        // Skip rename if source and destination are identical.
                        if file_path == new_path {
                            continue;
                        }

                        // Acquire directory lock before rename.
                        let _lock = match locking::acquire_dir_lock(&mailbox_dir).await {
                            Ok(l) => l,
                            Err(e) => {
                                tracing::warn!(
                                    "Could not acquire dir lock for {:?}: {}",
                                    mailbox_dir,
                                    e
                                );
                                continue;
                            }
                        };

                        if let Err(e) = tokio::fs::rename(&file_path, &new_path).await {
                            tracing::warn!(
                                "Failed to rename {:?} -> {:?}: {}",
                                file_path,
                                new_path,
                                e
                            );
                        } else {
                            tracing::debug!(
                                "set_flags: renamed {:?} -> {:?} for message {}",
                                file_path,
                                new_path,
                                sid
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn get_message_flags(
        &self,
        message_id: &MessageId,
    ) -> anyhow::Result<Option<MessageFlags>> {
        let mailboxes_dir = self.base_path.join("mailboxes");
        if !tokio::fs::try_exists(&mailboxes_dir).await.unwrap_or(false) {
            return Ok(None);
        }

        let mut mailbox_entries = tokio::fs::read_dir(&mailboxes_dir).await?;
        while let Some(mbx_entry) = mailbox_entries.next_entry().await? {
            let mbx_file_type = match mbx_entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !mbx_file_type.is_dir() {
                continue;
            }
            let mailbox_dir = mbx_entry.path();

            for subdir in ["new", "cur"] {
                let msg_dir = mailbox_dir.join(subdir);
                if !tokio::fs::try_exists(&msg_dir).await.unwrap_or(false) {
                    continue;
                }

                let mut msg_entries = match tokio::fs::read_dir(&msg_dir).await {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                while let Some(msg_entry) = msg_entries.next_entry().await? {
                    let msg_ft = match msg_entry.file_type().await {
                        Ok(ft) => ft,
                        Err(_) => continue,
                    };
                    if !msg_ft.is_file() {
                        continue;
                    }

                    let file_path = msg_entry.path();
                    let data = match tokio::fs::read(&file_path).await {
                        Ok(d) => d,
                        Err(_) => continue,
                    };

                    let mime = match rusmes_proto::MimeMessage::parse_from_bytes(&data) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };

                    let stored_id =
                        mime.headers()
                            .get_first("x-rusmes-message-id")
                            .and_then(|id_str| {
                                uuid::Uuid::from_str(id_str.trim())
                                    .ok()
                                    .map(MessageId::from_uuid)
                            });

                    if let Some(sid) = stored_id {
                        if &sid == message_id {
                            let filename = match file_path.file_name().and_then(|n| n.to_str()) {
                                Some(n) => n.to_string(),
                                None => continue,
                            };
                            let flags = MaildirFilename::decode_flags(&filename);
                            return Ok(Some(flags));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    async fn get_message_thread_id(
        &self,
        message_id: &MessageId,
    ) -> anyhow::Result<Option<String>> {
        let mailboxes_dir = self.base_path.join("mailboxes");
        thread_ops::scan_for_thread_id(&mailboxes_dir, message_id).await
    }

    async fn search(
        &self,
        mailbox_id: &MailboxId,
        criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>> {
        // Get all messages in the mailbox
        let messages = self.get_mailbox_messages(mailbox_id).await?;

        // Filter messages based on criteria
        let mut results = Vec::new();
        for metadata in messages {
            if matches_criteria_helper(self, &metadata, &criteria).await? {
                results.push(*metadata.message_id());
            }
        }
        Ok(results)
    }

    async fn copy_messages(
        &self,
        message_ids: &[MessageId],
        dest_mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        let mut result_metadata = Vec::new();

        for message_id in message_ids {
            // Clone the message before the await to avoid holding the lock across await
            let message = {
                let messages = self
                    .messages
                    .read()
                    .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
                messages.get(message_id).cloned()
            };

            if let Some(message) = message {
                // Append the message to the destination mailbox
                let metadata = self.append_message(dest_mailbox_id, message).await?;
                result_metadata.push(metadata);
            }
        }

        Ok(result_metadata)
    }

    async fn get_mailbox_messages(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        let mailbox_dir = self
            .base_path
            .join("mailboxes")
            .join(mailbox_id.to_string());

        let mut results = Vec::new();
        let mut uid_counter = 1u32;

        // Read from new/ directory (unread messages)
        let new_dir = mailbox_dir.join("new");
        if tokio::fs::try_exists(&new_dir).await.unwrap_or(false) {
            let mut entries = tokio::fs::read_dir(&new_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Ok(file_type) = entry.file_type().await {
                    if file_type.is_file() {
                        if let Some(filename) = entry.file_name().to_str() {
                            let file_path = entry.path();

                            // Parse the message from disk
                            match self
                                .parse_message_from_file(
                                    &file_path,
                                    mailbox_id,
                                    uid_counter,
                                    filename,
                                    &mailbox_dir,
                                )
                                .await
                            {
                                Ok(metadata) => {
                                    results.push(metadata);
                                    uid_counter += 1;
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to parse message {}: {}", filename, e);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Read from cur/ directory (messages that have been seen/flagged)
        let cur_dir = mailbox_dir.join("cur");
        if tokio::fs::try_exists(&cur_dir).await.unwrap_or(false) {
            let mut entries = tokio::fs::read_dir(&cur_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Ok(file_type) = entry.file_type().await {
                    if file_type.is_file() {
                        if let Some(filename) = entry.file_name().to_str() {
                            let file_path = entry.path();

                            // Parse the message from disk
                            match self
                                .parse_message_from_file(
                                    &file_path,
                                    mailbox_id,
                                    uid_counter,
                                    filename,
                                    &mailbox_dir,
                                )
                                .await
                            {
                                Ok(metadata) => {
                                    results.push(metadata);
                                    uid_counter += 1;
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to parse message {}: {}", filename, e);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(results)
    }
}

impl FilesystemMessageStore {
    /// Move a message from new/ to cur/ and apply flags
    #[allow(dead_code)]
    async fn mark_as_seen(
        &self,
        mailbox_id: &MailboxId,
        filename: &str,
        flags: &MessageFlags,
    ) -> anyhow::Result<()> {
        let mailbox_dir = self
            .base_path
            .join("mailboxes")
            .join(mailbox_id.to_string());

        let old_path = mailbox_dir.join("new").join(filename);
        let new_filename = MaildirFilename::with_flags(filename, flags);
        let new_path = mailbox_dir.join("cur").join(&new_filename);

        tokio::fs::create_dir_all(mailbox_dir.join("cur")).await?;
        tokio::fs::rename(&old_path, &new_path).await?;

        Ok(())
    }

    /// Update flags on an existing message in cur/
    #[allow(dead_code)]
    async fn update_flags(
        &self,
        mailbox_id: &MailboxId,
        old_filename: &str,
        new_flags: &MessageFlags,
    ) -> anyhow::Result<()> {
        let mailbox_dir = self
            .base_path
            .join("mailboxes")
            .join(mailbox_id.to_string());

        let old_path = mailbox_dir.join("cur").join(old_filename);
        let new_filename = MaildirFilename::with_flags(old_filename, new_flags);
        let new_path = mailbox_dir.join("cur").join(&new_filename);

        if old_path != new_path {
            tokio::fs::rename(&old_path, &new_path).await?;
        }

        Ok(())
    }

    /// List all messages in a mailbox
    #[allow(dead_code)]
    async fn list_messages(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<(String, MessageFlags)>> {
        let mailbox_dir = self
            .base_path
            .join("mailboxes")
            .join(mailbox_id.to_string());
        let mut results = Vec::new();

        // Read from new/ directory
        let new_dir = mailbox_dir.join("new");
        if tokio::fs::try_exists(&new_dir).await.unwrap_or(false) {
            let mut entries = tokio::fs::read_dir(&new_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Some(filename) = entry.file_name().to_str() {
                    let flags = MaildirFilename::decode_flags(filename);
                    results.push((filename.to_string(), flags));
                }
            }
        }

        // Read from cur/ directory
        let cur_dir = mailbox_dir.join("cur");
        if tokio::fs::try_exists(&cur_dir).await.unwrap_or(false) {
            let mut entries = tokio::fs::read_dir(&cur_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Some(filename) = entry.file_name().to_str() {
                    let flags = MaildirFilename::decode_flags(filename);
                    results.push((filename.to_string(), flags));
                }
            }
        }

        Ok(results)
    }

    /// Parse a message from a maildir file and return metadata.
    ///
    /// `mailbox_dir` is used to look up the per-mailbox thread index.
    async fn parse_message_from_file(
        &self,
        file_path: &std::path::Path,
        mailbox_id: &MailboxId,
        uid: u32,
        filename: &str,
        mailbox_dir: &std::path::Path,
    ) -> anyhow::Result<MessageMetadata> {
        // Read the file contents
        let data = tokio::fs::read(file_path).await?;

        // Parse the message
        let mime_message = rusmes_proto::MimeMessage::parse_from_bytes(&data)?;

        // Extract headers before consuming mime_message.
        let stored_message_id = mime_message
            .headers()
            .get_first("x-rusmes-message-id")
            .and_then(|id_str| uuid::Uuid::from_str(id_str).ok().map(MessageId::from_uuid));

        // Extract RFC 5322 Message-ID for thread index lookup (before mime_message is moved).
        let rfc_message_id_opt: Option<String> = mime_message
            .headers()
            .get_first("message-id")
            .map(threading::strip_angle_brackets);

        // Create a Mail object with the correct MessageId
        let mail = if let Some(msg_id) = stored_message_id {
            rusmes_proto::Mail::with_message_id(
                None,       // sender - would need to parse from headers
                Vec::new(), // recipients - would need to parse from headers
                mime_message,
                None, // remote_addr
                None, // remote_host
                msg_id,
            )
        } else {
            // Fallback to generating a new ID if not stored
            rusmes_proto::Mail::new(None, Vec::new(), mime_message, None, None)
        };

        // Get the message size
        let size = data.len();

        // Decode flags from filename
        let flags = MaildirFilename::decode_flags(filename);

        // Store the message in memory cache so it can be retrieved later
        let message_id = *mail.message_id();
        self.messages
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .insert(message_id, mail);

        // Look up thread_id in the per-mailbox index using the RFC 5322 Message-ID.
        let thread_id: Option<String> = if let Some(ref rfc_id) = rfc_message_id_opt {
            let engine = ThreadingEngine::new(mailbox_dir);
            engine.get_thread_id(rfc_id).await.unwrap_or(None)
        } else {
            None
        };

        // Create metadata with thread_id
        let metadata = MessageMetadata::new_with_thread_id(
            message_id,
            *mailbox_id,
            uid,
            flags,
            size,
            thread_id,
        );

        Ok(metadata)
    }
}

/// Filesystem metadata store
struct FilesystemMetadataStore {
    base_path: PathBuf,
    quotas: Arc<RwLock<HashMap<Username, Quota>>>,
}

#[async_trait]
impl MetadataStore for FilesystemMetadataStore {
    async fn get_user_quota(&self, user: &Username) -> anyhow::Result<Quota> {
        Ok(self
            .quotas
            .read()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .get(user)
            .cloned()
            .unwrap_or(Quota::new(0, 1024 * 1024 * 1024))) // Default 1GB
    }

    async fn set_user_quota(&self, user: &Username, quota: Quota) -> anyhow::Result<()> {
        self.quotas
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?
            .insert(user.clone(), quota);
        Ok(())
    }

    async fn get_mailbox_counters(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<MailboxCounters> {
        let mailbox_dir = self
            .base_path
            .join("mailboxes")
            .join(mailbox_id.to_string());

        let mut total = 0;
        let mut recent = 0;

        // Count messages in new/ directory (these are "recent")
        let new_dir = mailbox_dir.join("new");
        if tokio::fs::try_exists(&new_dir).await.unwrap_or(false) {
            let mut entries = tokio::fs::read_dir(&new_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Ok(file_type) = entry.file_type().await {
                    if file_type.is_file() {
                        total += 1;
                        recent += 1;
                    }
                }
            }
        }

        // Count messages in cur/ directory
        let cur_dir = mailbox_dir.join("cur");
        if tokio::fs::try_exists(&cur_dir).await.unwrap_or(false) {
            let mut entries = tokio::fs::read_dir(&cur_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if let Ok(file_type) = entry.file_type().await {
                    if file_type.is_file() {
                        total += 1;
                    }
                }
            }
        }

        Ok(MailboxCounters {
            exists: total,
            recent,
            unseen: 0, // Would need to parse flags to determine
        })
    }
}

/// Check whether a message matches the given search criteria.
fn matches_criteria_helper<'a>(
    store: &'a FilesystemMessageStore,
    metadata: &'a MessageMetadata,
    criteria: &'a SearchCriteria,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
    Box::pin(async move {
        match criteria {
            SearchCriteria::All => Ok(true),
            SearchCriteria::Unseen => Ok(!metadata.flags().is_seen()),
            SearchCriteria::Seen => Ok(metadata.flags().is_seen()),
            SearchCriteria::Flagged => Ok(metadata.flags().is_flagged()),
            SearchCriteria::Unflagged => Ok(!metadata.flags().is_flagged()),
            SearchCriteria::Deleted => Ok(metadata.flags().is_deleted()),
            SearchCriteria::Undeleted => Ok(!metadata.flags().is_deleted()),
            SearchCriteria::From(pattern) => {
                if let Some(mail) = store.get_message(metadata.message_id()).await? {
                    if let Some(v) = mail.message().headers().get_first("from") {
                        return Ok(v.to_lowercase().contains(&pattern.to_lowercase()));
                    }
                }
                Ok(false)
            }
            SearchCriteria::To(pattern) => {
                if let Some(mail) = store.get_message(metadata.message_id()).await? {
                    if let Some(v) = mail.message().headers().get_first("to") {
                        return Ok(v.to_lowercase().contains(&pattern.to_lowercase()));
                    }
                }
                Ok(false)
            }
            SearchCriteria::Subject(pattern) => {
                if let Some(mail) = store.get_message(metadata.message_id()).await? {
                    if let Some(v) = mail.message().headers().get_first("subject") {
                        return Ok(v.to_lowercase().contains(&pattern.to_lowercase()));
                    }
                }
                Ok(false)
            }
            SearchCriteria::Body(pattern) => {
                if let Some(mail) = store.get_message(metadata.message_id()).await? {
                    if let Ok(text) = mail.message().extract_text().await {
                        return Ok(text.to_lowercase().contains(&pattern.to_lowercase()));
                    }
                }
                Ok(false)
            }
            SearchCriteria::And(sub_criteria) => {
                for sub in sub_criteria {
                    if !matches_criteria_helper(store, metadata, sub).await? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            SearchCriteria::Or(sub_criteria) => {
                for sub in sub_criteria {
                    if matches_criteria_helper(store, metadata, sub).await? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            SearchCriteria::Not(sub) => Ok(!matches_criteria_helper(store, metadata, sub).await?),
        }
    })
}

/// Serialize a [`Mail`] object to raw maildir bytes.
///
/// Thin wrapper around [`message_helpers::serialize_message_to_bytes`] so call
/// sites within this module continue to work unchanged.
#[inline]
async fn serialize_message_to_bytes(mail: &Mail) -> anyhow::Result<Vec<u8>> {
    message_helpers::serialize_message_to_bytes(mail).await
}

#[cfg(test)]
#[path = "fs_tests.rs"]
mod tests;
