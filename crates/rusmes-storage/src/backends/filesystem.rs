//! Filesystem-based storage backend using maildir format

use crate::traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria,
};
use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId, Username};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

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

        Self {
            base_path: base_path_buf,
            mailboxes: Arc::new(RwLock::new(mailboxes)),
            messages: Arc::new(RwLock::new(HashMap::new())),
            quotas: Arc::new(RwLock::new(HashMap::new())),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            hostname,
            pid,
            delivery_counter: Arc::new(RwLock::new(0)),
        }
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
        })
    }

    fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        Arc::new(FilesystemMetadataStore {
            base_path: self.base_path.clone(),
            quotas: self.quotas.clone(),
        })
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

        // Get the mailbox to determine the user
        let user = {
            let mailboxes = self
                .mailboxes
                .read()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            let mailbox = mailboxes
                .get(mailbox_id)
                .ok_or_else(|| anyhow::anyhow!("Mailbox not found"))?;
            mailbox.path().user().clone()
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

        // Get unique counter for this delivery
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

        // Step 1: Write to tmp/ directory (atomic delivery part 1)
        let tmp_path = mailbox_dir.join("tmp").join(&filename);
        tokio::fs::create_dir_all(mailbox_dir.join("tmp")).await?;

        // Serialize message to disk
        let message_data = serialize_message_to_bytes(&message)?;
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
        {
            let mut quotas = self
                .quotas
                .write()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            let user_quota = quotas
                .entry(user)
                .or_insert(Quota::new(0, 1024 * 1024 * 1024));
            user_quota.used += message_size as u64;
        }

        // Create metadata
        let metadata = MessageMetadata::new(
            message_id,
            *mailbox_id,
            1, // UID would be properly generated from mailbox state
            MessageFlags::new(),
            message.size(),
        );

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
        let mut messages = self
            .messages
            .write()
            .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
        for id in message_ids {
            messages.remove(id);
        }
        Ok(())
    }

    async fn set_flags(
        &self,
        message_ids: &[MessageId],
        flags: MessageFlags,
    ) -> anyhow::Result<()> {
        // For each message, update its filename to reflect the new flags
        for message_id in message_ids {
            let messages = self
                .messages
                .read()
                .map_err(|e| anyhow::anyhow!("RwLock poisoned: {}", e))?;
            if messages.get(message_id).is_some() {
                // In a full implementation, we would:
                // 1. Find the file in cur/ or new/
                // 2. Rename it with the new flags encoding
                // 3. Update in-memory metadata
                // For now, just store the flags intention
                tracing::debug!("Setting flags {:?} for message {:?}", flags, message_id);
            }
        }
        Ok(())
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

    /// Parse a message from a maildir file and return metadata
    async fn parse_message_from_file(
        &self,
        file_path: &std::path::Path,
        mailbox_id: &MailboxId,
        uid: u32,
        filename: &str,
    ) -> anyhow::Result<MessageMetadata> {
        // Read the file contents
        let data = tokio::fs::read(file_path).await?;

        // Parse the message
        let mime_message = rusmes_proto::MimeMessage::parse_from_bytes(&data)?;

        // Extract the stored MessageId from X-Rusmes-Message-Id header if present
        let stored_message_id = mime_message
            .headers()
            .get_first("x-rusmes-message-id")
            .and_then(|id_str| uuid::Uuid::from_str(id_str).ok().map(MessageId::from_uuid));

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

        // Create metadata
        let metadata = MessageMetadata::new(message_id, *mailbox_id, uid, flags, size);

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

/// Helper: Check if a message matches search criteria
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

            // Content-based searches require loading the full message
            SearchCriteria::From(pattern) => {
                matches_header_pattern_helper(store, metadata, "from", pattern).await
            }
            SearchCriteria::To(pattern) => {
                matches_header_pattern_helper(store, metadata, "to", pattern).await
            }
            SearchCriteria::Subject(pattern) => {
                matches_header_pattern_helper(store, metadata, "subject", pattern).await
            }
            SearchCriteria::Body(pattern) => {
                matches_body_pattern_helper(store, metadata, pattern).await
            }

            // Logical operators
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
            SearchCriteria::Not(sub_criteria) => {
                Ok(!matches_criteria_helper(store, metadata, sub_criteria).await?)
            }
        }
    })
}

/// Helper: Check if a header matches a pattern
async fn matches_header_pattern_helper(
    store: &FilesystemMessageStore,
    metadata: &MessageMetadata,
    header_name: &str,
    pattern: &str,
) -> anyhow::Result<bool> {
    // Load the message from storage
    if let Some(mail) = store.get_message(metadata.message_id()).await? {
        if let Some(header_value) = mail.message().headers().get_first(header_name) {
            // Case-insensitive substring match
            return Ok(header_value
                .to_lowercase()
                .contains(&pattern.to_lowercase()));
        }
    }
    Ok(false)
}

/// Helper: Check if message body matches a pattern
async fn matches_body_pattern_helper(
    store: &FilesystemMessageStore,
    metadata: &MessageMetadata,
    pattern: &str,
) -> anyhow::Result<bool> {
    // Load the message from storage
    if let Some(mail) = store.get_message(metadata.message_id()).await? {
        // Extract text from message body
        if let Ok(text) = mail.message().extract_text() {
            // Case-insensitive substring match
            return Ok(text.to_lowercase().contains(&pattern.to_lowercase()));
        }
    }
    Ok(false)
}

/// Helper function to serialize a Mail object to bytes for storage
fn serialize_message_to_bytes(mail: &Mail) -> anyhow::Result<Vec<u8>> {
    let message = mail.message();
    let headers = message.headers();
    let body = message.body();

    let mut output = Vec::new();

    // Write custom header with MessageId for retrieval
    // This is stored as X-Rusmes-Message-Id to avoid conflicts
    output.extend_from_slice(b"X-Rusmes-Message-Id: ");
    output.extend_from_slice(mail.message_id().to_string().as_bytes());
    output.extend_from_slice(b"\r\n");

    // Write original headers
    for (name, values) in headers.iter() {
        for value in values {
            output.extend_from_slice(name.as_bytes());
            output.extend_from_slice(b": ");
            output.extend_from_slice(value.as_bytes());
            output.extend_from_slice(b"\r\n");
        }
    }

    // Blank line separating headers from body
    output.extend_from_slice(b"\r\n");

    // Write body
    match body {
        rusmes_proto::MessageBody::Small(bytes) => {
            output.extend_from_slice(bytes);
        }
        rusmes_proto::MessageBody::Large(_) => {
            // For large messages, we'd need to stream
            // For now, just return empty
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_filesystem_backend() {
        let backend = FilesystemBackend::new("/tmp/rusmes-test");
        let mailbox_store = backend.mailbox_store();

        let user: Username = "testuser".parse().unwrap();
        let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

        let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();
        let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();

        assert!(mailbox.is_some());
        assert_eq!(mailbox.unwrap().path().user(), &user);
    }

    #[tokio::test]
    async fn test_get_mailbox_messages() {
        use rusmes_proto::{MailAddress, MimeMessage};

        let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();

        let user: Username = "testuser".parse().unwrap();
        let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

        // Create mailbox
        let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

        // Create and append a test message
        let headers = rusmes_proto::HeaderMap::new();
        let body = rusmes_proto::MessageBody::Small(bytes::Bytes::from("Test message body"));
        let mime_message = MimeMessage::new(headers, body);

        let sender = Some("sender@example.com".parse::<MailAddress>().unwrap());
        let recipients = vec!["testuser@localhost".parse::<MailAddress>().unwrap()];
        let mail = rusmes_proto::Mail::new(sender, recipients, mime_message, None, None);

        // Append message
        let metadata = message_store
            .append_message(&mailbox_id, mail)
            .await
            .unwrap();
        assert_eq!(metadata.mailbox_id(), &mailbox_id);

        // Get mailbox messages
        let messages = message_store
            .get_mailbox_messages(&mailbox_id)
            .await
            .unwrap();

        // Verify we got the message back
        assert_eq!(messages.len(), 1, "Should have exactly 1 message");
        let msg = &messages[0];
        assert_eq!(msg.mailbox_id(), &mailbox_id);
        assert_eq!(msg.uid(), 1, "First message should have UID 1");
        assert!(msg.size() > 0, "Message should have non-zero size");

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_get_mailbox_messages_multiple() {
        use rusmes_proto::{MailAddress, MimeMessage};

        let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();

        let user: Username = "testuser".parse().unwrap();
        let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

        // Create mailbox
        let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

        // Append multiple messages
        for i in 0..5 {
            let headers = rusmes_proto::HeaderMap::new();
            let body = rusmes_proto::MessageBody::Small(bytes::Bytes::from(format!(
                "Test message body {}",
                i
            )));
            let mime_message = MimeMessage::new(headers, body);

            let sender = Some(
                format!("sender{}@example.com", i)
                    .parse::<MailAddress>()
                    .unwrap(),
            );
            let recipients = vec!["testuser@localhost".parse::<MailAddress>().unwrap()];
            let mail = rusmes_proto::Mail::new(sender, recipients, mime_message, None, None);

            message_store
                .append_message(&mailbox_id, mail)
                .await
                .unwrap();
        }

        // Get mailbox messages
        let messages = message_store
            .get_mailbox_messages(&mailbox_id)
            .await
            .unwrap();

        // Verify we got all messages
        assert_eq!(messages.len(), 5, "Should have exactly 5 messages");

        // Verify UIDs are sequential
        for (i, msg) in messages.iter().enumerate() {
            assert_eq!(
                msg.uid(),
                (i + 1) as u32,
                "Message {} should have UID {}",
                i,
                i + 1
            );
            assert_eq!(msg.mailbox_id(), &mailbox_id);
            assert!(msg.size() > 0, "Message should have non-zero size");
        }

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_get_mailbox_messages_with_flags() {
        use rusmes_proto::{MailAddress, MimeMessage};

        let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();

        let user: Username = "testuser".parse().unwrap();
        let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

        // Create mailbox
        let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

        // Append a message
        let headers = rusmes_proto::HeaderMap::new();
        let body = rusmes_proto::MessageBody::Small(bytes::Bytes::from("Test message with flags"));
        let mime_message = MimeMessage::new(headers, body);

        let sender = Some("sender@example.com".parse::<MailAddress>().unwrap());
        let recipients = vec!["testuser@localhost".parse::<MailAddress>().unwrap()];
        let mail = rusmes_proto::Mail::new(sender, recipients, mime_message, None, None);

        let _metadata = message_store
            .append_message(&mailbox_id, mail)
            .await
            .unwrap();

        // Initially, message should be in new/ directory with no flags
        let messages = message_store
            .get_mailbox_messages(&mailbox_id)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        let initial_flags = messages[0].flags();
        assert!(
            !initial_flags.is_seen(),
            "New message should not be marked as seen"
        );

        // Manually move the message to cur/ with flags to simulate IMAP flag setting
        let mailbox_dir = temp_dir.join("mailboxes").join(mailbox_id.to_string());
        let new_dir = mailbox_dir.join("new");
        let cur_dir = mailbox_dir.join("cur");

        // Find the message file
        let mut entries = tokio::fs::read_dir(&new_dir).await.unwrap();
        if let Some(entry) = entries.next_entry().await.unwrap() {
            let old_filename = entry.file_name();
            let old_path = new_dir.join(&old_filename);

            // Create new filename with Seen flag (:2,S)
            let base_name = old_filename.to_str().unwrap();
            let new_filename = format!("{}:2,S", base_name.split(":2,").next().unwrap());
            let new_path = cur_dir.join(&new_filename);

            // Move the file
            tokio::fs::rename(&old_path, &new_path).await.unwrap();
        }

        // Re-read messages - should now see the Seen flag
        let messages = message_store
            .get_mailbox_messages(&mailbox_id)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        let updated_flags = messages[0].flags();
        assert!(
            updated_flags.is_seen(),
            "Message should now be marked as seen"
        );

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_get_message_from_disk() {
        use rusmes_proto::{MailAddress, MimeMessage};

        let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();

        let user: Username = "testuser".parse().unwrap();
        let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

        // Create mailbox
        let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

        // Create and append a test message
        let headers = rusmes_proto::HeaderMap::new();
        let body =
            rusmes_proto::MessageBody::Small(bytes::Bytes::from("Test message for disk retrieval"));
        let mime_message = MimeMessage::new(headers, body);

        let sender = Some("sender@example.com".parse::<MailAddress>().unwrap());
        let recipients = vec!["testuser@localhost".parse::<MailAddress>().unwrap()];
        let mail = rusmes_proto::Mail::new(sender, recipients, mime_message, None, None);

        // Store the message ID before appending
        let message_id = *mail.message_id();

        // Append message
        let _metadata = message_store
            .append_message(&mailbox_id, mail)
            .await
            .unwrap();

        // Create a new backend instance to simulate a fresh start (empty cache)
        let backend2 = FilesystemBackend::new(&temp_dir);
        let message_store2 = backend2.message_store();

        // Try to retrieve the message - should load from disk
        let retrieved_mail = message_store2.get_message(&message_id).await.unwrap();

        // Verify we got the message back
        assert!(
            retrieved_mail.is_some(),
            "Should retrieve message from disk"
        );
        let retrieved = retrieved_mail.unwrap();
        assert_eq!(
            retrieved.message_id(),
            &message_id,
            "Message ID should match"
        );
        assert!(
            retrieved.size() > 0,
            "Retrieved message should have non-zero size"
        );

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_mailbox_metadata_persistence() {
        let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));

        let user: Username = "testuser".parse().unwrap();
        let inbox_path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
        let sent_path = MailboxPath::new(user.clone(), vec!["Sent".to_string()]);

        let mailbox_id;
        let sent_id;

        // Create mailboxes in first backend instance
        {
            let backend = FilesystemBackend::new(&temp_dir);
            let mailbox_store = backend.mailbox_store();

            mailbox_id = mailbox_store.create_mailbox(&inbox_path).await.unwrap();
            sent_id = mailbox_store.create_mailbox(&sent_path).await.unwrap();

            // Verify mailboxes exist
            let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();
            assert!(mailbox.is_some());
            assert_eq!(mailbox.unwrap().path().name(), Some("INBOX"));

            let sent_mailbox = mailbox_store.get_mailbox(&sent_id).await.unwrap();
            assert!(sent_mailbox.is_some());
            assert_eq!(sent_mailbox.unwrap().path().name(), Some("Sent"));

            // Verify metadata file was created
            let metadata_file = temp_dir
                .join("users")
                .join(user.as_str())
                .join("mailboxes.json");
            assert!(tokio::fs::try_exists(&metadata_file).await.unwrap());
        }

        // Create new backend instance (simulates server restart)
        {
            let backend = FilesystemBackend::new(&temp_dir);
            let mailbox_store = backend.mailbox_store();

            // Verify mailboxes still exist after "restart"
            let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();
            assert!(mailbox.is_some(), "INBOX should be restored from disk");
            assert_eq!(mailbox.unwrap().path().name(), Some("INBOX"));

            let sent_mailbox = mailbox_store.get_mailbox(&sent_id).await.unwrap();
            assert!(sent_mailbox.is_some(), "Sent should be restored from disk");
            assert_eq!(sent_mailbox.unwrap().path().name(), Some("Sent"));

            // List mailboxes should return both
            let mailboxes = mailbox_store.list_mailboxes(&user).await.unwrap();
            assert_eq!(mailboxes.len(), 2, "Should have 2 mailboxes after restart");

            // Test delete and verify persistence
            mailbox_store.delete_mailbox(&sent_id).await.unwrap();
            let deleted_mailbox = mailbox_store.get_mailbox(&sent_id).await.unwrap();
            assert!(deleted_mailbox.is_none(), "Sent mailbox should be deleted");
        }

        // Create third backend instance to verify deletion was persisted
        {
            let backend = FilesystemBackend::new(&temp_dir);
            let mailbox_store = backend.mailbox_store();

            // INBOX should still exist
            let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();
            assert!(mailbox.is_some(), "INBOX should still exist");

            // Sent should not exist
            let sent_mailbox = mailbox_store.get_mailbox(&sent_id).await.unwrap();
            assert!(sent_mailbox.is_none(), "Sent should still be deleted");

            // List should only return INBOX
            let mailboxes = mailbox_store.list_mailboxes(&user).await.unwrap();
            assert_eq!(mailboxes.len(), 1, "Should have 1 mailbox after restart");
            assert_eq!(mailboxes[0].path().name(), Some("INBOX"));
        }

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_mailbox_metadata_persistence_multiple_users() {
        let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));

        let user1: Username = "user1".parse().unwrap();
        let user2: Username = "user2".parse().unwrap();

        let user1_inbox = MailboxPath::new(user1.clone(), vec!["INBOX".to_string()]);
        let user2_inbox = MailboxPath::new(user2.clone(), vec!["INBOX".to_string()]);

        // Create mailboxes for both users
        {
            let backend = FilesystemBackend::new(&temp_dir);
            let mailbox_store = backend.mailbox_store();

            mailbox_store.create_mailbox(&user1_inbox).await.unwrap();
            mailbox_store.create_mailbox(&user2_inbox).await.unwrap();
        }

        // Verify both users' mailboxes are restored
        {
            let backend = FilesystemBackend::new(&temp_dir);
            let mailbox_store = backend.mailbox_store();

            let user1_mailboxes = mailbox_store.list_mailboxes(&user1).await.unwrap();
            assert_eq!(user1_mailboxes.len(), 1);
            assert_eq!(user1_mailboxes[0].path().user(), &user1);

            let user2_mailboxes = mailbox_store.list_mailboxes(&user2).await.unwrap();
            assert_eq!(user2_mailboxes.len(), 1);
            assert_eq!(user2_mailboxes[0].path().user(), &user2);
        }

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_mailbox_metadata_rename_persistence() {
        let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));

        let user: Username = "testuser".parse().unwrap();
        let original_path = MailboxPath::new(user.clone(), vec!["OldName".to_string()]);
        let new_path = MailboxPath::new(user.clone(), vec!["NewName".to_string()]);

        let mailbox_id;

        // Create and rename mailbox
        {
            let backend = FilesystemBackend::new(&temp_dir);
            let mailbox_store = backend.mailbox_store();

            mailbox_id = mailbox_store.create_mailbox(&original_path).await.unwrap();
            mailbox_store
                .rename_mailbox(&mailbox_id, &new_path)
                .await
                .unwrap();
        }

        // Verify rename was persisted
        {
            let backend = FilesystemBackend::new(&temp_dir);
            let mailbox_store = backend.mailbox_store();

            let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();
            assert!(mailbox.is_some());
            assert_eq!(mailbox.unwrap().path().name(), Some("NewName"));
        }

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
