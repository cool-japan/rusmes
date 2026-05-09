//! Schema initialization and migration logic for the PostgreSQL backend.

use sqlx::postgres::PgPool;
use sqlx::Executor;

/// Initialize database schema with migrations.
pub(super) async fn init_schema(pool: &PgPool) -> anyhow::Result<()> {
    tracing::info!("Initializing PostgreSQL schema");

    // Create migrations table
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TIMESTAMP NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    // Apply migrations in order
    apply_migration_1_mailboxes(pool).await?;
    apply_migration_2_messages(pool).await?;
    apply_migration_3_message_blobs(pool).await?;
    apply_migration_4_indexes(pool).await?;
    apply_migration_5_fulltext(pool).await?;

    tracing::info!("PostgreSQL schema initialization complete");
    Ok(())
}

async fn is_migration_applied(pool: &PgPool, version: i32) -> anyhow::Result<bool> {
    let result = sqlx::query("SELECT version FROM schema_migrations WHERE version = $1")
        .bind(version)
        .fetch_optional(pool)
        .await?;
    Ok(result.is_some())
}

async fn mark_migration_applied(pool: &PgPool, version: i32) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO schema_migrations (version) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(version)
        .execute(pool)
        .await?;
    Ok(())
}

async fn apply_migration_1_mailboxes(pool: &PgPool) -> anyhow::Result<()> {
    let version = 1;
    if is_migration_applied(pool, version).await? {
        return Ok(());
    }

    tracing::info!("Applying migration {}: mailboxes and quotas", version);

    // Mailboxes table
    pool.execute(
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
    pool.execute(
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
    pool.execute(
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

    mark_migration_applied(pool, version).await?;
    Ok(())
}

async fn apply_migration_2_messages(pool: &PgPool) -> anyhow::Result<()> {
    let version = 2;
    if is_migration_applied(pool, version).await? {
        return Ok(());
    }

    tracing::info!("Applying migration {}: messages and flags", version);

    // Messages table with inline body storage
    pool.execute(
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
    pool.execute(
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

    mark_migration_applied(pool, version).await?;
    Ok(())
}

async fn apply_migration_3_message_blobs(pool: &PgPool) -> anyhow::Result<()> {
    let version = 3;
    if is_migration_applied(pool, version).await? {
        return Ok(());
    }

    tracing::info!("Applying migration {}: message blobs", version);

    // Message blobs table for large messages
    pool.execute(
        r#"
        CREATE TABLE IF NOT EXISTS message_blobs (
            id UUID PRIMARY KEY,
            data BYTEA NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .await?;

    mark_migration_applied(pool, version).await?;
    Ok(())
}

async fn apply_migration_4_indexes(pool: &PgPool) -> anyhow::Result<()> {
    let version = 4;
    if is_migration_applied(pool, version).await? {
        return Ok(());
    }

    tracing::info!("Applying migration {}: indexes", version);

    // Mailboxes indexes
    pool.execute("CREATE INDEX IF NOT EXISTS idx_mailboxes_username ON mailboxes(username)")
        .await?;
    pool.execute("CREATE INDEX IF NOT EXISTS idx_mailboxes_path ON mailboxes(path)")
        .await?;

    // Messages indexes for IMAP operations
    pool.execute("CREATE INDEX IF NOT EXISTS idx_messages_mailbox ON messages(mailbox_id)")
        .await?;
    pool.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_mailbox_uid ON messages(mailbox_id, uid)",
    )
    .await?;
    pool.execute("CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender)")
        .await?;
    pool.execute("CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at)")
        .await?;

    // Message flags indexes for search
    pool.execute("CREATE INDEX IF NOT EXISTS idx_flags_message ON message_flags(message_id)")
        .await?;
    pool.execute("CREATE INDEX IF NOT EXISTS idx_flags_seen ON message_flags(message_id) WHERE flag_seen = FALSE")
        .await?;
    pool.execute("CREATE INDEX IF NOT EXISTS idx_flags_recent ON message_flags(message_id) WHERE flag_recent = TRUE")
        .await?;
    pool.execute("CREATE INDEX IF NOT EXISTS idx_flags_deleted ON message_flags(message_id) WHERE flag_deleted = TRUE")
        .await?;

    mark_migration_applied(pool, version).await?;
    Ok(())
}

async fn apply_migration_5_fulltext(pool: &PgPool) -> anyhow::Result<()> {
    let version = 5;
    if is_migration_applied(pool, version).await? {
        return Ok(());
    }

    tracing::info!("Applying migration {}: full-text search", version);

    // Full-text search index
    pool.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_search ON messages USING GIN(search_vector)",
    )
    .await?;

    // Create trigger function for updating search_vector
    pool.execute(
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
    )
    .await?;

    // Create trigger
    pool.execute(
        r#"
        DROP TRIGGER IF EXISTS messages_search_vector_trigger ON messages;
        CREATE TRIGGER messages_search_vector_trigger
        BEFORE INSERT OR UPDATE ON messages
        FOR EACH ROW EXECUTE FUNCTION messages_search_vector_update()
        "#,
    )
    .await?;

    mark_migration_applied(pool, version).await?;
    Ok(())
}
