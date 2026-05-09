//! AmateRS distributed storage backend
//!
//! This backend provides distributed message storage using AmateRS, a Fully Homomorphic
//! Encrypted distributed key-value store. When compiled with the `amaters-backend`
//! feature the real `amaters-sdk-rust v0.2` gRPC client is used; without the feature
//! flag an in-memory mock (HashMap) is used for development and testing.
//!
//! # Features
//!
//! - **Distributed blob storage**: Message bodies stored separately from metadata
//! - **Replication**: Configurable replication factor (default: 3)
//! - **Consistency levels**: Support for ONE/QUORUM/ALL/LocalQuorum
//! - **Circuit breaker**: Automatic failover on node failures (mock path)
//! - **Retry logic**: Built-in exponential backoff in the SDK (real path)
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
//! # Real client integration
//!
//! Build with `--features amaters-backend` (or `--all-features`) to activate the
//! real `amaters-sdk-rust` gRPC client.  Use `AmatersBackend::connect_real(config)`
//! to construct a backend that connects to a live AmateRS cluster.  The mock path
//! (`AmatersBackend::new`) remains available for testing without a server.

mod circuit_breaker;
mod client;
pub mod config;
mod mailboxes;
mod messages;
mod metadata;
mod records;
#[cfg(test)]
mod tests;

pub use config::{AmatersConfig, ConsistencyLevel};

use crate::traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
use client::AmatersClient;
use mailboxes::AmatersMailboxStore;
use messages::AmatersMessageStore;
use metadata::AmatersMetadataStore;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Per-mailbox next-UID map.
///
/// Each entry is `Arc<Mutex<u32>>` where the `u32` is the *next* UID to hand
/// out for that mailbox.  The outer `Arc<Mutex<...>>` guards the map itself
/// (needed only to insert new entries); once an entry exists the outer lock is
/// released and only the inner per-mailbox mutex is held during allocation.
///
/// This is **Strategy B** (in-process mutex) because the AmateRS SDK v0.2 has
/// no compare-and-swap primitive — only `set/get/delete/range`.
pub(in crate::backends::amaters) type UidLockMap = Arc<Mutex<HashMap<String, Arc<Mutex<u32>>>>>;

/// AmateRS distributed storage backend
pub struct AmatersBackend {
    pub(in crate::backends::amaters) client: Arc<AmatersClient>,
    pub(in crate::backends::amaters) config: AmatersConfig,
    /// Per-mailbox next-UID mutexes.  See [`UidLockMap`].
    pub(in crate::backends::amaters) uid_locks: UidLockMap,
}

impl AmatersBackend {
    /// Create a new AmateRS backend backed by the in-memory mock.
    ///
    /// This is safe to use without a live AmateRS server — all data lives in
    /// process memory.  Suitable for development and unit tests.
    pub async fn new(config: AmatersConfig) -> anyhow::Result<Self> {
        let client = Arc::new(AmatersClient::new(config.clone()));
        client.connect().await?;
        client.init_keyspaces().await?;

        Ok(Self {
            client,
            config,
            uid_locks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Create a new AmateRS backend connected to a real AmateRS cluster.
    ///
    /// Requires the `amaters-backend` feature.  Connects to the cluster
    /// specified in `config.cluster_endpoints` using the `amaters-sdk-rust`
    /// gRPC client.
    #[cfg(feature = "amaters-backend")]
    pub async fn connect_real(config: AmatersConfig) -> anyhow::Result<Self> {
        let client = Arc::new(AmatersClient::new_real(&config).await?);
        Ok(Self {
            client,
            config,
            uid_locks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Initialize schema
    pub async fn init_schema(&self) -> anyhow::Result<()> {
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
            uid_locks: self.uid_locks.clone(),
        })
    }

    fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        Arc::new(AmatersMetadataStore {
            client: self.client.clone(),
            keyspace: self.config.metadata_keyspace.clone(),
        })
    }
}
