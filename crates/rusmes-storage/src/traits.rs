//! Storage abstraction traits

use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria, SpecialUseAttributes,
};
use crate::StorageEvent;
use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId, Username};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// Mailbox storage operations
#[async_trait]
pub trait MailboxStore: Send + Sync {
    /// Create a new mailbox
    async fn create_mailbox(&self, path: &MailboxPath) -> anyhow::Result<MailboxId>;

    /// Create a new mailbox with special-use attributes
    async fn create_mailbox_with_special_use(
        &self,
        path: &MailboxPath,
        special_use: SpecialUseAttributes,
    ) -> anyhow::Result<MailboxId> {
        // Default implementation creates mailbox and sets attributes separately
        let id = self.create_mailbox(path).await?;
        self.set_mailbox_special_use(&id, special_use).await?;
        Ok(id)
    }

    /// Delete a mailbox
    async fn delete_mailbox(&self, id: &MailboxId) -> anyhow::Result<()>;

    /// Rename a mailbox
    async fn rename_mailbox(&self, id: &MailboxId, new_path: &MailboxPath) -> anyhow::Result<()>;

    /// Get mailbox by ID
    async fn get_mailbox(&self, id: &MailboxId) -> anyhow::Result<Option<Mailbox>>;

    /// List all mailboxes for a user
    async fn list_mailboxes(&self, user: &Username) -> anyhow::Result<Vec<Mailbox>>;

    /// Get a user's INBOX mailbox ID (primary mailbox)
    async fn get_user_inbox(&self, user: &Username) -> anyhow::Result<Option<MailboxId>>;

    /// Get mailbox special-use attributes
    async fn get_mailbox_special_use(
        &self,
        _id: &MailboxId,
    ) -> anyhow::Result<SpecialUseAttributes> {
        // Default implementation returns empty attributes
        Ok(SpecialUseAttributes::new())
    }

    /// Set mailbox special-use attributes
    async fn set_mailbox_special_use(
        &self,
        id: &MailboxId,
        special_use: SpecialUseAttributes,
    ) -> anyhow::Result<()> {
        // Default implementation does nothing
        let _ = (id, special_use);
        Ok(())
    }

    /// List mailboxes with a specific special-use attribute
    async fn list_mailboxes_by_special_use(
        &self,
        user: &Username,
        special_use: &str,
    ) -> anyhow::Result<Vec<Mailbox>> {
        // Default implementation filters all mailboxes
        let mailboxes = self.list_mailboxes(user).await?;
        let mut result = Vec::new();
        for mailbox in mailboxes {
            let attrs = self.get_mailbox_special_use(mailbox.id()).await?;
            if attrs.has_attribute(special_use) {
                result.push(mailbox);
            }
        }
        Ok(result)
    }

    /// Subscribe to a mailbox
    async fn subscribe_mailbox(&self, user: &Username, mailbox_name: String) -> anyhow::Result<()>;

    /// Unsubscribe from a mailbox
    async fn unsubscribe_mailbox(&self, user: &Username, mailbox_name: &str) -> anyhow::Result<()>;

    /// List subscribed mailboxes
    async fn list_subscriptions(&self, user: &Username) -> anyhow::Result<Vec<String>>;
}

/// Message storage operations
#[async_trait]
pub trait MessageStore: Send + Sync {
    /// Append a message to a mailbox
    async fn append_message(
        &self,
        mailbox_id: &MailboxId,
        message: Mail,
    ) -> anyhow::Result<MessageMetadata>;

    /// Get a message by ID
    async fn get_message(&self, message_id: &MessageId) -> anyhow::Result<Option<Mail>>;

    /// Delete messages
    async fn delete_messages(&self, message_ids: &[MessageId]) -> anyhow::Result<()>;

    /// Set flags on messages
    async fn set_flags(&self, message_ids: &[MessageId], flags: MessageFlags)
        -> anyhow::Result<()>;

    /// Search messages in a mailbox
    async fn search(
        &self,
        mailbox_id: &MailboxId,
        criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>>;

    /// Copy messages to another mailbox
    async fn copy_messages(
        &self,
        message_ids: &[MessageId],
        dest_mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>>;

    /// Get all message metadata for a mailbox
    async fn get_mailbox_messages(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>>;

    /// Get the current flags for a message by scanning all mailboxes.
    ///
    /// Returns `Ok(None)` when the message is not found.  The default
    /// implementation scans `get_mailbox_messages` for every known mailbox,
    /// which may be expensive on backends with many mailboxes.  Implementations
    /// that can answer this query more efficiently should override this method.
    ///
    /// IMPORTANT: callers must supply the full list of mailbox IDs to search.
    /// Because `MessageStore` does not track the global mailbox index, this
    /// default implementation cannot enumerate mailboxes on its own.  The
    /// default simply returns `Ok(None)`.  Backends that can enumerate their
    /// own mailboxes (e.g. `FilesystemMessageStore`) override this method.
    async fn get_message_flags(
        &self,
        _message_id: &MessageId,
    ) -> anyhow::Result<Option<MessageFlags>> {
        Ok(None)
    }

    /// Look up the RFC 5256 thread ID for a stored message.
    ///
    /// Returns `Ok(None)` for backends that do not implement threading, or when
    /// the message is not found in the thread index. The filesystem backend
    /// overrides this by scanning mailbox directories.
    async fn get_message_thread_id(
        &self,
        _message_id: &MessageId,
    ) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

/// Metadata storage operations
#[async_trait]
pub trait MetadataStore: Send + Sync {
    /// Get user quota
    async fn get_user_quota(&self, user: &Username) -> anyhow::Result<Quota>;

    /// Set user quota
    async fn set_user_quota(&self, user: &Username, quota: Quota) -> anyhow::Result<()>;

    /// Get mailbox counters
    async fn get_mailbox_counters(&self, mailbox_id: &MailboxId)
        -> anyhow::Result<MailboxCounters>;
}

/// Combined storage backend
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Get mailbox store
    fn mailbox_store(&self) -> Arc<dyn MailboxStore>;

    /// Get message store
    fn message_store(&self) -> Arc<dyn MessageStore>;

    /// Get metadata store
    fn metadata_store(&self) -> Arc<dyn MetadataStore>;

    /// Subscribe to storage events.
    ///
    /// The default implementation returns a receiver from a channel whose
    /// sender is immediately dropped, so the receiver will never yield events.
    /// Backends that support events (filesystem) override this method.
    fn event_stream(&self) -> broadcast::Receiver<StorageEvent> {
        let (tx, rx) = broadcast::channel(1);
        drop(tx);
        rx
    }

    /// Remove expunged messages older than `older_than`.
    ///
    /// Returns the number of messages removed. The default implementation is
    /// a no-op returning 0. Each concrete backend overrides this with its own
    /// compaction strategy.
    async fn compact_expunged(&self, _older_than: Duration) -> anyhow::Result<usize> {
        Ok(0)
    }

    /// Return the filesystem base path if this is a filesystem backend.
    ///
    /// Used by the backup/restore library API. Non-filesystem backends return `None`.
    fn as_filesystem_path(&self) -> Option<&std::path::Path> {
        None
    }

    /// List every user known to this backend.
    ///
    /// Used by maintenance code (e.g. rusmes-search rebuild) that needs to walk
    /// every mailbox in the system. The default implementation returns an empty
    /// vector; backends that can enumerate users override this.
    async fn list_all_users(&self) -> anyhow::Result<Vec<Username>> {
        Ok(Vec::new())
    }
}
