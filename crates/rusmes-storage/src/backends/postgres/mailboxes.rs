//! PostgreSQL mailbox store implementation.

use crate::traits::MailboxStore;
use crate::types::{Mailbox, MailboxId, MailboxPath, SpecialUseAttributes};
use async_trait::async_trait;
use rusmes_proto::Username;
use sqlx::postgres::{PgPool, PgRow};
use sqlx::Row;

/// PostgreSQL mailbox store implementation
pub(super) struct PostgresMailboxStore {
    pub(super) pool: PgPool,
}

#[async_trait]
impl MailboxStore for PostgresMailboxStore {
    async fn create_mailbox(&self, path: &MailboxPath) -> anyhow::Result<MailboxId> {
        let mailbox = Mailbox::new(path.clone());
        let id = *mailbox.id();

        sqlx::query(
            r#"
            INSERT INTO mailboxes (id, username, path, uid_validity, uid_next, special_use)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(*id.as_uuid())
        .bind(path.user().to_string())
        .bind(path.path().join("/"))
        .bind(mailbox.uid_validity() as i32)
        .bind(mailbox.uid_next() as i32)
        .bind(mailbox.special_use())
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create mailbox: {}", e))?;

        tracing::debug!("Created mailbox {} at path {}", id, path);
        Ok(id)
    }

    async fn create_mailbox_with_special_use(
        &self,
        path: &MailboxPath,
        special_use: SpecialUseAttributes,
    ) -> anyhow::Result<MailboxId> {
        let mailbox = Mailbox::new(path.clone());
        let id = *mailbox.id();
        let special_use_str = special_use.to_string();

        sqlx::query(
            r#"
            INSERT INTO mailboxes (id, username, path, uid_validity, uid_next, special_use)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(*id.as_uuid())
        .bind(path.user().to_string())
        .bind(path.path().join("/"))
        .bind(mailbox.uid_validity() as i32)
        .bind(mailbox.uid_next() as i32)
        .bind(special_use_str)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create mailbox with special use: {}", e))?;

        tracing::debug!("Created mailbox {} with special use at path {}", id, path);
        Ok(id)
    }

    async fn delete_mailbox(&self, id: &MailboxId) -> anyhow::Result<()> {
        let result = sqlx::query("DELETE FROM mailboxes WHERE id = $1")
            .bind(*id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete mailbox: {}", e))?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("Mailbox not found: {}", id));
        }

        tracing::debug!("Deleted mailbox {}", id);
        Ok(())
    }

    async fn rename_mailbox(&self, id: &MailboxId, new_path: &MailboxPath) -> anyhow::Result<()> {
        let result =
            sqlx::query("UPDATE mailboxes SET path = $1, updated_at = NOW() WHERE id = $2")
                .bind(new_path.path().join("/"))
                .bind(*id.as_uuid())
                .execute(&self.pool)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to rename mailbox: {}", e))?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("Mailbox not found: {}", id));
        }

        tracing::debug!("Renamed mailbox {} to {}", id, new_path);
        Ok(())
    }

    async fn get_mailbox(&self, id: &MailboxId) -> anyhow::Result<Option<Mailbox>> {
        let row = sqlx::query(
            "SELECT id, username, path, uid_validity, uid_next, special_use FROM mailboxes WHERE id = $1"
        )
        .bind(*id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get mailbox: {}", e))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let mailbox = row_to_mailbox(row)?;
        Ok(Some(mailbox))
    }

    async fn list_mailboxes(&self, user: &Username) -> anyhow::Result<Vec<Mailbox>> {
        let rows = sqlx::query(
            "SELECT id, username, path, uid_validity, uid_next, special_use FROM mailboxes WHERE username = $1 ORDER BY path"
        )
        .bind(user.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to list mailboxes: {}", e))?;

        let mailboxes = rows
            .into_iter()
            .filter_map(|row| row_to_mailbox(row).ok())
            .collect();

        Ok(mailboxes)
    }

    async fn get_user_inbox(&self, user: &Username) -> anyhow::Result<Option<MailboxId>> {
        let row =
            sqlx::query("SELECT id FROM mailboxes WHERE username = $1 AND path = 'INBOX' LIMIT 1")
                .bind(user.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get user inbox: {}", e))?;

        if let Some(row) = row {
            let uuid: uuid::Uuid = row.get(0);
            Ok(Some(MailboxId::from_uuid(uuid)))
        } else {
            Ok(None)
        }
    }

    async fn get_mailbox_special_use(
        &self,
        id: &MailboxId,
    ) -> anyhow::Result<SpecialUseAttributes> {
        let row = sqlx::query("SELECT special_use FROM mailboxes WHERE id = $1")
            .bind(*id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get special use: {}", e))?;

        match row {
            Some(r) => {
                let special_use: Option<String> = r.try_get("special_use")?;
                match special_use {
                    Some(s) if !s.is_empty() => {
                        let attrs = s
                            .split_whitespace()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>();
                        Ok(SpecialUseAttributes::from_vec(attrs))
                    }
                    _ => Ok(SpecialUseAttributes::new()),
                }
            }
            None => Ok(SpecialUseAttributes::new()),
        }
    }

    async fn set_mailbox_special_use(
        &self,
        id: &MailboxId,
        special_use: SpecialUseAttributes,
    ) -> anyhow::Result<()> {
        let special_use_str = special_use.to_string();

        sqlx::query("UPDATE mailboxes SET special_use = $1, updated_at = NOW() WHERE id = $2")
            .bind(special_use_str)
            .bind(*id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to set special use: {}", e))?;

        Ok(())
    }

    async fn subscribe_mailbox(&self, user: &Username, mailbox_name: String) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO subscriptions (username, mailbox_name) VALUES ($1, $2) ON CONFLICT DO NOTHING"
        )
        .bind(user.to_string())
        .bind(mailbox_name)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to subscribe: {}", e))?;

        Ok(())
    }

    async fn unsubscribe_mailbox(&self, user: &Username, mailbox_name: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM subscriptions WHERE username = $1 AND mailbox_name = $2")
            .bind(user.to_string())
            .bind(mailbox_name)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to unsubscribe: {}", e))?;

        Ok(())
    }

    async fn list_subscriptions(&self, user: &Username) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query("SELECT mailbox_name FROM subscriptions WHERE username = $1")
            .bind(user.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list subscriptions: {}", e))?;

        let subscriptions = rows
            .into_iter()
            .filter_map(|row| row.try_get("mailbox_name").ok())
            .collect();

        Ok(subscriptions)
    }
}

fn row_to_mailbox(row: PgRow) -> anyhow::Result<Mailbox> {
    let username: String = row.try_get("username")?;
    let path_str: String = row.try_get("path")?;
    let path_parts: Vec<String> = path_str.split('/').map(|s| s.to_string()).collect();
    let username_obj =
        Username::new(username).map_err(|e| anyhow::anyhow!("Invalid username: {}", e))?;
    let path = MailboxPath::new(username_obj, path_parts);

    let mut mailbox = Mailbox::new(path);
    let special_use: Option<String> = row.try_get("special_use")?;
    mailbox.set_special_use(special_use);

    Ok(mailbox)
}
