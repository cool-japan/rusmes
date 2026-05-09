//! Storage abstraction layer for RusMES
//!
//! This crate provides a unified storage interface for the RusMES mail server, covering
//! mailbox management, message delivery, metadata tracking, and quota enforcement.
//!
//! # Architecture
//!
//! The storage layer is composed of three orthogonal trait abstractions:
//!
//! - [`MailboxStore`] — Create, rename, delete, subscribe, and list mailboxes per user.
//! - [`MessageStore`] — Append, retrieve, copy, flag, search, and delete messages.
//! - [`MetadataStore`] — Manage per-user quotas and per-mailbox counters (exists/recent/unseen).
//!
//! These three stores are obtained from a single [`StorageBackend`] factory, which bundles
//! them together and can be shared across protocol handlers (IMAP, JMAP, POP3, SMTP).
//!
//! # Backends
//!
//! | Backend | Module | Notes |
//! |---------|--------|-------|
//! | Filesystem / Maildir | [`backends::filesystem`] | Atomic delivery via `tmp/` → `new/` rename; flag encoding in filenames |
//! | AmateRS distributed KV | [`backends::amaters`] | Mock implementation with circuit-breaker and exponential-backoff retry logic |
//! | PostgreSQL | [`backends::postgres`] | Full connection pool via `sqlx`, full-text search, migrations |
//!
//! # Example — Filesystem backend
//!
//! ```rust,no_run
//! use rusmes_storage::{StorageBackend, MailboxStore, MailboxPath};
//! use rusmes_storage::backends::filesystem::FilesystemBackend;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let backend = FilesystemBackend::new("/var/mail/rusmes");
//! let mb_store = backend.mailbox_store();
//!
//! let user: rusmes_proto::Username = "alice@example.com".parse()?;
//! let path = MailboxPath::new(user, vec!["INBOX".to_string()]);
//! let id = mb_store.create_mailbox(&path).await?;
//!
//! let mailbox = mb_store.get_mailbox(&id).await?;
//! println!("Created: {:?}", mailbox);
//! # Ok(())
//! # }
//! ```
//!
//! # ModSeq (Modification Sequence Numbers)
//!
//! [`ModSeqGenerator`] produces monotonically increasing sequence numbers suitable for
//! IMAP CONDSTORE / QRESYNC extensions.  Both mailbox-level and message-level ModSeq
//! values are tracked via [`MailboxModSeq`] and [`MessageModSeq`] wrappers.
//!
//! # Metrics
//!
//! [`StorageMetrics`] records per-operation histograms for timing storage calls from
//! higher-level protocol handlers, enabling Prometheus-compatible export.

pub mod backends;
pub mod backup;
pub mod metrics;
pub mod modseq;
mod traits;
mod types;

pub use backup::{backup, restore};
pub use metrics::{Histogram, MetricsSummary, StorageMetrics, StorageTimer};
pub use modseq::{MailboxModSeq, MessageModSeq, ModSeq, ModSeqGenerator};
pub use traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
pub use types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria, SpecialUseAttributes,
};

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Events emitted by storage backends after write operations.
///
/// Consumed by rusmes-search (Cluster 9) for incremental indexing and by
/// any other subscriber that needs to react to mailbox changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageEvent {
    /// A message was stored in a mailbox.
    MessageStored {
        account: String,
        mailbox: String,
        uid: u32,
    },
    /// A message was expunged (permanently deleted) from a mailbox.
    MessageExpunged {
        account: String,
        mailbox: String,
        uid: u32,
    },
}

/// Configuration for which storage backend to build.
#[derive(Debug, Clone)]
pub enum BackendKind {
    /// Filesystem (maildir) backend.
    Filesystem { path: String },
    /// SQLite backend (file path, e.g. `"sqlite:///var/mail/rusmes.db?mode=rwc"`).
    Sqlite { connection_string: String },
    /// PostgreSQL backend.
    Postgres { connection_string: String },
    /// AmateRS distributed backend (mock only).
    Amaters {
        endpoints: Vec<String>,
        replication_factor: usize,
    },
}

/// Construct a storage backend from configuration.
///
/// For SQL backends, migrations are run before returning.
/// For the PostgreSQL backend, a background VACUUM scheduler is also started
/// with the default 24-hour interval.
pub async fn build_storage(kind: &BackendKind) -> anyhow::Result<Arc<dyn StorageBackend>> {
    match kind {
        BackendKind::Filesystem { path } => {
            use backends::filesystem::FilesystemBackend;
            let backend = FilesystemBackend::new(path);
            Ok(Arc::new(backend))
        }
        BackendKind::Sqlite { connection_string } => {
            use backends::sqlite::SqliteBackend;
            // SqliteBackend::new runs migrations automatically.
            let backend = SqliteBackend::new(connection_string).await?;
            Ok(Arc::new(backend))
        }
        BackendKind::Postgres { connection_string } => {
            use backends::postgres::PostgresBackend;
            // with_config starts the background VACUUM task automatically.
            let backend = PostgresBackend::new(connection_string).await?;
            // Also run the hand-rolled idempotent migration DDL for backwards compat.
            backend.init_schema().await?;
            Ok(Arc::new(backend))
        }
        BackendKind::Amaters {
            endpoints,
            replication_factor,
        } => {
            use backends::amaters::{AmatersBackend, AmatersConfig};
            let config = AmatersConfig {
                cluster_endpoints: endpoints.clone(),
                replication_factor: *replication_factor,
                ..Default::default()
            };
            let backend = AmatersBackend::new(config).await?;
            Ok(Arc::new(backend))
        }
    }
}

/// Perform compaction: remove expunged messages older than `older_than`.
///
/// Dispatches to `backend.compact_expunged(older_than)`.
pub async fn compact_expunged(
    backend: &dyn StorageBackend,
    older_than: Duration,
) -> anyhow::Result<usize> {
    backend.compact_expunged(older_than).await
}
