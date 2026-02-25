//! Mailbox change detection for IDLE support

use rusmes_storage::{MailboxId, MetadataStore};
use std::sync::Arc;

/// Mailbox change information
#[derive(Debug, Clone, PartialEq)]
pub struct MailboxChanges {
    pub exists: u32,
    pub recent: u32,
}

impl MailboxChanges {
    /// Create new mailbox changes
    pub fn new(exists: u32, recent: u32) -> Self {
        Self { exists, recent }
    }

    /// Check if there are differences from snapshot
    pub fn has_changes(&self, snapshot: &MailboxChanges) -> bool {
        self.exists != snapshot.exists || self.recent != snapshot.recent
    }
}

/// Mailbox watcher for detecting changes
pub struct MailboxWatcher {
    metadata_store: Arc<dyn MetadataStore>,
}

impl MailboxWatcher {
    /// Create a new mailbox watcher
    pub fn new(metadata_store: Arc<dyn MetadataStore>) -> Self {
        Self { metadata_store }
    }

    /// Get current mailbox state
    pub async fn get_mailbox_state(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<MailboxChanges> {
        let counters = self.metadata_store.get_mailbox_counters(mailbox_id).await?;
        Ok(MailboxChanges::new(counters.exists, counters.recent))
    }
}
