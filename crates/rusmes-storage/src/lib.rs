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
pub mod metrics;
pub mod modseq;
mod traits;
mod types;

pub use metrics::{Histogram, MetricsSummary, StorageMetrics, StorageTimer};
pub use modseq::{MailboxModSeq, MessageModSeq, ModSeq, ModSeqGenerator};
pub use traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
pub use types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria, SpecialUseAttributes,
};
