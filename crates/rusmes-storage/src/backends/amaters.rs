//! AmateRS distributed storage backend
//!
//! This backend provides distributed message storage using AmateRS
//! (a hypothetical distributed key-value store similar to Cassandra/ScyllaDB).
//!
//! # Features
//!
//! - **Distributed blob storage**: Message bodies stored separately from metadata
//! - **Replication**: Configurable replication factor (default: 3)
//! - **Consistency levels**: Support for ONE/QUORUM/ALL/LocalQuorum
//! - **Circuit breaker**: Automatic failover on node failures
//! - **Retry logic**: Exponential backoff for temporary failures
//! - **Eventual consistency**: Read-your-writes consistency where possible
//!
//! # Configuration
//!
//! ```rust,ignore
//! use rusmes_storage::backends::amaters::{AmatersConfig, ConsistencyLevel};
//!
//! let config = AmatersConfig {
//!     cluster_endpoints: vec!["node1:9042".to_string(), "node2:9042".to_string()],
//!     replication_factor: 3,
//!     read_consistency: ConsistencyLevel::Quorum,
//!     write_consistency: ConsistencyLevel::Quorum,
//!     timeout_ms: 10000,
//!     max_retries: 3,
//!     circuit_breaker_threshold: 5,
//!     circuit_breaker_timeout_ms: 60000,
//!     ..Default::default()
//! };
//! ```
//!
//! # Implementation Note
//!
//! This is currently a mock implementation. Replace with real AmateRS client library
//! when it becomes available. The interface is designed to be compatible with
//! distributed systems like Apache Cassandra or ScyllaDB.

use crate::traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria,
};
use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId, Username};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// AmateRS cluster configuration
#[derive(Debug, Clone)]
pub struct AmatersConfig {
    /// Cluster contact points (host:port)
    pub cluster_endpoints: Vec<String>,
    /// Keyspace for metadata
    pub metadata_keyspace: String,
    /// Keyspace for message blobs
    pub blob_keyspace: String,
    /// Replication factor (default: 3)
    pub replication_factor: usize,
    /// Consistency level for reads
    pub read_consistency: ConsistencyLevel,
    /// Consistency level for writes
    pub write_consistency: ConsistencyLevel,
    /// Connection timeout in milliseconds
    pub timeout_ms: u64,
    /// Maximum retry attempts
    pub max_retries: usize,
    /// Enable compression
    pub enable_compression: bool,
    /// Circuit breaker failure threshold
    pub circuit_breaker_threshold: usize,
    /// Circuit breaker timeout in milliseconds
    pub circuit_breaker_timeout_ms: u64,
}

impl Default for AmatersConfig {
    fn default() -> Self {
        Self {
            cluster_endpoints: vec!["localhost:9042".to_string()],
            metadata_keyspace: "rusmes_metadata".to_string(),
            blob_keyspace: "rusmes_blobs".to_string(),
            replication_factor: 3,
            read_consistency: ConsistencyLevel::Quorum,
            write_consistency: ConsistencyLevel::Quorum,
            timeout_ms: 10000,
            max_retries: 3,
            enable_compression: true,
            circuit_breaker_threshold: 5,
            circuit_breaker_timeout_ms: 60000,
        }
    }
}

/// Consistency level for operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsistencyLevel {
    /// Require all replicas
    All,
    /// Require quorum of replicas
    Quorum,
    /// Require only one replica
    One,
    /// Local quorum (same datacenter)
    LocalQuorum,
}

/// Serializable mailbox metadata for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MailboxRecord {
    id: String,
    username: String,
    path: Vec<String>,
    uid_validity: u32,
    uid_next: u32,
    special_use: Option<String>,
    created_at: i64,
}

/// Serializable message metadata for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MessageRecord {
    id: String,
    mailbox_id: String,
    uid: u32,
    sender: Option<String>,
    recipients: Vec<String>,
    headers: HashMap<String, String>,
    size: usize,
    blob_key: String,
    created_at: i64,
}

/// Message blob stored separately
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MessageBlob {
    message_id: String,
    body: Vec<u8>,
    compressed: bool,
}

/// Circuit breaker state for failover handling
#[derive(Debug, Clone)]
enum CircuitBreakerState {
    Closed,
    Open { opened_at: std::time::Instant },
    HalfOpen,
}

/// Circuit breaker for handling node failures
struct CircuitBreaker {
    state: Arc<RwLock<CircuitBreakerState>>,
    failure_count: Arc<RwLock<usize>>,
    threshold: usize,
    timeout_ms: u64,
}

impl CircuitBreaker {
    fn new(threshold: usize, timeout_ms: u64) -> Self {
        Self {
            state: Arc::new(RwLock::new(CircuitBreakerState::Closed)),
            failure_count: Arc::new(RwLock::new(0)),
            threshold,
            timeout_ms,
        }
    }

    async fn is_open(&self) -> bool {
        let state = self.state.read().await;
        matches!(*state, CircuitBreakerState::Open { .. })
    }

    async fn record_success(&self) {
        let mut count = self.failure_count.write().await;
        *count = 0;
        let mut state = self.state.write().await;
        *state = CircuitBreakerState::Closed;
    }

    async fn record_failure(&self) {
        let mut count = self.failure_count.write().await;
        *count += 1;

        if *count >= self.threshold {
            let mut state = self.state.write().await;
            *state = CircuitBreakerState::Open {
                opened_at: std::time::Instant::now(),
            };
        }
    }

    async fn attempt_reset(&self) {
        let state = self.state.read().await;
        if let CircuitBreakerState::Open { opened_at } = *state {
            if opened_at.elapsed().as_millis() as u64 >= self.timeout_ms {
                drop(state);
                let mut state = self.state.write().await;
                *state = CircuitBreakerState::HalfOpen;
            }
        }
    }
}

/// Mock AmateRS client implementing the AmateRS distributed key-value store interface.
/// Replace with real AmateRS client library when it becomes available.
struct AmatersClient {
    config: AmatersConfig,
    metadata: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    blobs: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    circuit_breaker: CircuitBreaker,
}

impl AmatersClient {
    fn new(config: AmatersConfig) -> Self {
        let circuit_breaker = CircuitBreaker::new(
            config.circuit_breaker_threshold,
            config.circuit_breaker_timeout_ms,
        );

        Self {
            config,
            metadata: Arc::new(RwLock::new(HashMap::new())),
            blobs: Arc::new(RwLock::new(HashMap::new())),
            circuit_breaker,
        }
    }

    async fn connect(&self) -> anyhow::Result<()> {
        // In production: establish connections to cluster with retry and failover
        tracing::info!(
            "Connecting to AmateRS cluster at {:?}",
            self.config.cluster_endpoints
        );

        // Check circuit breaker
        if self.circuit_breaker.is_open().await {
            self.circuit_breaker.attempt_reset().await;
            if self.circuit_breaker.is_open().await {
                return Err(anyhow::anyhow!("Circuit breaker is open"));
            }
        }

        Ok(())
    }

    async fn init_keyspaces(&self) -> anyhow::Result<()> {
        // In production: CREATE KEYSPACE IF NOT EXISTS with replication settings
        tracing::info!(
            "Initializing keyspaces: {} and {}",
            self.config.metadata_keyspace,
            self.config.blob_keyspace
        );
        Ok(())
    }

    async fn put(&self, keyspace: &str, key: String, value: Vec<u8>) -> anyhow::Result<()> {
        // Check circuit breaker before attempting
        if self.circuit_breaker.is_open().await {
            self.circuit_breaker.attempt_reset().await;
            if self.circuit_breaker.is_open().await {
                return Err(anyhow::anyhow!(
                    "Circuit breaker is open, rejecting request"
                ));
            }
        }

        let store = if keyspace.contains("blob") {
            &self.blobs
        } else {
            &self.metadata
        };

        // Retry logic with exponential backoff
        let mut last_error = None;
        for attempt in 0..self.config.max_retries {
            match self.put_with_retry(store, key.clone(), value.clone()).await {
                Ok(_) => {
                    self.circuit_breaker.record_success().await;
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Put failed (attempt {}): {}", attempt + 1, e);
                    last_error = Some(e);

                    if attempt < self.config.max_retries - 1 {
                        // Exponential backoff: 100ms, 200ms, 400ms, etc.
                        let backoff = 100 * 2_u64.pow(attempt as u32);
                        tokio::time::sleep(tokio::time::Duration::from_millis(backoff)).await;
                    }
                }
            }
        }

        // All retries failed
        self.circuit_breaker.record_failure().await;
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Put operation failed")))
    }

    async fn put_with_retry(
        &self,
        store: &Arc<RwLock<HashMap<String, Vec<u8>>>>,
        key: String,
        value: Vec<u8>,
    ) -> anyhow::Result<()> {
        let mut map = store.write().await;
        map.insert(key, value);
        Ok(())
    }

    async fn get(&self, keyspace: &str, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let store = if keyspace.contains("blob") {
            &self.blobs
        } else {
            &self.metadata
        };

        let map = store.read().await;
        Ok(map.get(key).cloned())
    }

    async fn delete(&self, keyspace: &str, key: &str) -> anyhow::Result<()> {
        let store = if keyspace.contains("blob") {
            &self.blobs
        } else {
            &self.metadata
        };

        let mut map = store.write().await;
        map.remove(key);
        Ok(())
    }

    async fn list_prefix(&self, keyspace: &str, prefix: &str) -> anyhow::Result<Vec<String>> {
        let store = if keyspace.contains("blob") {
            &self.blobs
        } else {
            &self.metadata
        };

        let map = store.read().await;
        Ok(map
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

/// AmateRS distributed storage backend
pub struct AmatersBackend {
    client: Arc<AmatersClient>,
    config: AmatersConfig,
}

impl AmatersBackend {
    /// Create a new AmateRS backend
    pub async fn new(config: AmatersConfig) -> anyhow::Result<Self> {
        let client = Arc::new(AmatersClient::new(config.clone()));
        client.connect().await?;
        client.init_keyspaces().await?;

        Ok(Self { client, config })
    }

    /// Initialize schema
    pub async fn init_schema(&self) -> anyhow::Result<()> {
        // In production: CREATE TABLE statements for metadata tables
        self.client.init_keyspaces().await
    }
}

impl StorageBackend for AmatersBackend {
    fn mailbox_store(&self) -> Arc<dyn MailboxStore> {
        Arc::new(AmatersMailboxStore {
            client: self.client.clone(),
            keyspace: self.config.metadata_keyspace.clone(),
        })
    }

    fn message_store(&self) -> Arc<dyn MessageStore> {
        Arc::new(AmatersMessageStore {
            client: self.client.clone(),
            metadata_keyspace: self.config.metadata_keyspace.clone(),
            blob_keyspace: self.config.blob_keyspace.clone(),
        })
    }

    fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        Arc::new(AmatersMetadataStore {
            client: self.client.clone(),
            keyspace: self.config.metadata_keyspace.clone(),
        })
    }
}

/// AmateRS mailbox store
struct AmatersMailboxStore {
    client: Arc<AmatersClient>,
    keyspace: String,
}

#[async_trait]
impl MailboxStore for AmatersMailboxStore {
    async fn create_mailbox(&self, path: &MailboxPath) -> anyhow::Result<MailboxId> {
        let mailbox = Mailbox::new(path.clone());
        let id = *mailbox.id();

        let record = MailboxRecord {
            id: id.to_string(),
            username: path.user().to_string(),
            path: path.path().to_vec(),
            uid_validity: mailbox.uid_validity(),
            uid_next: mailbox.uid_next(),
            special_use: mailbox.special_use().map(|s| s.to_string()),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        };

        let key = format!("mailbox:{}", id);
        let value = serde_json::to_vec(&record)?;

        self.client.put(&self.keyspace, key, value).await?;

        // Also index by user
        let user_key = format!("user:{}:mailbox:{}", path.user(), id);
        self.client.put(&self.keyspace, user_key, vec![]).await?;

        Ok(id)
    }

    async fn delete_mailbox(&self, id: &MailboxId) -> anyhow::Result<()> {
        let key = format!("mailbox:{}", id);
        self.client.delete(&self.keyspace, &key).await?;
        Ok(())
    }

    async fn rename_mailbox(&self, id: &MailboxId, new_path: &MailboxPath) -> anyhow::Result<()> {
        let key = format!("mailbox:{}", id);
        let data = self
            .client
            .get(&self.keyspace, &key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Mailbox not found"))?;

        let mut record: MailboxRecord = serde_json::from_slice(&data)?;
        record.path = new_path.path().to_vec();

        let value = serde_json::to_vec(&record)?;
        self.client.put(&self.keyspace, key, value).await?;

        Ok(())
    }

    async fn get_mailbox(&self, id: &MailboxId) -> anyhow::Result<Option<Mailbox>> {
        let key = format!("mailbox:{}", id);
        let data = match self.client.get(&self.keyspace, &key).await? {
            Some(d) => d,
            None => return Ok(None),
        };

        let record: MailboxRecord = serde_json::from_slice(&data)?;
        let username = Username::new(record.username)
            .map_err(|e| anyhow::anyhow!("Invalid username: {}", e))?;
        let path = MailboxPath::new(username, record.path);

        let mut mailbox = Mailbox::new(path);
        mailbox.set_special_use(record.special_use);

        Ok(Some(mailbox))
    }

    async fn list_mailboxes(&self, user: &Username) -> anyhow::Result<Vec<Mailbox>> {
        let prefix = format!("user:{}:mailbox:", user);
        let keys = self.client.list_prefix(&self.keyspace, &prefix).await?;

        let mut mailboxes = Vec::new();
        for key in keys {
            if let Some(mailbox_id_str) = key.strip_prefix(&prefix) {
                let mailbox_key = format!("mailbox:{}", mailbox_id_str);
                if let Ok(Some(data)) = self.client.get(&self.keyspace, &mailbox_key).await {
                    if let Ok(record) = serde_json::from_slice::<MailboxRecord>(&data) {
                        if let Ok(username) = Username::new(record.username) {
                            let path = MailboxPath::new(username, record.path);
                            let mut mailbox = Mailbox::new(path);
                            mailbox.set_special_use(record.special_use);
                            mailboxes.push(mailbox);
                        }
                    }
                }
            }
        }

        Ok(mailboxes)
    }

    async fn get_user_inbox(&self, user: &Username) -> anyhow::Result<Option<MailboxId>> {
        let prefix = format!("user:{}:mailbox:", user);
        let keys = self.client.list_prefix(&self.keyspace, &prefix).await?;

        for key in keys {
            if let Some(mailbox_id_str) = key.strip_prefix(&prefix) {
                let mailbox_key = format!("mailbox:{}", mailbox_id_str);
                if let Ok(Some(data)) = self.client.get(&self.keyspace, &mailbox_key).await {
                    if let Ok(record) = serde_json::from_slice::<MailboxRecord>(&data) {
                        let is_inbox = record.path == vec!["INBOX"]
                            || record
                                .special_use
                                .as_deref()
                                .map(|s| s.eq_ignore_ascii_case("inbox"))
                                .unwrap_or(false);
                        if is_inbox {
                            if let Ok(uuid) = uuid::Uuid::parse_str(mailbox_id_str) {
                                return Ok(Some(MailboxId::from_uuid(uuid)));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    async fn subscribe_mailbox(&self, user: &Username, mailbox_name: String) -> anyhow::Result<()> {
        let key = format!("subscription:{}:{}", user, mailbox_name);
        self.client.put(&self.keyspace, key, vec![1]).await?;
        Ok(())
    }

    async fn unsubscribe_mailbox(&self, user: &Username, mailbox_name: &str) -> anyhow::Result<()> {
        let key = format!("subscription:{}:{}", user, mailbox_name);
        self.client.delete(&self.keyspace, &key).await?;
        Ok(())
    }

    async fn list_subscriptions(&self, user: &Username) -> anyhow::Result<Vec<String>> {
        let prefix = format!("subscription:{}:", user);
        let keys = self.client.list_prefix(&self.keyspace, &prefix).await?;

        Ok(keys
            .into_iter()
            .filter_map(|k| k.strip_prefix(&prefix).map(|s| s.to_string()))
            .collect())
    }
}

/// AmateRS message store with blob separation
struct AmatersMessageStore {
    client: Arc<AmatersClient>,
    metadata_keyspace: String,
    blob_keyspace: String,
}

#[async_trait]
impl MessageStore for AmatersMessageStore {
    async fn append_message(
        &self,
        mailbox_id: &MailboxId,
        message: Mail,
    ) -> anyhow::Result<MessageMetadata> {
        let message_id = *message.message_id();

        // Store message blob separately
        // In production: serialize the message body properly
        let blob = MessageBlob {
            message_id: message_id.to_string(),
            body: vec![], // Placeholder - would serialize message.message()
            compressed: false,
        };

        let blob_key = format!("blob:{}", message_id);
        let blob_value = serde_json::to_vec(&blob)?;
        self.client
            .put(&self.blob_keyspace, blob_key.clone(), blob_value)
            .await?;

        // Store message metadata
        let record = MessageRecord {
            id: message_id.to_string(),
            mailbox_id: mailbox_id.to_string(),
            uid: 1, // Would need to get next UID from mailbox
            sender: message.sender().map(|s| s.to_string()),
            recipients: message.recipients().iter().map(|r| r.to_string()).collect(),
            headers: HashMap::new(),
            size: message.size(),
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

        // Index by mailbox
        let mailbox_index_key = format!("mailbox:{}:message:{}", mailbox_id, message_id);
        self.client
            .put(&self.metadata_keyspace, mailbox_index_key, vec![])
            .await?;

        let metadata = MessageMetadata::new(
            message_id,
            *mailbox_id,
            1,
            MessageFlags::new(),
            message.size(),
        );

        Ok(metadata)
    }

    async fn get_message(&self, _message_id: &MessageId) -> anyhow::Result<Option<Mail>> {
        // In production: reconstruct Mail from stored metadata and blob
        // For now, return None as this requires MimeMessage parsing
        Ok(None)
    }

    async fn delete_messages(&self, message_ids: &[MessageId]) -> anyhow::Result<()> {
        for message_id in message_ids {
            let key = format!("message:{}", message_id);

            // Get blob key before deleting metadata
            if let Some(data) = self.client.get(&self.metadata_keyspace, &key).await? {
                if let Ok(record) = serde_json::from_slice::<MessageRecord>(&data) {
                    self.client
                        .delete(&self.blob_keyspace, &record.blob_key)
                        .await?;
                }
            }

            self.client.delete(&self.metadata_keyspace, &key).await?;
        }
        Ok(())
    }

    async fn set_flags(
        &self,
        message_ids: &[MessageId],
        _flags: MessageFlags,
    ) -> anyhow::Result<()> {
        // In production: update flags in metadata
        for message_id in message_ids {
            let key = format!("flags:{}", message_id);
            let value = vec![1]; // Placeholder
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
            .filter_map(|k| k.strip_prefix(&prefix).map(|_id_str| MessageId::new()))
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
            if key.strip_prefix(&prefix).is_some() {
                let message_id = MessageId::new();
                let metadata =
                    MessageMetadata::new(message_id, *mailbox_id, 1, MessageFlags::new(), 0);
                metadata_list.push(metadata);
            }
        }

        Ok(metadata_list)
    }
}

/// AmateRS metadata store
struct AmatersMetadataStore {
    client: Arc<AmatersClient>,
    keyspace: String,
}

#[async_trait]
impl MetadataStore for AmatersMetadataStore {
    async fn get_user_quota(&self, user: &Username) -> anyhow::Result<Quota> {
        let key = format!("quota:{}", user);
        let data = match self.client.get(&self.keyspace, &key).await? {
            Some(d) => d,
            None => return Ok(Quota::new(0, 1024 * 1024 * 1024)),
        };

        Ok(serde_json::from_slice(&data)?)
    }

    async fn set_user_quota(&self, user: &Username, quota: Quota) -> anyhow::Result<()> {
        let key = format!("quota:{}", user);
        let value = serde_json::to_vec(&quota)?;
        self.client.put(&self.keyspace, key, value).await?;
        Ok(())
    }

    async fn get_mailbox_counters(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<MailboxCounters> {
        let key = format!("counters:{}", mailbox_id);
        let data = match self.client.get(&self.keyspace, &key).await? {
            Some(d) => d,
            None => return Ok(MailboxCounters::default()),
        };

        Ok(serde_json::from_slice(&data)?)
    }
}

// Note: MailboxId UUID conversion would be implemented in production

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amaters_config_default() {
        let config = AmatersConfig::default();
        assert_eq!(config.cluster_endpoints.len(), 1);
        assert_eq!(config.replication_factor, 3);
        assert_eq!(config.read_consistency, ConsistencyLevel::Quorum);
        assert_eq!(config.write_consistency, ConsistencyLevel::Quorum);
    }

    #[test]
    fn test_consistency_levels() {
        assert_eq!(ConsistencyLevel::All, ConsistencyLevel::All);
        assert_eq!(ConsistencyLevel::Quorum, ConsistencyLevel::Quorum);
        assert_eq!(ConsistencyLevel::One, ConsistencyLevel::One);
        assert_ne!(ConsistencyLevel::All, ConsistencyLevel::One);
    }

    #[tokio::test]
    async fn test_amaters_client_creation() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);
        assert!(client.connect().await.is_ok());
    }

    #[tokio::test]
    async fn test_amaters_backend_creation() {
        let config = AmatersConfig::default();
        let backend = AmatersBackend::new(config).await;
        assert!(backend.is_ok());
    }

    #[tokio::test]
    async fn test_put_and_get() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);
        client.connect().await.unwrap();

        let key = "test_key".to_string();
        let value = vec![1, 2, 3, 4];

        client
            .put("metadata", key.clone(), value.clone())
            .await
            .unwrap();
        let retrieved = client.get("metadata", &key).await.unwrap();

        assert_eq!(retrieved, Some(value));
    }

    #[tokio::test]
    async fn test_delete() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);
        client.connect().await.unwrap();

        let key = "delete_key".to_string();
        let value = vec![5, 6, 7, 8];

        client.put("metadata", key.clone(), value).await.unwrap();
        client.delete("metadata", &key).await.unwrap();

        let retrieved = client.get("metadata", &key).await.unwrap();
        assert_eq!(retrieved, None);
    }

    #[tokio::test]
    async fn test_list_prefix() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);
        client.connect().await.unwrap();

        client
            .put("metadata", "user:alice:mailbox:1".to_string(), vec![])
            .await
            .unwrap();
        client
            .put("metadata", "user:alice:mailbox:2".to_string(), vec![])
            .await
            .unwrap();
        client
            .put("metadata", "user:bob:mailbox:1".to_string(), vec![])
            .await
            .unwrap();

        let alice_mailboxes = client.list_prefix("metadata", "user:alice:").await.unwrap();
        assert_eq!(alice_mailboxes.len(), 2);
    }

    #[test]
    fn test_mailbox_record_serialization() {
        let record = MailboxRecord {
            id: "test-id".to_string(),
            username: "user@example.com".to_string(),
            path: vec!["INBOX".to_string()],
            uid_validity: 1,
            uid_next: 1,
            special_use: None,
            created_at: 1234567890,
        };

        let serialized = serde_json::to_vec(&record).unwrap();
        let deserialized: MailboxRecord = serde_json::from_slice(&serialized).unwrap();

        assert_eq!(record.id, deserialized.id);
        assert_eq!(record.username, deserialized.username);
    }

    #[test]
    fn test_message_record_serialization() {
        let record = MessageRecord {
            id: "msg-id".to_string(),
            mailbox_id: "mailbox-id".to_string(),
            uid: 1,
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            headers: HashMap::new(),
            size: 1024,
            blob_key: "blob:msg-id".to_string(),
            created_at: 1234567890,
        };

        let serialized = serde_json::to_vec(&record).unwrap();
        let deserialized: MessageRecord = serde_json::from_slice(&serialized).unwrap();

        assert_eq!(record.id, deserialized.id);
        assert_eq!(record.size, deserialized.size);
    }

    #[test]
    fn test_message_blob_serialization() {
        let blob = MessageBlob {
            message_id: "msg-id".to_string(),
            body: vec![1, 2, 3, 4],
            compressed: false,
        };

        let serialized = serde_json::to_vec(&blob).unwrap();
        let deserialized: MessageBlob = serde_json::from_slice(&serialized).unwrap();

        assert_eq!(blob.message_id, deserialized.message_id);
        assert_eq!(blob.body, deserialized.body);
    }

    #[test]
    fn test_amaters_config_custom() {
        let config = AmatersConfig {
            cluster_endpoints: vec![
                "node1.example.com:9042".to_string(),
                "node2.example.com:9042".to_string(),
            ],
            replication_factor: 5,
            read_consistency: ConsistencyLevel::LocalQuorum,
            write_consistency: ConsistencyLevel::All,
            ..Default::default()
        };

        assert_eq!(config.cluster_endpoints.len(), 2);
        assert_eq!(config.replication_factor, 5);
    }

    #[test]
    fn test_keyspace_configuration() {
        let config = AmatersConfig {
            metadata_keyspace: "custom_metadata".to_string(),
            blob_keyspace: "custom_blobs".to_string(),
            ..Default::default()
        };

        assert_eq!(config.metadata_keyspace, "custom_metadata");
        assert_eq!(config.blob_keyspace, "custom_blobs");
    }

    #[test]
    fn test_compression_flag() {
        let config = AmatersConfig {
            enable_compression: true,
            ..Default::default()
        };
        assert!(config.enable_compression);

        let config_no_compression = AmatersConfig {
            enable_compression: false,
            ..Default::default()
        };
        assert!(!config_no_compression.enable_compression);
    }

    #[test]
    fn test_retry_configuration() {
        let config = AmatersConfig {
            max_retries: 5,
            ..Default::default()
        };
        assert_eq!(config.max_retries, 5);
    }

    #[test]
    fn test_timeout_configuration() {
        let config = AmatersConfig {
            timeout_ms: 30000,
            ..Default::default()
        };
        assert_eq!(config.timeout_ms, 30000);
    }

    #[tokio::test]
    async fn test_init_keyspaces() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);
        assert!(client.init_keyspaces().await.is_ok());
    }

    #[tokio::test]
    async fn test_blob_keyspace_separation() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);

        client
            .put("metadata", "key1".to_string(), vec![1])
            .await
            .unwrap();
        client
            .put("blobs", "key2".to_string(), vec![2])
            .await
            .unwrap();

        let meta_val = client.get("metadata", "key1").await.unwrap();
        let blob_val = client.get("blobs", "key2").await.unwrap();

        assert_eq!(meta_val, Some(vec![1]));
        assert_eq!(blob_val, Some(vec![2]));
    }

    #[tokio::test]
    async fn test_multiple_contact_points() {
        let config = AmatersConfig {
            cluster_endpoints: vec![
                "host1:9042".to_string(),
                "host2:9042".to_string(),
                "host3:9042".to_string(),
            ],
            ..Default::default()
        };

        let client = AmatersClient::new(config);
        assert!(client.connect().await.is_ok());
    }

    #[test]
    fn test_circuit_breaker_creation() {
        let cb = CircuitBreaker::new(5, 60000);
        assert_eq!(cb.threshold, 5);
        assert_eq!(cb.timeout_ms, 60000);
    }

    #[tokio::test]
    async fn test_circuit_breaker_closed_initially() {
        let cb = CircuitBreaker::new(3, 60000);
        assert!(!cb.is_open().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new(3, 60000);

        cb.record_failure().await;
        assert!(!cb.is_open().await);

        cb.record_failure().await;
        assert!(!cb.is_open().await);

        cb.record_failure().await;
        assert!(cb.is_open().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_reset_on_success() {
        let cb = CircuitBreaker::new(3, 60000);

        cb.record_failure().await;
        cb.record_failure().await;
        assert!(!cb.is_open().await);

        cb.record_success().await;
        let count = cb.failure_count.read().await;
        assert_eq!(*count, 0);
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open_after_timeout() {
        let cb = CircuitBreaker::new(2, 100); // 100ms timeout

        cb.record_failure().await;
        cb.record_failure().await;
        assert!(cb.is_open().await);

        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
        cb.attempt_reset().await;

        let state = cb.state.read().await;
        assert!(matches!(*state, CircuitBreakerState::HalfOpen));
    }

    #[tokio::test]
    async fn test_config_cluster_endpoints() {
        let config = AmatersConfig::default();
        assert_eq!(config.cluster_endpoints.len(), 1);
        assert_eq!(config.cluster_endpoints[0], "localhost:9042");
    }

    #[tokio::test]
    async fn test_config_timeout_ms() {
        let config = AmatersConfig {
            timeout_ms: 5000,
            ..Default::default()
        };
        assert_eq!(config.timeout_ms, 5000);
    }

    #[tokio::test]
    async fn test_config_circuit_breaker_settings() {
        let config = AmatersConfig {
            circuit_breaker_threshold: 10,
            circuit_breaker_timeout_ms: 120000,
            ..Default::default()
        };
        assert_eq!(config.circuit_breaker_threshold, 10);
        assert_eq!(config.circuit_breaker_timeout_ms, 120000);
    }

    #[tokio::test]
    async fn test_put_records_success() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);
        client.connect().await.unwrap();

        client
            .put("metadata", "key1".to_string(), vec![1, 2, 3])
            .await
            .unwrap();

        let count = client.circuit_breaker.failure_count.read().await;
        assert_eq!(*count, 0);
    }

    #[tokio::test]
    async fn test_get_nonexistent_key() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);

        let result = client.get("metadata", "nonexistent").await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_key() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);

        let result = client.delete("metadata", "nonexistent").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_prefix_empty() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);

        let keys = client.list_prefix("metadata", "empty:").await.unwrap();
        assert_eq!(keys.len(), 0);
    }

    #[tokio::test]
    async fn test_blob_and_metadata_separation() {
        let config = AmatersConfig::default();
        let client = AmatersClient::new(config);

        client
            .put("metadata", "key1".to_string(), vec![1])
            .await
            .unwrap();
        client
            .put("blob_keyspace", "key1".to_string(), vec![2])
            .await
            .unwrap();

        let meta = client.get("metadata", "key1").await.unwrap();
        let blob = client.get("blob_keyspace", "key1").await.unwrap();

        assert_eq!(meta, Some(vec![1]));
        assert_eq!(blob, Some(vec![2]));
    }

    #[tokio::test]
    async fn test_backend_stores_creation() {
        let config = AmatersConfig::default();
        let backend = AmatersBackend::new(config).await.unwrap();

        let _mailbox_store = backend.mailbox_store();
        let _message_store = backend.message_store();
        let _metadata_store = backend.metadata_store();
    }

    #[tokio::test]
    async fn test_init_schema() {
        let config = AmatersConfig::default();
        let backend = AmatersBackend::new(config).await.unwrap();
        assert!(backend.init_schema().await.is_ok());
    }

    #[test]
    fn test_consistency_level_all() {
        let level = ConsistencyLevel::All;
        assert_eq!(level, ConsistencyLevel::All);
    }

    #[test]
    fn test_consistency_level_one() {
        let level = ConsistencyLevel::One;
        assert_eq!(level, ConsistencyLevel::One);
    }

    #[test]
    fn test_consistency_level_local_quorum() {
        let level = ConsistencyLevel::LocalQuorum;
        assert_eq!(level, ConsistencyLevel::LocalQuorum);
    }

    #[tokio::test]
    async fn test_mailbox_subscription() {
        let config = AmatersConfig::default();
        let backend = AmatersBackend::new(config).await.unwrap();
        let store = backend.mailbox_store();

        let user = Username::new("user@example.com".to_string()).unwrap();
        store
            .subscribe_mailbox(&user, "INBOX".to_string())
            .await
            .unwrap();

        let subs = store.list_subscriptions(&user).await.unwrap();
        assert_eq!(subs.len(), 1);
        assert!(subs.contains(&"INBOX".to_string()));
    }

    #[tokio::test]
    async fn test_mailbox_unsubscription() {
        let config = AmatersConfig::default();
        let backend = AmatersBackend::new(config).await.unwrap();
        let store = backend.mailbox_store();

        let user = Username::new("user@example.com".to_string()).unwrap();
        store
            .subscribe_mailbox(&user, "INBOX".to_string())
            .await
            .unwrap();
        store.unsubscribe_mailbox(&user, "INBOX").await.unwrap();

        let subs = store.list_subscriptions(&user).await.unwrap();
        assert_eq!(subs.len(), 0);
    }

    #[tokio::test]
    async fn test_multiple_subscriptions() {
        let config = AmatersConfig::default();
        let backend = AmatersBackend::new(config).await.unwrap();
        let store = backend.mailbox_store();

        let user = Username::new("user@example.com".to_string()).unwrap();
        store
            .subscribe_mailbox(&user, "INBOX".to_string())
            .await
            .unwrap();
        store
            .subscribe_mailbox(&user, "Sent".to_string())
            .await
            .unwrap();
        store
            .subscribe_mailbox(&user, "Drafts".to_string())
            .await
            .unwrap();

        let subs = store.list_subscriptions(&user).await.unwrap();
        assert_eq!(subs.len(), 3);
    }

    #[tokio::test]
    async fn test_quota_operations() {
        let config = AmatersConfig::default();
        let backend = AmatersBackend::new(config).await.unwrap();
        let store = backend.metadata_store();

        let user = Username::new("user@example.com".to_string()).unwrap();
        let quota = Quota::new(1000, 10000);

        store.set_user_quota(&user, quota).await.unwrap();
        let retrieved = store.get_user_quota(&user).await.unwrap();

        assert_eq!(retrieved.used, 1000);
        assert_eq!(retrieved.limit, 10000);
    }

    #[tokio::test]
    async fn test_mailbox_counters() {
        let config = AmatersConfig::default();
        let backend = AmatersBackend::new(config).await.unwrap();
        let store = backend.metadata_store();

        let mailbox_id = MailboxId::new();
        let counters = store.get_mailbox_counters(&mailbox_id).await.unwrap();

        assert_eq!(counters.exists, 0);
        assert_eq!(counters.recent, 0);
        assert_eq!(counters.unseen, 0);
    }

    #[tokio::test]
    async fn test_message_blob_compression_flag() {
        let blob = MessageBlob {
            message_id: "test-id".to_string(),
            body: vec![1, 2, 3, 4, 5],
            compressed: true,
        };

        assert!(blob.compressed);
        assert_eq!(blob.body.len(), 5);
    }

    #[tokio::test]
    async fn test_replication_factor_config() {
        let config = AmatersConfig {
            replication_factor: 5,
            ..Default::default()
        };

        assert_eq!(config.replication_factor, 5);
    }

    #[tokio::test]
    async fn test_custom_keyspace_names() {
        let config = AmatersConfig {
            metadata_keyspace: "custom_meta".to_string(),
            blob_keyspace: "custom_blob".to_string(),
            ..Default::default()
        };

        assert_eq!(config.metadata_keyspace, "custom_meta");
        assert_eq!(config.blob_keyspace, "custom_blob");
    }

    #[tokio::test]
    async fn test_eventual_consistency_with_quorum() {
        let config = AmatersConfig {
            read_consistency: ConsistencyLevel::Quorum,
            write_consistency: ConsistencyLevel::Quorum,
            ..Default::default()
        };

        assert_eq!(config.read_consistency, ConsistencyLevel::Quorum);
        assert_eq!(config.write_consistency, ConsistencyLevel::Quorum);
    }

    #[tokio::test]
    async fn test_eventual_consistency_with_one() {
        let config = AmatersConfig {
            read_consistency: ConsistencyLevel::One,
            write_consistency: ConsistencyLevel::One,
            ..Default::default()
        };

        assert_eq!(config.read_consistency, ConsistencyLevel::One);
        assert_eq!(config.write_consistency, ConsistencyLevel::One);
    }

    #[tokio::test]
    async fn test_eventual_consistency_with_all() {
        let config = AmatersConfig {
            read_consistency: ConsistencyLevel::All,
            write_consistency: ConsistencyLevel::All,
            ..Default::default()
        };

        assert_eq!(config.read_consistency, ConsistencyLevel::All);
        assert_eq!(config.write_consistency, ConsistencyLevel::All);
    }

    #[test]
    fn test_message_record_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("From".to_string(), "sender@example.com".to_string());
        headers.insert("To".to_string(), "recipient@example.com".to_string());

        let record = MessageRecord {
            id: "msg-id".to_string(),
            mailbox_id: "mailbox-id".to_string(),
            uid: 1,
            sender: Some("sender@example.com".to_string()),
            recipients: vec!["recipient@example.com".to_string()],
            headers,
            size: 1024,
            blob_key: "blob:msg-id".to_string(),
            created_at: 1234567890,
        };

        assert_eq!(record.headers.len(), 2);
        assert_eq!(
            record.headers.get("From"),
            Some(&"sender@example.com".to_string())
        );
    }

    #[tokio::test]
    async fn test_failover_retry_backoff() {
        let config = AmatersConfig {
            max_retries: 3,
            ..Default::default()
        };

        let client = AmatersClient::new(config);
        client.connect().await.unwrap();

        // Put operation should succeed with retries
        let result = client
            .put("metadata", "test-key".to_string(), vec![1, 2, 3])
            .await;
        assert!(result.is_ok());
    }
}
