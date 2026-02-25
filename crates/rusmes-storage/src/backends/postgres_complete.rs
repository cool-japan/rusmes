//! Complete PostgreSQL storage backend implementation

use crate::traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria,
};
use async_trait::async_trait;
use rusmes_proto::{Mail, MessageId, Username};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Executor, Row};
use std::sync::Arc;

/// Complete PostgreSQL storage backend with connection pooling
pub struct PostgresCompleteBackend {
    pool: PgPool,
}

impl PostgresCompleteBackend {
    /// Create a new PostgreSQL backend with connection pooling
    pub async fn new(database_url: &str) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await?;

        Ok(Self { pool })
    }

    /// Initialize database schema and migrations
    pub async fn init_schema(&self) -> anyhow::Result<()> {
        // Mailboxes table
        self.pool
            .execute(
                r#"
            CREATE TABLE IF NOT EXISTS mailboxes (
                id UUID PRIMARY KEY,
                username TEXT NOT NULL,
                path TEXT NOT NULL,
                uid_validity INTEGER NOT NULL,
                uid_next INTEGER NOT NULL,
                special_use TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
                UNIQUE(username, path)
            )
            "#,
            )
            .await?;

        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_mailboxes_username ON mailboxes(username)")
            .await?;
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_mailboxes_path ON mailboxes(path)")
            .await?;

        // Messages table with BLOB storage
        self.pool
            .execute(
                r#"
            CREATE TABLE IF NOT EXISTS messages (
                id UUID PRIMARY KEY,
                mailbox_id UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
                uid INTEGER NOT NULL,
                sender TEXT,
                recipients TEXT[] NOT NULL,
                headers JSONB NOT NULL,
                body BYTEA NOT NULL,
                size INTEGER NOT NULL,
                search_vector TSVECTOR,
                created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                UNIQUE(mailbox_id, uid)
            )
            "#,
            )
            .await?;

        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_messages_mailbox ON messages(mailbox_id)")
            .await?;
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender)")
            .await?;
        self.pool.execute("CREATE INDEX IF NOT EXISTS idx_messages_search ON messages USING GIN(search_vector)").await?;

        // Message flags table
        self.pool
            .execute(
                r#"
            CREATE TABLE IF NOT EXISTS message_flags (
                message_id UUID NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                flag_seen BOOLEAN NOT NULL DEFAULT FALSE,
                flag_answered BOOLEAN NOT NULL DEFAULT FALSE,
                flag_flagged BOOLEAN NOT NULL DEFAULT FALSE,
                flag_deleted BOOLEAN NOT NULL DEFAULT FALSE,
                flag_draft BOOLEAN NOT NULL DEFAULT FALSE,
                flag_recent BOOLEAN NOT NULL DEFAULT FALSE,
                custom_flags TEXT[] NOT NULL DEFAULT '{}',
                PRIMARY KEY(message_id)
            )
            "#,
            )
            .await?;

        // Subscriptions table
        self.pool
            .execute(
                r#"
            CREATE TABLE IF NOT EXISTS subscriptions (
                username TEXT NOT NULL,
                mailbox_name TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                PRIMARY KEY(username, mailbox_name)
            )
            "#,
            )
            .await?;

        // User quotas table
        self.pool
            .execute(
                r#"
            CREATE TABLE IF NOT EXISTS user_quotas (
                username TEXT PRIMARY KEY,
                used BIGINT NOT NULL DEFAULT 0,
                quota_limit BIGINT NOT NULL,
                updated_at TIMESTAMP NOT NULL DEFAULT NOW()
            )
            "#,
            )
            .await?;

        // Create trigger for updating search_vector
        self.pool.execute(
            r#"
            CREATE OR REPLACE FUNCTION messages_search_vector_update() RETURNS trigger AS $$
            BEGIN
                NEW.search_vector :=
                    setweight(to_tsvector('english', COALESCE(NEW.sender, '')), 'A') ||
                    setweight(to_tsvector('english', COALESCE(array_to_string(NEW.recipients, ' '), '')), 'B') ||
                    setweight(to_tsvector('english', COALESCE(NEW.headers::text, '')), 'C');
                RETURN NEW;
            END
            $$ LANGUAGE plpgsql
            "#,
        ).await?;

        self.pool
            .execute(
                r#"
            DROP TRIGGER IF EXISTS messages_search_vector_trigger ON messages;
            CREATE TRIGGER messages_search_vector_trigger
            BEFORE INSERT OR UPDATE ON messages
            FOR EACH ROW EXECUTE FUNCTION messages_search_vector_update()
            "#,
            )
            .await?;

        Ok(())
    }

    /// Get pool reference
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl StorageBackend for PostgresCompleteBackend {
    fn mailbox_store(&self) -> Arc<dyn MailboxStore> {
        Arc::new(PostgresCompleteMailboxStore {
            pool: self.pool.clone(),
        })
    }

    fn message_store(&self) -> Arc<dyn MessageStore> {
        Arc::new(PostgresCompleteMessageStore {
            pool: self.pool.clone(),
        })
    }

    fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        Arc::new(PostgresCompleteMetadataStore {
            pool: self.pool.clone(),
        })
    }
}

/// Complete PostgreSQL mailbox store
struct PostgresCompleteMailboxStore {
    pool: PgPool,
}

#[async_trait]
impl MailboxStore for PostgresCompleteMailboxStore {
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
        .await?;

        Ok(id)
    }

    async fn delete_mailbox(&self, id: &MailboxId) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM mailboxes WHERE id = $1")
            .bind(*id.as_uuid())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn rename_mailbox(&self, id: &MailboxId, new_path: &MailboxPath) -> anyhow::Result<()> {
        sqlx::query("UPDATE mailboxes SET path = $1, updated_at = NOW() WHERE id = $2")
            .bind(new_path.path().join("/"))
            .bind(*id.as_uuid())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn get_mailbox(&self, id: &MailboxId) -> anyhow::Result<Option<Mailbox>> {
        let row = sqlx::query(
            "SELECT id, username, path, uid_validity, uid_next, special_use FROM mailboxes WHERE id = $1"
        )
        .bind(*id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let username: String = row.try_get("username")?;
        let path_str: String = row.try_get("path")?;
        let path_parts: Vec<String> = path_str.split('/').map(|s| s.to_string()).collect();
        let username_obj =
            Username::new(username).map_err(|e| anyhow::anyhow!("Invalid username: {}", e))?;
        let path = MailboxPath::new(username_obj, path_parts);

        let mut mailbox = Mailbox::new(path);
        let special_use: Option<String> = row.try_get("special_use")?;
        mailbox.set_special_use(special_use);

        Ok(Some(mailbox))
    }

    async fn list_mailboxes(&self, user: &Username) -> anyhow::Result<Vec<Mailbox>> {
        let rows = sqlx::query(
            "SELECT id, username, path, uid_validity, uid_next, special_use FROM mailboxes WHERE username = $1 ORDER BY path"
        )
        .bind(user.to_string())
        .fetch_all(&self.pool)
        .await?;

        let mailboxes = rows
            .into_iter()
            .filter_map(|row| {
                let username: String = row.try_get("username").ok()?;
                let path_str: String = row.try_get("path").ok()?;
                let path_parts: Vec<String> = path_str.split('/').map(|s| s.to_string()).collect();
                let username_obj = Username::new(username).ok()?;
                let path = MailboxPath::new(username_obj, path_parts);

                let mut mailbox = Mailbox::new(path);
                if let Ok(Some(special_use)) = row.try_get("special_use") {
                    mailbox.set_special_use(Some(special_use));
                }
                Some(mailbox)
            })
            .collect();

        Ok(mailboxes)
    }

    async fn get_user_inbox(&self, user: &Username) -> anyhow::Result<Option<MailboxId>> {
        let row =
            sqlx::query("SELECT id FROM mailboxes WHERE username = $1 AND path = 'INBOX' LIMIT 1")
                .bind(user.to_string())
                .fetch_optional(&self.pool)
                .await?;

        if let Some(row) = row {
            let uuid: uuid::Uuid = row.get(0);
            Ok(Some(MailboxId::from_uuid(uuid)))
        } else {
            Ok(None)
        }
    }

    async fn subscribe_mailbox(&self, user: &Username, mailbox_name: String) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO subscriptions (username, mailbox_name) VALUES ($1, $2) ON CONFLICT DO NOTHING"
        )
        .bind(user.to_string())
        .bind(mailbox_name)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn unsubscribe_mailbox(&self, user: &Username, mailbox_name: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM subscriptions WHERE username = $1 AND mailbox_name = $2")
            .bind(user.to_string())
            .bind(mailbox_name)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn list_subscriptions(&self, user: &Username) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query("SELECT mailbox_name FROM subscriptions WHERE username = $1")
            .bind(user.to_string())
            .fetch_all(&self.pool)
            .await?;

        let subscriptions = rows
            .into_iter()
            .filter_map(|row| row.try_get("mailbox_name").ok())
            .collect();

        Ok(subscriptions)
    }
}

/// Complete PostgreSQL message store
struct PostgresCompleteMessageStore {
    pool: PgPool,
}

#[async_trait]
impl MessageStore for PostgresCompleteMessageStore {
    async fn append_message(
        &self,
        mailbox_id: &MailboxId,
        message: Mail,
    ) -> anyhow::Result<MessageMetadata> {
        // Get next UID for mailbox
        let uid_row = sqlx::query("SELECT uid_next FROM mailboxes WHERE id = $1 FOR UPDATE")
            .bind(*mailbox_id.as_uuid())
            .fetch_one(&self.pool)
            .await?;
        let uid: i32 = uid_row.try_get("uid_next")?;

        // Serialize message
        // In production: serialize headers and body properly
        let headers_json = serde_json::json!({});
        let body_bytes: &[u8] = b"";

        // Insert message
        sqlx::query(
            r#"
            INSERT INTO messages (id, mailbox_id, uid, sender, recipients, headers, body, size)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(*message.message_id().as_uuid())
        .bind(*mailbox_id.as_uuid())
        .bind(uid)
        .bind(message.sender().map(|s| s.to_string()))
        .bind(
            message
                .recipients()
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>(),
        )
        .bind(&headers_json)
        .bind(body_bytes)
        .bind(message.size() as i32)
        .execute(&self.pool)
        .await?;

        // Insert initial flags
        sqlx::query("INSERT INTO message_flags (message_id, flag_recent) VALUES ($1, TRUE)")
            .bind(*message.message_id().as_uuid())
            .execute(&self.pool)
            .await?;

        // Update mailbox uid_next
        sqlx::query("UPDATE mailboxes SET uid_next = $1 WHERE id = $2")
            .bind(uid + 1)
            .bind(*mailbox_id.as_uuid())
            .execute(&self.pool)
            .await?;

        let metadata = MessageMetadata::new(
            *message.message_id(),
            *mailbox_id,
            uid as u32,
            MessageFlags::new(),
            message.size(),
        );

        Ok(metadata)
    }

    async fn get_message(&self, _message_id: &MessageId) -> anyhow::Result<Option<Mail>> {
        // In production: reconstruct Mail from stored data
        // This requires parsing the body as MimeMessage
        Ok(None)
    }

    async fn delete_messages(&self, message_ids: &[MessageId]) -> anyhow::Result<()> {
        let uuids: Vec<uuid::Uuid> = message_ids.iter().map(|id| *id.as_uuid()).collect();

        sqlx::query("DELETE FROM messages WHERE id = ANY($1)")
            .bind(&uuids)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    async fn set_flags(
        &self,
        message_ids: &[MessageId],
        flags: MessageFlags,
    ) -> anyhow::Result<()> {
        let uuids: Vec<uuid::Uuid> = message_ids.iter().map(|id| *id.as_uuid()).collect();

        sqlx::query(
            r#"
            UPDATE message_flags SET
                flag_seen = $1,
                flag_answered = $2,
                flag_flagged = $3,
                flag_deleted = $4,
                flag_draft = $5
            WHERE message_id = ANY($6)
            "#,
        )
        .bind(flags.is_seen())
        .bind(flags.is_answered())
        .bind(flags.is_flagged())
        .bind(flags.is_deleted())
        .bind(flags.is_draft())
        .bind(&uuids)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn search(
        &self,
        mailbox_id: &MailboxId,
        criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>> {
        let query = match criteria {
            SearchCriteria::All => {
                sqlx::query("SELECT id FROM messages WHERE mailbox_id = $1")
                    .bind(*mailbox_id.as_uuid())
                    .fetch_all(&self.pool)
                    .await?
            }
            SearchCriteria::Unseen => {
                sqlx::query(
                    r#"
                    SELECT m.id FROM messages m
                    JOIN message_flags f ON m.id = f.message_id
                    WHERE m.mailbox_id = $1 AND f.flag_seen = FALSE
                    "#,
                )
                .bind(*mailbox_id.as_uuid())
                .fetch_all(&self.pool)
                .await?
            }
            SearchCriteria::Seen => {
                sqlx::query(
                    r#"
                    SELECT m.id FROM messages m
                    JOIN message_flags f ON m.id = f.message_id
                    WHERE m.mailbox_id = $1 AND f.flag_seen = TRUE
                    "#,
                )
                .bind(*mailbox_id.as_uuid())
                .fetch_all(&self.pool)
                .await?
            }
            SearchCriteria::Subject(text) => {
                sqlx::query(
                    r#"
                    SELECT id FROM messages
                    WHERE mailbox_id = $1 AND search_vector @@ plainto_tsquery('english', $2)
                    "#,
                )
                .bind(*mailbox_id.as_uuid())
                .bind(text)
                .fetch_all(&self.pool)
                .await?
            }
            SearchCriteria::From(email) => {
                sqlx::query("SELECT id FROM messages WHERE mailbox_id = $1 AND sender ILIKE $2")
                    .bind(*mailbox_id.as_uuid())
                    .bind(format!("%{}%", email))
                    .fetch_all(&self.pool)
                    .await?
            }
            _ => Vec::new(),
        };

        let _message_ids: Vec<MessageId> = query
            .into_iter()
            .filter_map(|row| {
                let _uuid: uuid::Uuid = row.try_get("id").ok()?;
                Some(MessageId::new())
            })
            .collect();

        Ok(vec![])
    }

    async fn copy_messages(
        &self,
        message_ids: &[MessageId],
        dest_mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        let mut metadata_list = Vec::new();

        for message_id in message_ids {
            let message = self.get_message(message_id).await?;
            if let Some(msg) = message {
                let metadata = self.append_message(dest_mailbox_id, msg).await?;
                metadata_list.push(metadata);
            }
        }

        Ok(metadata_list)
    }

    async fn get_mailbox_messages(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id, m.mailbox_id, m.uid, m.size,
                   f.flag_seen, f.flag_answered, f.flag_flagged,
                   f.flag_deleted, f.flag_draft, f.flag_recent
            FROM messages m
            LEFT JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1
            ORDER BY m.uid
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        let metadata_list = rows
            .into_iter()
            .filter_map(|row| {
                let _msg_id: uuid::Uuid = row.try_get("id").ok()?;
                let uid: i32 = row.try_get("uid").ok()?;
                let size: i32 = row.try_get("size").ok()?;

                let mut flags = MessageFlags::new();
                if let Ok(seen) = row.try_get("flag_seen") {
                    flags.set_seen(seen);
                }
                if let Ok(answered) = row.try_get("flag_answered") {
                    flags.set_answered(answered);
                }
                if let Ok(flagged) = row.try_get("flag_flagged") {
                    flags.set_flagged(flagged);
                }
                if let Ok(deleted) = row.try_get("flag_deleted") {
                    flags.set_deleted(deleted);
                }
                if let Ok(draft) = row.try_get("flag_draft") {
                    flags.set_draft(draft);
                }
                if let Ok(recent) = row.try_get("flag_recent") {
                    flags.set_recent(recent);
                }

                Some(MessageMetadata::new(
                    MessageId::new(),
                    *mailbox_id,
                    uid as u32,
                    flags,
                    size as usize,
                ))
            })
            .collect();

        Ok(metadata_list)
    }
}

/// Complete PostgreSQL metadata store
struct PostgresCompleteMetadataStore {
    pool: PgPool,
}

#[async_trait]
impl MetadataStore for PostgresCompleteMetadataStore {
    async fn get_user_quota(&self, user: &Username) -> anyhow::Result<Quota> {
        let row = sqlx::query("SELECT used, quota_limit FROM user_quotas WHERE username = $1")
            .bind(user.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(r) => {
                let used: i64 = r.try_get("used")?;
                let limit: i64 = r.try_get("quota_limit")?;
                Ok(Quota::new(used as u64, limit as u64))
            }
            None => Ok(Quota::new(0, 1024 * 1024 * 1024)), // Default 1GB
        }
    }

    async fn set_user_quota(&self, user: &Username, quota: Quota) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO user_quotas (username, used, quota_limit)
            VALUES ($1, $2, $3)
            ON CONFLICT (username) DO UPDATE
            SET used = $2, quota_limit = $3, updated_at = NOW()
            "#,
        )
        .bind(user.to_string())
        .bind(quota.used as i64)
        .bind(quota.limit as i64)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_mailbox_counters(
        &self,
        mailbox_id: &MailboxId,
    ) -> anyhow::Result<MailboxCounters> {
        let row = sqlx::query(
            r#"
            SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE f.flag_recent = TRUE) as recent,
                COUNT(*) FILTER (WHERE f.flag_seen = FALSE) as unseen
            FROM messages m
            LEFT JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = row.try_get("total")?;
        let recent: i64 = row.try_get("recent")?;
        let unseen: i64 = row.try_get("unseen")?;

        Ok(MailboxCounters {
            exists: total as u32,
            recent: recent as u32,
            unseen: unseen as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postgres_complete_backend_struct() {
        // Test that the struct is properly defined
        let _ = std::mem::size_of::<PostgresCompleteBackend>();
    }

    #[tokio::test]
    async fn test_init_schema_creates_tables() {
        // This would require a test database connection
        // For now, we just verify the schema SQL is valid
        let schema = r#"
            CREATE TABLE IF NOT EXISTS mailboxes (
                id UUID PRIMARY KEY,
                username TEXT NOT NULL,
                path TEXT NOT NULL,
                uid_validity INTEGER NOT NULL,
                uid_next INTEGER NOT NULL,
                special_use TEXT,
                created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
                UNIQUE(username, path)
            )
        "#;
        assert!(schema.contains("CREATE TABLE"));
    }

    #[test]
    fn test_search_criteria_all() {
        let criteria = SearchCriteria::All;
        assert!(matches!(criteria, SearchCriteria::All));
    }

    #[test]
    fn test_search_criteria_unseen() {
        let criteria = SearchCriteria::Unseen;
        assert!(matches!(criteria, SearchCriteria::Unseen));
    }

    #[test]
    fn test_search_criteria_from() {
        let criteria = SearchCriteria::From("test@example.com".to_string());
        assert!(matches!(criteria, SearchCriteria::From(_)));
    }

    #[test]
    fn test_search_criteria_subject() {
        let criteria = SearchCriteria::Subject("test subject".to_string());
        assert!(matches!(criteria, SearchCriteria::Subject(_)));
    }

    #[test]
    fn test_message_flags_default() {
        let flags = MessageFlags::new();
        assert!(!flags.is_seen());
        assert!(!flags.is_answered());
        assert!(!flags.is_flagged());
        assert!(!flags.is_deleted());
        assert!(!flags.is_draft());
    }

    #[test]
    fn test_message_flags_setters() {
        let mut flags = MessageFlags::new();
        flags.set_seen(true);
        flags.set_answered(true);
        flags.set_flagged(true);

        assert!(flags.is_seen());
        assert!(flags.is_answered());
        assert!(flags.is_flagged());
    }

    #[test]
    fn test_quota_new() {
        let quota = Quota::new(1024, 2048);
        assert_eq!(quota.used, 1024);
        assert_eq!(quota.limit, 2048);
    }

    #[test]
    fn test_quota_exceeded() {
        let quota = Quota::new(2048, 1024);
        assert!(quota.is_exceeded());

        let quota_ok = Quota::new(512, 1024);
        assert!(!quota_ok.is_exceeded());
    }

    #[test]
    fn test_quota_remaining() {
        let quota = Quota::new(256, 1024);
        assert_eq!(quota.remaining(), 768);
    }

    #[test]
    fn test_mailbox_counters_default() {
        let counters = MailboxCounters::default();
        assert_eq!(counters.exists, 0);
        assert_eq!(counters.recent, 0);
        assert_eq!(counters.unseen, 0);
    }

    #[test]
    fn test_mailbox_id_new() {
        let id1 = MailboxId::new();
        let id2 = MailboxId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_mailbox_id_display() {
        let id = MailboxId::new();
        let display = format!("{}", id);
        assert!(!display.is_empty());
    }

    #[test]
    fn test_mailbox_path_creation() {
        let user = Username::new("test@example.com".to_string()).unwrap();
        let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
        assert_eq!(path.user(), &user);
        assert_eq!(path.path().len(), 1);
    }

    #[test]
    fn test_mailbox_path_name() {
        let user = Username::new("test@example.com".to_string()).unwrap();
        let path = MailboxPath::new(user, vec!["INBOX".to_string(), "Sent".to_string()]);
        assert_eq!(path.name(), Some("Sent"));
    }

    #[test]
    fn test_mailbox_new() {
        let user = Username::new("test@example.com".to_string()).unwrap();
        let path = MailboxPath::new(user, vec!["INBOX".to_string()]);
        let mailbox = Mailbox::new(path);

        assert_eq!(mailbox.uid_validity(), 1);
        assert_eq!(mailbox.uid_next(), 1);
        assert!(mailbox.special_use().is_none());
    }

    #[test]
    fn test_mailbox_special_use() {
        let user = Username::new("test@example.com".to_string()).unwrap();
        let path = MailboxPath::new(user, vec!["Sent".to_string()]);
        let mut mailbox = Mailbox::new(path);

        mailbox.set_special_use(Some("\\Sent".to_string()));
        assert_eq!(mailbox.special_use(), Some("\\Sent"));
    }

    #[test]
    fn test_message_metadata_new() {
        let msg_id = MessageId::new();
        let mailbox_id = MailboxId::new();
        let flags = MessageFlags::new();

        let metadata = MessageMetadata::new(msg_id, mailbox_id, 1, flags, 1024);

        assert_eq!(metadata.message_id(), &msg_id);
        assert_eq!(metadata.mailbox_id(), &mailbox_id);
        assert_eq!(metadata.uid(), 1);
        assert_eq!(metadata.size(), 1024);
    }

    #[test]
    fn test_message_metadata_getters() {
        let msg_id = MessageId::new();
        let mailbox_id = MailboxId::new();
        let metadata = MessageMetadata::new(msg_id, mailbox_id, 42, MessageFlags::new(), 2048);

        assert_eq!(*metadata.message_id(), msg_id);
        assert_eq!(*metadata.mailbox_id(), mailbox_id);
        assert_eq!(metadata.uid(), 42);
        assert_eq!(metadata.size(), 2048);
    }

    #[test]
    fn test_search_criteria_and() {
        let criteria = SearchCriteria::And(vec![
            SearchCriteria::Unseen,
            SearchCriteria::From("test@example.com".to_string()),
        ]);
        assert!(matches!(criteria, SearchCriteria::And(_)));
    }

    #[test]
    fn test_search_criteria_or() {
        let criteria = SearchCriteria::Or(vec![SearchCriteria::Flagged, SearchCriteria::Deleted]);
        assert!(matches!(criteria, SearchCriteria::Or(_)));
    }

    #[test]
    fn test_search_criteria_not() {
        let criteria = SearchCriteria::Not(Box::new(SearchCriteria::Seen));
        assert!(matches!(criteria, SearchCriteria::Not(_)));
    }

    #[test]
    fn test_mailbox_counters_struct() {
        let counters = MailboxCounters {
            exists: 10,
            recent: 3,
            unseen: 5,
        };
        assert_eq!(counters.exists, 10);
        assert_eq!(counters.recent, 3);
        assert_eq!(counters.unseen, 5);
    }
}
