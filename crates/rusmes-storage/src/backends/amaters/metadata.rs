//! AmateRS metadata store implementation.

use super::client::AmatersClient;
use crate::traits::MetadataStore;
use crate::types::{MailboxCounters, MailboxId, Quota};
use async_trait::async_trait;
use rusmes_proto::Username;
use std::sync::Arc;

/// AmateRS metadata store
pub(super) struct AmatersMetadataStore {
    pub(super) client: Arc<AmatersClient>,
    pub(super) keyspace: String,
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
