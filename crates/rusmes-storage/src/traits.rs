//! Storage abstraction traits

use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria, SpecialUseAttributes,
};
use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId, Username};
use std::sync::Arc;

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
pub trait StorageBackend: Send + Sync {
    /// Get mailbox store
    fn mailbox_store(&self) -> Arc<dyn MailboxStore>;

    /// Get message store
    fn message_store(&self) -> Arc<dyn MessageStore>;

    /// Get metadata store
    fn metadata_store(&self) -> Arc<dyn MetadataStore>;
}
