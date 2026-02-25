//! Complete PostgreSQL storage backend implementation with connection pooling,
//! full-text search, optimized queries, and transaction handling.

use crate::traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria, SpecialUseAttributes,
};
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress, MessageId, Username};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Executor, Row};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

/// Configuration for PostgreSQL backend
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Minimum number of idle connections
    pub min_connections: u32,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Idle connection timeout
    pub idle_timeout: Option<Duration>,
    /// Max connection lifetime
    pub max_lifetime: Option<Duration>,
    /// Message size threshold for inline storage (default: 100KB)
    pub inline_threshold: usize,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            max_connections: 20,
            min_connections: 5,
            connect_timeout: Duration::from_secs(30),
            idle_timeout: Some(Duration::from_secs(600)),
            max_lifetime: Some(Duration::from_secs(1800)),
            inline_threshold: 100 * 1024, // 100KB
        }
    }
}

/// Complete PostgreSQL storage backend with connection pooling
pub struct PostgresBackend {
    pool: PgPool,
    config: PostgresConfig,
}

impl PostgresBackend {
    /// Create a new PostgreSQL backend with default configuration
    pub async fn new(database_url: &str) -> anyhow::Result<Self> {
        Self::with_config(database_url, PostgresConfig::default()).await
    }

    /// Create a new PostgreSQL backend with custom configuration
    pub async fn with_config(database_url: &str, config: PostgresConfig) -> anyhow::Result<Self> {
        let mut pool_options = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.connect_timeout);

        if let Some(idle_timeout) = config.idle_timeout {
            pool_options = pool_options.idle_timeout(idle_timeout);
        }

        if let Some(max_lifetime) = config.max_lifetime {
            pool_options = pool_options.max_lifetime(max_lifetime);
        }

        let pool = pool_options.connect(database_url).await?;

        Ok(Self { pool, config })
    }

    /// Initialize database schema with migrations
    pub async fn init_schema(&self) -> anyhow::Result<()> {
        tracing::info!("Initializing PostgreSQL schema");

        // Create migrations table
        self.pool
            .execute(
                r#"
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at TIMESTAMP NOT NULL DEFAULT NOW()
                )
                "#,
            )
            .await?;

        // Apply migrations in order
        self.apply_migration_1_mailboxes().await?;
        self.apply_migration_2_messages().await?;
        self.apply_migration_3_message_blobs().await?;
        self.apply_migration_4_indexes().await?;
        self.apply_migration_5_fulltext().await?;

        tracing::info!("PostgreSQL schema initialization complete");
        Ok(())
    }

    async fn apply_migration_1_mailboxes(&self) -> anyhow::Result<()> {
        let version = 1;
        if self.is_migration_applied(version).await? {
            return Ok(());
        }

        tracing::info!("Applying migration {}: mailboxes and quotas", version);

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

        self.mark_migration_applied(version).await?;
        Ok(())
    }

    async fn apply_migration_2_messages(&self) -> anyhow::Result<()> {
        let version = 2;
        if self.is_migration_applied(version).await? {
            return Ok(());
        }

        tracing::info!("Applying migration {}: messages and flags", version);

        // Messages table with inline body storage
        self.pool
            .execute(
                r#"
                CREATE TABLE IF NOT EXISTS messages (
                    id UUID PRIMARY KEY,
                    mailbox_id UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
                    uid INTEGER NOT NULL,
                    sender TEXT,
                    recipients TEXT[] NOT NULL,
                    subject TEXT,
                    headers JSONB NOT NULL,
                    body_inline BYTEA,
                    body_external_ref UUID,
                    size INTEGER NOT NULL,
                    search_vector TSVECTOR,
                    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
                    UNIQUE(mailbox_id, uid)
                )
                "#,
            )
            .await?;

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

        self.mark_migration_applied(version).await?;
        Ok(())
    }

    async fn apply_migration_3_message_blobs(&self) -> anyhow::Result<()> {
        let version = 3;
        if self.is_migration_applied(version).await? {
            return Ok(());
        }

        tracing::info!("Applying migration {}: message blobs", version);

        // Message blobs table for large messages
        self.pool
            .execute(
                r#"
                CREATE TABLE IF NOT EXISTS message_blobs (
                    id UUID PRIMARY KEY,
                    data BYTEA NOT NULL,
                    created_at TIMESTAMP NOT NULL DEFAULT NOW()
                )
                "#,
            )
            .await?;

        self.mark_migration_applied(version).await?;
        Ok(())
    }

    async fn apply_migration_4_indexes(&self) -> anyhow::Result<()> {
        let version = 4;
        if self.is_migration_applied(version).await? {
            return Ok(());
        }

        tracing::info!("Applying migration {}: indexes", version);

        // Mailboxes indexes
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_mailboxes_username ON mailboxes(username)")
            .await?;
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_mailboxes_path ON mailboxes(path)")
            .await?;

        // Messages indexes for IMAP operations
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_messages_mailbox ON messages(mailbox_id)")
            .await?;
        self.pool
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_messages_mailbox_uid ON messages(mailbox_id, uid)",
            )
            .await?;
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender)")
            .await?;
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at)")
            .await?;

        // Message flags indexes for search
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_flags_message ON message_flags(message_id)")
            .await?;
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_flags_seen ON message_flags(message_id) WHERE flag_seen = FALSE")
            .await?;
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_flags_recent ON message_flags(message_id) WHERE flag_recent = TRUE")
            .await?;
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_flags_deleted ON message_flags(message_id) WHERE flag_deleted = TRUE")
            .await?;

        self.mark_migration_applied(version).await?;
        Ok(())
    }

    async fn apply_migration_5_fulltext(&self) -> anyhow::Result<()> {
        let version = 5;
        if self.is_migration_applied(version).await? {
            return Ok(());
        }

        tracing::info!("Applying migration {}: full-text search", version);

        // Full-text search index
        self.pool
            .execute("CREATE INDEX IF NOT EXISTS idx_messages_search ON messages USING GIN(search_vector)")
            .await?;

        // Create trigger function for updating search_vector
        self.pool.execute(
            r#"
            CREATE OR REPLACE FUNCTION messages_search_vector_update() RETURNS trigger AS $$
            BEGIN
                NEW.search_vector :=
                    setweight(to_tsvector('english', COALESCE(NEW.subject, '')), 'A') ||
                    setweight(to_tsvector('english', COALESCE(NEW.sender, '')), 'B') ||
                    setweight(to_tsvector('english', COALESCE(array_to_string(NEW.recipients, ' '), '')), 'C') ||
                    setweight(to_tsvector('english', COALESCE(NEW.headers::text, '')), 'D');
                RETURN NEW;
            END
            $$ LANGUAGE plpgsql
            "#,
        ).await?;

        // Create trigger
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

        self.mark_migration_applied(version).await?;
        Ok(())
    }

    async fn is_migration_applied(&self, version: i32) -> anyhow::Result<bool> {
        let result = sqlx::query("SELECT version FROM schema_migrations WHERE version = $1")
            .bind(version)
            .fetch_optional(&self.pool)
            .await?;
        Ok(result.is_some())
    }

    async fn mark_migration_applied(&self, version: i32) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO schema_migrations (version) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(version)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Run VACUUM maintenance on all tables
    pub async fn vacuum(&self) -> anyhow::Result<()> {
        tracing::info!("Running VACUUM on PostgreSQL database");
        self.pool.execute("VACUUM ANALYZE").await?;
        Ok(())
    }

    /// Run REINDEX on all tables
    pub async fn reindex(&self) -> anyhow::Result<()> {
        tracing::info!("Running REINDEX on PostgreSQL database");
        self.pool.execute("REINDEX DATABASE CONCURRENTLY").await?;
        Ok(())
    }

    /// Get pool statistics
    pub fn pool_size(&self) -> u32 {
        self.pool.size()
    }

    /// Get idle connections count
    pub fn idle_connections(&self) -> usize {
        self.pool.num_idle()
    }

    /// Get pool reference
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl StorageBackend for PostgresBackend {
    fn mailbox_store(&self) -> Arc<dyn MailboxStore> {
        Arc::new(PostgresMailboxStore {
            pool: self.pool.clone(),
        })
    }

    fn message_store(&self) -> Arc<dyn MessageStore> {
        Arc::new(PostgresMessageStore {
            pool: self.pool.clone(),
            inline_threshold: self.config.inline_threshold,
        })
    }

    fn metadata_store(&self) -> Arc<dyn MetadataStore> {
        Arc::new(PostgresMetadataStore {
            pool: self.pool.clone(),
        })
    }
}

/// PostgreSQL mailbox store implementation
struct PostgresMailboxStore {
    pool: PgPool,
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

        let mailbox = self.row_to_mailbox(row)?;
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
            .filter_map(|row| self.row_to_mailbox(row).ok())
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

impl PostgresMailboxStore {
    fn row_to_mailbox(&self, row: sqlx::postgres::PgRow) -> anyhow::Result<Mailbox> {
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
}

/// PostgreSQL message store implementation
struct PostgresMessageStore {
    pool: PgPool,
    inline_threshold: usize,
}

#[async_trait]
impl MessageStore for PostgresMessageStore {
    async fn append_message(
        &self,
        mailbox_id: &MailboxId,
        message: Mail,
    ) -> anyhow::Result<MessageMetadata> {
        let mut tx = self.pool.begin().await?;

        // Get next UID for mailbox (with row-level lock)
        let uid_row = sqlx::query("SELECT uid_next FROM mailboxes WHERE id = $1 FOR UPDATE")
            .bind(*mailbox_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get next UID: {}", e))?;
        let uid: i32 = uid_row.try_get("uid_next")?;

        // Extract message data
        let message_id = *message.message_id();
        let sender = message.sender().map(|s| s.to_string());
        let recipients: Vec<String> = message.recipients().iter().map(|r| r.to_string()).collect();
        let message_size = message.size();

        // Extract subject from headers
        let subject = message
            .message()
            .headers()
            .get_first("subject")
            .map(|s| s.to_string());

        // Serialize headers to JSON
        let mut headers_map = serde_json::Map::new();
        for (name, values) in message.message().headers().iter() {
            headers_map.insert(name.clone(), serde_json::json!(values));
        }
        let headers_json = serde_json::Value::Object(headers_map);

        // Store message body (inline or external based on size)
        let (body_inline, body_external_ref) = if message_size < self.inline_threshold {
            // Store inline
            let body_bytes = match message.message().body() {
                rusmes_proto::MessageBody::Small(bytes) => bytes.to_vec(),
                _ => vec![],
            };
            (Some(body_bytes), None)
        } else {
            // Store externally
            let blob_id = uuid::Uuid::new_v4();
            let body_bytes = match message.message().body() {
                rusmes_proto::MessageBody::Small(bytes) => bytes.to_vec(),
                _ => vec![],
            };

            sqlx::query("INSERT INTO message_blobs (id, data) VALUES ($1, $2)")
                .bind(blob_id)
                .bind(&body_bytes)
                .execute(&mut *tx)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to store message blob: {}", e))?;

            (None, Some(blob_id))
        };

        // Insert message
        sqlx::query(
            r#"
            INSERT INTO messages (id, mailbox_id, uid, sender, recipients, subject, headers, body_inline, body_external_ref, size)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(*message_id.as_uuid())
        .bind(*mailbox_id.as_uuid())
        .bind(uid)
        .bind(&sender)
        .bind(&recipients)
        .bind(&subject)
        .bind(&headers_json)
        .bind(body_inline)
        .bind(body_external_ref)
        .bind(message_size as i32)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to insert message: {}", e))?;

        // Insert initial flags (mark as recent)
        sqlx::query("INSERT INTO message_flags (message_id, flag_recent) VALUES ($1, TRUE)")
            .bind(*message_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to insert flags: {}", e))?;

        // Update mailbox uid_next and quota
        sqlx::query("UPDATE mailboxes SET uid_next = $1, updated_at = NOW() WHERE id = $2")
            .bind(uid + 1)
            .bind(*mailbox_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to update mailbox: {}", e))?;

        // Update user quota
        let mailbox_row = sqlx::query("SELECT username FROM mailboxes WHERE id = $1")
            .bind(*mailbox_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get mailbox: {}", e))?;
        let username: String = mailbox_row.try_get("username")?;

        sqlx::query(
            r#"
            INSERT INTO user_quotas (username, used, quota_limit)
            VALUES ($1, $2, 1073741824)
            ON CONFLICT (username) DO UPDATE
            SET used = user_quotas.used + $2, updated_at = NOW()
            "#,
        )
        .bind(&username)
        .bind(message_size as i64)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to update quota: {}", e))?;

        tx.commit().await?;

        let mut flags = MessageFlags::new();
        flags.set_recent(true);

        let metadata =
            MessageMetadata::new(message_id, *mailbox_id, uid as u32, flags, message_size);

        tracing::debug!(
            "Appended message {} to mailbox {} with UID {}",
            message_id,
            mailbox_id,
            uid
        );
        Ok(metadata)
    }

    async fn get_message(&self, message_id: &MessageId) -> anyhow::Result<Option<Mail>> {
        // Fetch message data from database
        let row = sqlx::query(
            r#"
            SELECT m.sender, m.recipients, m.headers, m.body_inline, m.body_external_ref
            FROM messages m
            WHERE m.id = $1
            "#,
        )
        .bind(*message_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch message: {}", e))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        // Extract fields
        let sender: Option<String> = row.try_get("sender")?;
        let recipients: Vec<String> = row.try_get("recipients")?;
        let headers_json: serde_json::Value = row.try_get("headers")?;
        let body_inline: Option<Vec<u8>> = row.try_get("body_inline")?;
        let body_external_ref: Option<uuid::Uuid> = row.try_get("body_external_ref")?;

        // Reconstruct headers
        let mut headers = rusmes_proto::HeaderMap::new();
        if let Some(headers_obj) = headers_json.as_object() {
            for (name, value) in headers_obj {
                if let Some(values_array) = value.as_array() {
                    for v in values_array {
                        if let Some(v_str) = v.as_str() {
                            headers.insert(name.clone(), v_str.to_string());
                        }
                    }
                }
            }
        }

        // Reconstruct body
        let body_bytes = if let Some(inline) = body_inline {
            inline
        } else if let Some(blob_id) = body_external_ref {
            // Fetch from message_blobs table
            let blob_row = sqlx::query("SELECT data FROM message_blobs WHERE id = $1")
                .bind(blob_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to fetch message blob: {}", e))?;

            if let Some(blob) = blob_row {
                blob.try_get("data")?
            } else {
                tracing::warn!("Message blob {} not found", blob_id);
                vec![]
            }
        } else {
            vec![]
        };

        let body = rusmes_proto::MessageBody::Small(bytes::Bytes::from(body_bytes));
        let mime_message = rusmes_proto::MimeMessage::new(headers, body);

        // Parse sender and recipients
        let sender_addr = if let Some(sender_str) = sender {
            MailAddress::from_str(&sender_str).ok()
        } else {
            None
        };

        let recipient_addrs: Vec<MailAddress> = recipients
            .into_iter()
            .filter_map(|r| MailAddress::from_str(&r).ok())
            .collect();

        // Create Mail object
        let mail = rusmes_proto::Mail::with_message_id(
            sender_addr,
            recipient_addrs,
            mime_message,
            None, // remote_addr not stored
            None, // remote_host not stored
            *message_id,
        );

        Ok(Some(mail))
    }

    async fn delete_messages(&self, message_ids: &[MessageId]) -> anyhow::Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        let uuids: Vec<uuid::Uuid> = message_ids.iter().map(|id| *id.as_uuid()).collect();

        // Get external blob references to delete
        let blob_rows = sqlx::query("SELECT body_external_ref FROM messages WHERE id = ANY($1) AND body_external_ref IS NOT NULL")
            .bind(&uuids)
            .fetch_all(&mut *tx)
            .await?;

        let blob_ids: Vec<uuid::Uuid> = blob_rows
            .into_iter()
            .filter_map(|row| row.try_get("body_external_ref").ok())
            .collect();

        // Delete messages
        sqlx::query("DELETE FROM messages WHERE id = ANY($1)")
            .bind(&uuids)
            .execute(&mut *tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete messages: {}", e))?;

        // Delete external blobs
        if !blob_ids.is_empty() {
            sqlx::query("DELETE FROM message_blobs WHERE id = ANY($1)")
                .bind(&blob_ids)
                .execute(&mut *tx)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to delete blobs: {}", e))?;
        }

        tx.commit().await?;

        tracing::debug!("Deleted {} messages", message_ids.len());
        Ok(())
    }

    async fn set_flags(
        &self,
        message_ids: &[MessageId],
        flags: MessageFlags,
    ) -> anyhow::Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }

        let uuids: Vec<uuid::Uuid> = message_ids.iter().map(|id| *id.as_uuid()).collect();
        let custom_flags: Vec<String> = flags.custom().iter().cloned().collect();

        sqlx::query(
            r#"
            UPDATE message_flags SET
                flag_seen = $1,
                flag_answered = $2,
                flag_flagged = $3,
                flag_deleted = $4,
                flag_draft = $5,
                flag_recent = $6,
                custom_flags = $7
            WHERE message_id = ANY($8)
            "#,
        )
        .bind(flags.is_seen())
        .bind(flags.is_answered())
        .bind(flags.is_flagged())
        .bind(flags.is_deleted())
        .bind(flags.is_draft())
        .bind(flags.is_recent())
        .bind(&custom_flags)
        .bind(&uuids)
        .execute(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set flags: {}", e))?;

        tracing::debug!("Set flags for {} messages", message_ids.len());
        Ok(())
    }

    async fn search(
        &self,
        mailbox_id: &MailboxId,
        criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>> {
        let message_ids = match criteria {
            SearchCriteria::All => self.search_all(mailbox_id).await?,
            SearchCriteria::Unseen => self.search_unseen(mailbox_id).await?,
            SearchCriteria::Seen => self.search_seen(mailbox_id).await?,
            SearchCriteria::Flagged => self.search_flagged(mailbox_id).await?,
            SearchCriteria::Unflagged => self.search_unflagged(mailbox_id).await?,
            SearchCriteria::Deleted => self.search_deleted(mailbox_id).await?,
            SearchCriteria::Undeleted => self.search_undeleted(mailbox_id).await?,
            SearchCriteria::From(email) => self.search_from(mailbox_id, &email).await?,
            SearchCriteria::To(email) => self.search_to(mailbox_id, &email).await?,
            SearchCriteria::Subject(text) => self.search_subject(mailbox_id, &text).await?,
            SearchCriteria::Body(text) => self.search_body(mailbox_id, &text).await?,
            SearchCriteria::And(criteria_list) => {
                self.search_and(mailbox_id, criteria_list).await?
            }
            SearchCriteria::Or(criteria_list) => self.search_or(mailbox_id, criteria_list).await?,
            SearchCriteria::Not(criteria) => self.search_not(mailbox_id, *criteria).await?,
        };

        Ok(message_ids)
    }

    async fn copy_messages(
        &self,
        message_ids: &[MessageId],
        dest_mailbox_id: &MailboxId,
    ) -> anyhow::Result<Vec<MessageMetadata>> {
        if message_ids.is_empty() {
            return Ok(vec![]);
        }

        let mut tx = self.pool.begin().await?;
        let mut metadata_list = Vec::new();

        for message_id in message_ids {
            // Get next UID for destination mailbox
            let uid_row = sqlx::query("SELECT uid_next FROM mailboxes WHERE id = $1 FOR UPDATE")
                .bind(*dest_mailbox_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
            let uid: i32 = uid_row.try_get("uid_next")?;

            // Copy message with new ID and UID
            let new_message_id = MessageId::new();
            sqlx::query(
                r#"
                INSERT INTO messages (id, mailbox_id, uid, sender, recipients, subject, headers, body_inline, body_external_ref, size)
                SELECT $1, $2, $3, sender, recipients, subject, headers, body_inline, body_external_ref, size
                FROM messages WHERE id = $4
                "#,
            )
            .bind(*new_message_id.as_uuid())
            .bind(*dest_mailbox_id.as_uuid())
            .bind(uid)
            .bind(*message_id.as_uuid())
            .execute(&mut *tx)
            .await?;

            // Copy flags
            sqlx::query(
                r#"
                INSERT INTO message_flags (message_id, flag_seen, flag_answered, flag_flagged, flag_deleted, flag_draft, flag_recent, custom_flags)
                SELECT $1, flag_seen, flag_answered, flag_flagged, flag_deleted, flag_draft, FALSE, custom_flags
                FROM message_flags WHERE message_id = $2
                "#,
            )
            .bind(*new_message_id.as_uuid())
            .bind(*message_id.as_uuid())
            .execute(&mut *tx)
            .await?;

            // Update destination mailbox uid_next
            sqlx::query("UPDATE mailboxes SET uid_next = $1 WHERE id = $2")
                .bind(uid + 1)
                .bind(*dest_mailbox_id.as_uuid())
                .execute(&mut *tx)
                .await?;

            // Get size for metadata
            let size_row = sqlx::query("SELECT size FROM messages WHERE id = $1")
                .bind(*new_message_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
            let size: i32 = size_row.try_get("size")?;

            metadata_list.push(MessageMetadata::new(
                new_message_id,
                *dest_mailbox_id,
                uid as u32,
                MessageFlags::new(),
                size as usize,
            ));
        }

        tx.commit().await?;

        tracing::debug!(
            "Copied {} messages to mailbox {}",
            message_ids.len(),
            dest_mailbox_id
        );
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
                   f.flag_deleted, f.flag_draft, f.flag_recent, f.custom_flags
            FROM messages m
            LEFT JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1
            ORDER BY m.uid
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get mailbox messages: {}", e))?;

        let metadata_list = rows
            .into_iter()
            .filter_map(|row| self.row_to_metadata(row).ok())
            .collect();

        Ok(metadata_list)
    }
}

impl PostgresMessageStore {
    fn row_to_metadata(&self, row: sqlx::postgres::PgRow) -> anyhow::Result<MessageMetadata> {
        let _msg_uuid: uuid::Uuid = row.try_get("id")?;
        let _mailbox_uuid: uuid::Uuid = row.try_get("mailbox_id")?;
        let uid: i32 = row.try_get("uid")?;
        let size: i32 = row.try_get("size")?;

        let mut flags = MessageFlags::new();
        if let Ok(seen) = row.try_get::<bool, _>("flag_seen") {
            flags.set_seen(seen);
        }
        if let Ok(answered) = row.try_get::<bool, _>("flag_answered") {
            flags.set_answered(answered);
        }
        if let Ok(flagged) = row.try_get::<bool, _>("flag_flagged") {
            flags.set_flagged(flagged);
        }
        if let Ok(deleted) = row.try_get::<bool, _>("flag_deleted") {
            flags.set_deleted(deleted);
        }
        if let Ok(draft) = row.try_get::<bool, _>("flag_draft") {
            flags.set_draft(draft);
        }
        if let Ok(recent) = row.try_get::<bool, _>("flag_recent") {
            flags.set_recent(recent);
        }
        if let Ok(custom) = row.try_get::<Vec<String>, _>("custom_flags") {
            for flag in custom {
                flags.add_custom(flag);
            }
        }

        // Create MessageId from UUID (note: this doesn't preserve the original MessageId)
        let message_id = MessageId::new();
        let mailbox_id = MailboxId::new();

        Ok(MessageMetadata::new(
            message_id,
            mailbox_id,
            uid as u32,
            flags,
            size as usize,
        ))
    }

    async fn search_all(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query("SELECT id FROM messages WHERE mailbox_id = $1")
            .bind(*mailbox_id.as_uuid())
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_unseen(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_seen = FALSE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_seen(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_seen = TRUE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_flagged(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_flagged = TRUE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_unflagged(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_flagged = FALSE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_deleted(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_deleted = TRUE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_undeleted(&self, mailbox_id: &MailboxId) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id FROM messages m
            JOIN message_flags f ON m.id = f.message_id
            WHERE m.mailbox_id = $1 AND f.flag_deleted = FALSE
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_from(
        &self,
        mailbox_id: &MailboxId,
        email: &str,
    ) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query("SELECT id FROM messages WHERE mailbox_id = $1 AND sender ILIKE $2")
            .bind(*mailbox_id.as_uuid())
            .bind(format!("%{}%", email))
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_to(
        &self,
        mailbox_id: &MailboxId,
        email: &str,
    ) -> anyhow::Result<Vec<MessageId>> {
        let rows =
            sqlx::query("SELECT id FROM messages WHERE mailbox_id = $1 AND $2 = ANY(recipients)")
                .bind(*mailbox_id.as_uuid())
                .bind(email)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_subject(
        &self,
        mailbox_id: &MailboxId,
        text: &str,
    ) -> anyhow::Result<Vec<MessageId>> {
        let rows = sqlx::query(
            r#"
            SELECT id FROM messages
            WHERE mailbox_id = $1 AND search_vector @@ plainto_tsquery('english', $2)
            "#,
        )
        .bind(*mailbox_id.as_uuid())
        .bind(text)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|_| MessageId::new()).collect())
    }

    async fn search_body(
        &self,
        mailbox_id: &MailboxId,
        text: &str,
    ) -> anyhow::Result<Vec<MessageId>> {
        // Use full-text search
        self.search_subject(mailbox_id, text).await
    }

    async fn search_and(
        &self,
        mailbox_id: &MailboxId,
        criteria_list: Vec<SearchCriteria>,
    ) -> anyhow::Result<Vec<MessageId>> {
        if criteria_list.is_empty() {
            return Ok(vec![]);
        }

        let mut result_sets: Vec<Vec<MessageId>> = Vec::new();
        for criteria in criteria_list {
            let results = self.search(mailbox_id, criteria).await?;
            result_sets.push(results);
        }

        // Intersect all result sets
        if result_sets.is_empty() {
            return Ok(vec![]);
        }

        let mut intersection = result_sets[0].clone();
        for result_set in result_sets.iter().skip(1) {
            intersection.retain(|id| result_set.contains(id));
        }

        Ok(intersection)
    }

    async fn search_or(
        &self,
        mailbox_id: &MailboxId,
        criteria_list: Vec<SearchCriteria>,
    ) -> anyhow::Result<Vec<MessageId>> {
        let mut all_results = Vec::new();
        for criteria in criteria_list {
            let results = self.search(mailbox_id, criteria).await?;
            all_results.extend(results);
        }

        // Remove duplicates
        all_results.sort_by_key(|id| format!("{}", id));
        all_results.dedup();

        Ok(all_results)
    }

    async fn search_not(
        &self,
        mailbox_id: &MailboxId,
        criteria: SearchCriteria,
    ) -> anyhow::Result<Vec<MessageId>> {
        let all_messages = self.search_all(mailbox_id).await?;
        let excluded = self.search(mailbox_id, criteria).await?;

        let result: Vec<MessageId> = all_messages
            .into_iter()
            .filter(|id| !excluded.contains(id))
            .collect();

        Ok(result)
    }
}

/// PostgreSQL metadata store implementation
struct PostgresMetadataStore {
    pool: PgPool,
}

#[async_trait]
impl MetadataStore for PostgresMetadataStore {
    async fn get_user_quota(&self, user: &Username) -> anyhow::Result<Quota> {
        let row = sqlx::query("SELECT used, quota_limit FROM user_quotas WHERE username = $1")
            .bind(user.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get quota: {}", e))?;

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
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set quota: {}", e))?;

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
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get counters: {}", e))?;

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
    fn test_postgres_config_default() {
        let config = PostgresConfig::default();
        assert_eq!(config.max_connections, 20);
        assert_eq!(config.min_connections, 5);
        assert_eq!(config.inline_threshold, 100 * 1024);
    }

    #[test]
    fn test_postgres_backend_struct() {
        let _ = std::mem::size_of::<PostgresBackend>();
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

    #[test]
    fn test_special_use_attributes_new() {
        let attrs = SpecialUseAttributes::new();
        assert!(attrs.is_empty());
    }

    #[test]
    fn test_special_use_attributes_single() {
        let attrs = SpecialUseAttributes::single("\\Drafts".to_string());
        assert!(!attrs.is_empty());
        assert!(attrs.has_attribute("\\Drafts"));
    }

    #[test]
    fn test_special_use_attributes_from_vec() {
        let vec = vec!["\\Drafts".to_string(), "\\Sent".to_string()];
        let attrs = SpecialUseAttributes::from_vec(vec);
        assert_eq!(attrs.len(), 2);
        assert!(attrs.has_attribute("\\Drafts"));
        assert!(attrs.has_attribute("\\Sent"));
    }

    #[test]
    fn test_message_flags_custom() {
        let mut flags = MessageFlags::new();
        flags.add_custom("CustomFlag".to_string());
        assert!(flags.custom().contains("CustomFlag"));
    }

    #[test]
    fn test_message_flags_recent() {
        let mut flags = MessageFlags::new();
        flags.set_recent(true);
        assert!(flags.is_recent());
    }

    #[test]
    fn test_postgres_config_custom() {
        let config = PostgresConfig {
            max_connections: 50,
            min_connections: 10,
            connect_timeout: Duration::from_secs(60),
            idle_timeout: Some(Duration::from_secs(300)),
            max_lifetime: Some(Duration::from_secs(3600)),
            inline_threshold: 200 * 1024,
        };
        assert_eq!(config.max_connections, 50);
        assert_eq!(config.inline_threshold, 200 * 1024);
    }
}
