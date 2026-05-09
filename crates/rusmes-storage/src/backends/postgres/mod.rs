//! Complete PostgreSQL storage backend implementation with connection pooling,
//! full-text search, optimized queries, transaction handling, and background
//! VACUUM scheduling.

mod mailboxes;
mod messages;
mod metadata;
mod schema;

#[cfg(test)]
mod tests;

use crate::traits::{MailboxStore, MessageStore, MetadataStore, StorageBackend};
use mailboxes::PostgresMailboxStore;
use messages::PostgresMessageStore;
use metadata::PostgresMetadataStore;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

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
    /// Watch sender used to signal the background VACUUM task to exit.
    shutdown_tx: watch::Sender<bool>,
}

impl PostgresBackend {
    /// Create a new PostgreSQL backend with default configuration.
    ///
    /// A background VACUUM scheduler is started with a 24-hour default interval.
    /// Use [`PostgresBackend::with_config_and_vacuum`] to customise the interval.
    pub async fn new(database_url: &str) -> anyhow::Result<Self> {
        Self::with_config(database_url, PostgresConfig::default()).await
    }

    /// Create a new PostgreSQL backend with custom configuration.
    ///
    /// Starts the background VACUUM scheduler with the default 24-hour interval.
    pub async fn with_config(database_url: &str, config: PostgresConfig) -> anyhow::Result<Self> {
        Self::with_config_and_vacuum(
            database_url,
            config,
            Duration::from_secs(86_400), // 24 h
        )
        .await
    }

    /// Create a new PostgreSQL backend with a configurable VACUUM interval.
    ///
    /// The VACUUM task runs `VACUUM (ANALYZE)` on the database at the given
    /// `vacuum_interval`.  It listens on an internal watch channel for a shutdown
    /// signal; call [`PostgresBackend::shutdown`] or drop the backend to stop it.
    pub async fn with_config_and_vacuum(
        database_url: &str,
        config: PostgresConfig,
        vacuum_interval: Duration,
    ) -> anyhow::Result<Self> {
        use sqlx::Executor;

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

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Spawn background VACUUM task.
        let vacuum_pool = pool.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(vacuum_interval);
            let mut rx = shutdown_rx;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = vacuum_pool.execute("VACUUM (ANALYZE)").await {
                            tracing::warn!("Background VACUUM failed: {}", e);
                        } else {
                            tracing::debug!("Background VACUUM (ANALYZE) completed");
                        }
                    }
                    _ = rx.changed() => {
                        if *rx.borrow() {
                            tracing::debug!("PostgresBackend vacuum task shutting down");
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            pool,
            config,
            shutdown_tx,
        })
    }

    /// Signal the background VACUUM task to stop.
    ///
    /// The task exits on the next tick after this call.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Initialize database schema with migrations
    pub async fn init_schema(&self) -> anyhow::Result<()> {
        schema::init_schema(&self.pool).await
    }

    /// Run VACUUM maintenance on all tables
    pub async fn vacuum(&self) -> anyhow::Result<()> {
        use sqlx::Executor;
        tracing::info!("Running VACUUM on PostgreSQL database");
        self.pool.execute("VACUUM ANALYZE").await?;
        Ok(())
    }

    /// Run REINDEX on all tables
    pub async fn reindex(&self) -> anyhow::Result<()> {
        use sqlx::Executor;
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
