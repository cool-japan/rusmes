//! SQLite storage backend for rusmes-storage.
//!
//! Provides a lightweight, file-based SQL backend backed by sqlx's SQLite driver.
//! Runs `sqlx::migrate!` from `./migrations` on first connect, so schema creation
//! is automatic and idempotent.
//!
//! Use this backend for single-node deployments, CI tests, or development where a
//! full PostgreSQL server is unavailable.

use crate::traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria, SpecialUseAttributes,
};
use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId, Username};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::str::FromStr;
use std::sync::Arc;

/// SQLite storage backend.
pub struct SqliteBackend {
    pool: SqlitePool,
}

impl SqliteBackend {
    /// Create a new SQLite backend from a file path.
    ///
    /// Runs `sqlx::migrate!("./migrations")` automatically on first connect so
    /// that the schema is always up-to-date.
    pub async fn new(path: &str) -> anyhow::Result<Self> {
        let opts = SqliteConnectOptions::from_str(path)
            .map_err(|e| anyhow::anyhow!("Invalid SQLite path '{}': {}", path, e))?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .map_err(|e| anyhow::anyhow!("SQLite connect failed: {}", e))?;

        // Run all pending migrations from the `migrations/` directory.
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("SQLite migration failed: {}", e))?;

        tracing::info!("SQLite backend ready: {}", path);
        Ok(Self { pool })
    }

    /// Expose the underlying pool for advanced use.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

impl StorageBackend for SqliteBackend {
    fn mailbox_store(&self) -> Arc<dyn MailboxStore> {
        Arc::new(SqliteMailboxStore {
            pool: self.pool.clone(),
        })
    }

    fn message_store(&self) -> Arc<dyn MessageStore> {
        Arc::new(SqliteMessageStore {
            pool: self.pool.clone(),
        })
    }

    fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        Arc::new(SqliteMetadataStore {
            pool: self.pool.clone(),
        })
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Mailbox store
// ──────────────────────────────────────────────────────────────────────────────

struct SqliteMailboxStore {
    pool: SqlitePool,
}

#[async_trait]
impl MailboxStore for SqliteMailboxStore {
    async fn create_mailbox(&self, path: &MailboxPath) -> anyhow::Result<MailboxId> {
        let mailbox = Mailbox::new(path.clone());
        let id = *mailbox.id();
        let id_str = id.as_uuid().to_string();

        sqlx::query(
            "INSERT INTO mailboxes (id, username, path, uid_validity, uid_next) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&id_str)
        .bind(path.user().to_string())
        .bind(path.path().join("/"))
        .bind(mailbox.uid_validity() as i64)
        .bind(mailbox.uid_next() as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("create_mailbox failed: {}", e))?;

        Ok(id)
    }

    async fn delete_mailbox(&self, id: &MailboxId) -> anyhow::Result<()> {
        let res = sqlx::query("DELETE FROM mailboxes WHERE id = ?1")
            .bind(id.as_uuid().to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("delete_mailbox failed: {}", e))?;

        if res.rows_affected() == 0 {
            return Err(anyhow::anyhow!("Mailbox not found: {}", id));
        }
        Ok(())
    }

    async fn rename_mailbox(&self, id: &MailboxId, new_path: &MailboxPath) -> anyhow::Result<()> {
        let res = sqlx::query("UPDATE mailboxes SET path = ?1 WHERE id = ?2")
            .bind(new_path.path().join("/"))
            .bind(id.as_uuid().to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("rename_mailbox failed: {}", e))?;

        if res.rows_affected() == 0 {
            return Err(anyhow::anyhow!("Mailbox not found: {}", id));
        }
        Ok(())
    }

    async fn get_mailbox(&self, id: &MailboxId) -> anyhow::Result<Option<Mailbox>> {
        let row = sqlx::query(
            "SELECT id, username, path, uid_validity, uid_next, special_use \
             FROM mailboxes WHERE id = ?1",
        )
        .bind(id.as_uuid().to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("get_mailbox failed: {}", e))?;

        match row {
            None => Ok(None),
            Some(r) => Ok(Some(row_to_mailbox(r)?)),
        }
    }

    async fn list_mailboxes(&self, user: &Username) -> anyhow::Result<Vec<Mailbox>> {
        let rows = sqlx::query(
            "SELECT id, username, path, uid_validity, uid_next, special_use \
             FROM mailboxes WHERE username = ?1 ORDER BY path",
        )
        .bind(user.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("list_mailboxes failed: {}", e))?;

        rows.into_iter()
            .map(row_to_mailbox)
            .collect::<anyhow::Result<Vec<_>>>()
    }

    async fn get_user_inbox(&self, user: &Username) -> anyhow::Result<Option<MailboxId>> {
        let row =
            sqlx::query("SELECT id FROM mailboxes WHERE username = ?1 AND path = 'INBOX' LIMIT 1")
                .bind(user.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| anyhow::anyhow!("get_user_inbox failed: {}", e))?;

        match row {
            None => Ok(None),
            Some(r) => {
                let id_str: String = r.try_get("id")?;
                let uuid = uuid::Uuid::from_str(&id_str)?;
                Ok(Some(MailboxId::from_uuid(uuid)))
            }
        }
    }

    async fn get_mailbox_special_use(
        &self,
        id: &MailboxId,
    ) -> anyhow::Result<SpecialUseAttributes> {
        let row = sqlx::query("SELECT special_use FROM mailboxes WHERE id = ?1")
            .bind(id.as_uuid().to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("get_mailbox_special_use failed: {}", e))?;

        match row {
            None => Ok(SpecialUseAttributes::new()),
            Some(r) => {
                let val: Option<String> = r.try_get("special_use")?;
                match val {
                    Some(s) if !s.is_empty() => {
                        let attrs: Vec<String> =
                            s.split_whitespace().map(|x| x.to_string()).collect();
                        Ok(SpecialUseAttributes::from_vec(attrs))
                    }
                    _ => Ok(SpecialUseAttributes::new()),
                }
            }
        }
    }

    async fn set_mailbox_special_use(
        &self,
        id: &MailboxId,
        special_use: SpecialUseAttributes,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE mailboxes SET special_use = ?1 WHERE id = ?2")
            .bind(special_use.to_string())
            .bind(id.as_uuid().to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("set_mailbox_special_use failed: {}", e))?;
        Ok(())
    }

    async fn subscribe_mailbox(&self, user: &Username, mailbox_name: String) -> anyhow::Result<()> {
        sqlx::query("INSERT OR IGNORE INTO subscriptions (username, mailbox_name) VALUES (?1, ?2)")
            .bind(user.to_string())
            .bind(mailbox_name)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("subscribe_mailbox failed: {}", e))?;
        Ok(())
    }

    async fn unsubscribe_mailbox(&self, user: &Username, mailbox_name: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM subscriptions WHERE username = ?1 AND mailbox_name = ?2")
            .bind(user.to_string())
            .bind(mailbox_name)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("unsubscribe_mailbox failed: {}", e))?;
        Ok(())
    }

    async fn list_subscriptions(&self, user: &Username) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query("SELECT mailbox_name FROM subscriptions WHERE username = ?1")
            .bind(user.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("list_subscriptions failed: {}", e))?;

        let subs = rows
            .into_iter()
            .filter_map(|r| r.try_get::<String, _>("mailbox_name").ok())
            .collect();
        Ok(subs)
    }
}

/// Convert a SQLite row to a `Mailbox`.
fn row_to_mailbox(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<Mailbox> {
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

// ──────────────────────────────────────────────────────────────────────────────
// Message store
// ──────────────────────────────────────────────────────────────────────────────

struct SqliteMessageStore {
    pool: SqlitePool,
}

#[async_trait]
impl MessageStore for SqliteMessageStore {
    async fn append_message(
        &self,
        mailbox_id: &MailboxId,
        message: Mail,
    ) -> anyhow::Result<MessageMetadata> {
        let message_id = *message.message_id();
        let mailbox_id_str = mailbox_id.as_uuid().to_string();
        let message_id_str = message_id.as_uuid().to_string();

        // Next UID — SQLite has no FOR UPDATE, so we use a transaction + SELECT.
        let mut tx = self.pool.begin().await?;

        let uid_row = sqlx::query("SELECT uid_next FROM mailboxes WHERE id = ?1")
            .bind(&mailbox_id_str)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("append_message: mailbox not found: {}", e))?;
        let uid: i64 = uid_row.try_get("uid_next")?;

        // Bump uid_next.
        sqlx::query("UPDATE mailboxes SET uid_next = uid_next + 1 WHERE id = ?1")
            .bind(&mailbox_id_str)
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("append_message: uid_next update failed: {}", e))?;

        let sender = message.sender().map(|s| s.to_string());
        let recipients: Vec<String> = message.recipients().iter().map(|r| r.to_string()).collect();
        let subject = message
            .message()
            .headers()
            .get_first("subject")
            .map(|s| s.to_string());

        // Serialize headers to JSON.
        let mut headers_map = serde_json::Map::new();
        for (name, values) in message.message().headers().iter() {
            headers_map.insert(
                name.to_string(),
                serde_json::Value::Array(
                    values
                        .iter()
                        .map(|v| serde_json::Value::String(v.to_string()))
                        .collect(),
                ),
            );
        }
        let headers_json = serde_json::to_string(&headers_map).unwrap_or_else(|_| "{}".to_string());
        let recipients_json =
            serde_json::to_string(&recipients).unwrap_or_else(|_| "[]".to_string());
        let message_size = message.size();

        sqlx::query(
            "INSERT INTO messages (id, mailbox_id, uid, sender, recipients, subject, headers, size) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&message_id_str)
        .bind(&mailbox_id_str)
        .bind(uid)
        .bind(&sender)
        .bind(&recipients_json)
        .bind(&subject)
        .bind(&headers_json)
        .bind(message_size as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("append_message: insert failed: {}", e))?;

        // Insert default flags row.
        sqlx::query("INSERT OR IGNORE INTO message_flags (message_id) VALUES (?1)")
            .bind(&message_id_str)
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("append_message: flags insert failed: {}", e))?;

        tx.commit().await?;

        let metadata = MessageMetadata::new(
            message_id,
            *mailbox_id,
            uid as u32,
            MessageFlags::new(),
            message_size,
        );
        Ok(metadata)
    }

    async fn get_message(&self, _message_id: &MessageId) -> anyhow::Result<Option<Mail>> {
        // SQLite backend stores metadata only; raw bytes are not stored inline here.
        // Return None — callers that need bytes should use the filesystem backend.
        Ok(None)
    }

    async fn delete_messages(&self, message_ids: &[MessageId]) -> anyhow::Result<()> {
        for id in message_ids {
            sqlx::query("DELETE FROM messages WHERE id = ?1")
                .bind(id.as_uuid().to_string())
                .execute(&self.pool)
                .await
                .map_err(|e| anyhow::anyhow!("delete_messages failed for {}: {}", id, e))?;
        }
        Ok(())
    }

    async fn set_flags(
        &self,
        message_ids: &[MessageId],
        flags: MessageFlags,
    ) -> anyhow::Result<()> {
        for id in message_ids {
            sqlx::query(
                "INSERT INTO message_flags \
                    (message_id, flag_seen, flag_answered, flag_flagged, flag_deleted, flag_draft) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(message_id) DO UPDATE SET \
                    flag_seen     = excluded.flag_seen,
                    flag_answered = excluded.flag_answered,
                    flag_flagged  = excluded.flag_flagged,
                    flag_deleted  = excluded.flag_deleted,
                    flag_draft    = excluded.flag_draft",
            )
            .bind(id.as_uuid().to_string())
            .bind(flags.is_seen() as i64)
            .bind(flags.is_answered() as i64)
            .bind(flags.is_flagged() as i64)
            .bind(flags.is_deleted() as i64)
            .bind(flags.is_draft() as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("set_flags failed for {}: {}", id, e))?;
        }
        Ok(())
    }

    async fn search(
        &self,
        mailbox_id: &MailboxId,
        _criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>> {
        // Minimal implementation: return all message IDs in the mailbox.
        let rows = sqlx::query("SELECT id FROM messages WHERE mailbox_id = ?1")
            .bind(mailbox_id.as_uuid().to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("search failed: {}", e))?;

        let ids = rows
            .into_iter()
            .filter_map(|r| {
                r.try_get::<String, _>("id")
                    .ok()
                    .and_then(|s| uuid::Uuid::from_str(&s).ok())
                    .map(MessageId::from_uuid)
            })
            .collect();
        Ok(ids)
    }

    async fn copy_messages(
        &self,
        message_ids: &[MessageId],
        dest_mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        // SQLite backend copies metadata rows only; no raw bytes to duplicate.
        let mut results = Vec::new();
        for &src_id in message_ids {
            // Look up the source message metadata.
            let row = sqlx::query("SELECT mailbox_id, uid, size FROM messages WHERE id = ?1")
                .bind(src_id.as_uuid().to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| anyhow::anyhow!("copy_messages: source lookup failed: {}", e))?;

            if let Some(r) = row {
                let size: i64 = r.try_get("size")?;

                // Allocate new UID in destination mailbox.
                let uid_row = sqlx::query("SELECT uid_next FROM mailboxes WHERE id = ?1")
                    .bind(dest_mailbox_id.as_uuid().to_string())
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|e| anyhow::anyhow!("copy_messages: dest uid fetch: {}", e))?;
                let uid: i64 = uid_row.try_get("uid_next")?;

                let new_id = MessageId::new();
                sqlx::query(
                    "INSERT INTO messages (id, mailbox_id, uid, sender, recipients, subject, headers, size) \
                     SELECT ?1, ?2, ?3, sender, recipients, subject, headers, size \
                     FROM messages WHERE id = ?4",
                )
                .bind(new_id.as_uuid().to_string())
                .bind(dest_mailbox_id.as_uuid().to_string())
                .bind(uid)
                .bind(src_id.as_uuid().to_string())
                .execute(&self.pool)
                .await
                .map_err(|e| anyhow::anyhow!("copy_messages: insert failed: {}", e))?;

                sqlx::query("UPDATE mailboxes SET uid_next = uid_next + 1 WHERE id = ?1")
                    .bind(dest_mailbox_id.as_uuid().to_string())
                    .execute(&self.pool)
                    .await
                    .map_err(|e| anyhow::anyhow!("copy_messages: uid_next bump: {}", e))?;

                results.push(MessageMetadata::new(
                    new_id,
                    *dest_mailbox_id,
                    uid as u32,
                    MessageFlags::new(),
                    size as usize,
                ));
            }
        }
        Ok(results)
    }

    async fn get_mailbox_messages(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        let rows = sqlx::query(
            "SELECT m.id, m.uid, m.size, \
                    COALESCE(f.flag_seen,     0) AS flag_seen, \
                    COALESCE(f.flag_answered, 0) AS flag_answered, \
                    COALESCE(f.flag_flagged,  0) AS flag_flagged, \
                    COALESCE(f.flag_deleted,  0) AS flag_deleted, \
                    COALESCE(f.flag_draft,    0) AS flag_draft \
             FROM messages m \
             LEFT JOIN message_flags f ON f.message_id = m.id \
             WHERE m.mailbox_id = ?1 \
             ORDER BY m.uid",
        )
        .bind(mailbox_id.as_uuid().to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("get_mailbox_messages failed: {}", e))?;

        let mut result = Vec::new();
        for r in rows {
            let id_str: String = r.try_get("id")?;
            let uuid = uuid::Uuid::from_str(&id_str)?;
            let msg_id = MessageId::from_uuid(uuid);
            let uid: i64 = r.try_get("uid")?;
            let size: i64 = r.try_get("size")?;

            let mut flags = MessageFlags::new();
            let flag_seen: i64 = r.try_get("flag_seen")?;
            let flag_answered: i64 = r.try_get("flag_answered")?;
            let flag_flagged: i64 = r.try_get("flag_flagged")?;
            let flag_deleted: i64 = r.try_get("flag_deleted")?;
            let flag_draft: i64 = r.try_get("flag_draft")?;
            flags.set_seen(flag_seen != 0);
            flags.set_answered(flag_answered != 0);
            flags.set_flagged(flag_flagged != 0);
            flags.set_deleted(flag_deleted != 0);
            flags.set_draft(flag_draft != 0);

            result.push(MessageMetadata::new(
                msg_id,
                *mailbox_id,
                uid as u32,
                flags,
                size as usize,
            ));
        }
        Ok(result)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Metadata store
// ──────────────────────────────────────────────────────────────────────────────

struct SqliteMetadataStore {
    pool: SqlitePool,
}

#[async_trait]
impl MetadataStore for SqliteMetadataStore {
    async fn get_user_quota(&self, user: &Username) -> anyhow::Result<Quota> {
        let row = sqlx::query("SELECT used, quota_limit FROM user_quotas WHERE username = ?1")
            .bind(user.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("get_user_quota failed: {}", e))?;

        match row {
            None => Ok(Quota::new(0, 1024 * 1024 * 1024)), // 1 GB default
            Some(r) => {
                let used: i64 = r.try_get("used")?;
                let limit: i64 = r.try_get("quota_limit")?;
                Ok(Quota::new(used as u64, limit as u64))
            }
        }
    }

    async fn set_user_quota(&self, user: &Username, quota: Quota) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO user_quotas (username, used, quota_limit) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(username) DO UPDATE SET used = excluded.used, quota_limit = excluded.quota_limit",
        )
        .bind(user.to_string())
        .bind(quota.used as i64)
        .bind(quota.limit as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("set_user_quota failed: {}", e))?;
        Ok(())
    }

    async fn get_mailbox_counters(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<MailboxCounters> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS total, \
                    SUM(CASE WHEN f.flag_seen = 0 OR f.flag_seen IS NULL THEN 1 ELSE 0 END) AS unseen \
             FROM messages m \
             LEFT JOIN message_flags f ON f.message_id = m.id \
             WHERE m.mailbox_id = ?1",
        )
        .bind(mailbox_id.as_uuid().to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("get_mailbox_counters failed: {}", e))?;

        let exists: i64 = row.try_get(0)?;
        let unseen: Option<i64> = row.try_get(1).ok();

        Ok(MailboxCounters {
            exists: exists as u32,
            recent: 0,
            unseen: unseen.unwrap_or(0) as u32,
        })
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_url(path: &str) -> String {
        format!("sqlite://{}?mode=rwc", path)
    }

    /// Verify that `SqliteBackend::new` runs migrations and creates the core tables.
    #[tokio::test]
    async fn test_migrations_sqlite_creates_tables() {
        let tmp =
            std::env::temp_dir().join(format!("rusmes-sqlite-test-{}.db", uuid::Uuid::new_v4()));
        let url = sqlite_url(tmp.to_str().expect("valid path"));

        let backend = SqliteBackend::new(&url)
            .await
            .expect("SqliteBackend::new should succeed");

        // Verify that the migration created the expected tables.
        for table in &[
            "mailboxes",
            "messages",
            "message_flags",
            "user_quotas",
            "subscriptions",
        ] {
            let row = sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name=?1")
                .bind(*table)
                .fetch_optional(backend.pool())
                .await
                .expect("query should succeed");

            assert!(
                row.is_some(),
                "Table '{}' should exist after migration",
                table
            );
        }

        let _ = tokio::fs::remove_file(&tmp).await;
    }

    /// Round-trip: create mailbox → append message → list messages.
    #[tokio::test]
    async fn test_sqlite_roundtrip_message() {
        use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};

        let tmp =
            std::env::temp_dir().join(format!("rusmes-sqlite-test-{}.db", uuid::Uuid::new_v4()));
        let url = sqlite_url(tmp.to_str().expect("valid path"));

        let backend = SqliteBackend::new(&url).await.expect("SqliteBackend::new");

        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();

        let user: Username = "sqlite-test-user".parse().expect("username");
        let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
        let mailbox_id = mailbox_store
            .create_mailbox(&path)
            .await
            .expect("create_mailbox");

        let headers = HeaderMap::new();
        let body = MessageBody::Small(bytes::Bytes::from("SQLite test body"));
        let mime = MimeMessage::new(headers, body);
        let mail = rusmes_proto::Mail::new(
            Some("sender@example.com".parse::<MailAddress>().expect("addr")),
            vec!["sqlite-test-user@localhost"
                .parse::<MailAddress>()
                .expect("addr")],
            mime,
            None,
            None,
        );

        let metadata = message_store
            .append_message(&mailbox_id, mail)
            .await
            .expect("append_message");

        assert_eq!(metadata.mailbox_id(), &mailbox_id);
        assert_eq!(metadata.uid(), 1);

        let messages = message_store
            .get_mailbox_messages(&mailbox_id)
            .await
            .expect("get_mailbox_messages");
        assert_eq!(messages.len(), 1);

        let _ = tokio::fs::remove_file(&tmp).await;
    }
}
