//! AmateRS mailbox store implementation.

use super::client::AmatersClient;
use super::records::MailboxRecord;
use crate::traits::MailboxStore;
use crate::types::{Mailbox, MailboxId, MailboxPath};
use async_trait::async_trait;
use rusmes_proto::Username;
use std::sync::Arc;

/// AmateRS mailbox store
pub(super) struct AmatersMailboxStore {
    pub(super) client: Arc<AmatersClient>,
    pub(super) keyspace: String,
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
