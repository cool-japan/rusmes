//! SMTP server implementation

use crate::session::{SmtpConfig, SmtpSessionHandler};
use rusmes_auth::AuthBackend;
use rusmes_core::{MailProcessorRouter, RateLimiter};
use rusmes_storage::StorageBackend;
use std::sync::Arc;
use tokio::net::TcpListener;

/// SMTP server
pub struct SmtpServer {
    config: SmtpConfig,
    bind_addr: String,
    listener: Option<TcpListener>,
    tls_config: Option<Arc<rustls::ServerConfig>>,
    processor_router: Arc<MailProcessorRouter>,
    auth_backend: Arc<dyn AuthBackend>,
    rate_limiter: Arc<RateLimiter>,
    storage_backend: Arc<dyn StorageBackend>,
}

impl SmtpServer {
    /// Create a new SMTP server
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: SmtpConfig,
        bind_addr: impl Into<String>,
        processor_router: Arc<MailProcessorRouter>,
        auth_backend: Arc<dyn AuthBackend>,
        rate_limiter: Arc<RateLimiter>,
        storage_backend: Arc<dyn StorageBackend>,
    ) -> Self {
        Self {
            config,
            bind_addr: bind_addr.into(),
            listener: None,
            tls_config: None,
            processor_router,
            auth_backend,
            rate_limiter,
            storage_backend,
        }
    }

    /// Set TLS configuration
    pub fn with_tls(mut self, tls_config: Arc<rustls::ServerConfig>) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    /// Bind to the configured address
    pub async fn bind(&mut self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!("SMTP server listening on {}", self.bind_addr);
        self.listener = Some(listener);
        Ok(())
    }

    /// Serve incoming connections
    pub async fn serve(&self) -> anyhow::Result<()> {
        let listener = self
            .listener
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Server not bound - call bind() first"))?;

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            tracing::info!("New SMTP connection from {}", remote_addr);

            // Check connection rate limit
            let ip = remote_addr.ip();
            if !self.rate_limiter.allow_connection(ip).await {
                tracing::warn!("Connection rate limit exceeded for {}", ip);
                // Drop the connection without sending a response
                drop(stream);
                continue;
            }

            let session = SmtpSessionHandler::new(
                stream,
                remote_addr,
                self.config.clone(),
                self.processor_router.clone(),
                self.auth_backend.clone(),
                self.rate_limiter.clone(),
                self.storage_backend.clone(),
            );

            let rate_limiter = self.rate_limiter.clone();

            // Spawn a new task for each connection
            tokio::spawn(async move {
                if let Err(e) = session.handle().await {
                    tracing::error!("SMTP session error from {}: {}", remote_addr, e);
                }
                // Release the connection when done
                rate_limiter.release_connection(ip).await;
            });
        }
    }

    /// Run the server (bind and serve)
    pub async fn run(mut self) -> anyhow::Result<()> {
        self.bind().await?;
        self.serve().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusmes_metrics::MetricsCollector;
    use rusmes_proto::Username;
    use rusmes_storage::{MailboxStore, MessageStore, MetadataStore};

    #[allow(dead_code)]
    struct DummyAuthBackend;

    #[async_trait::async_trait]
    impl AuthBackend for DummyAuthBackend {
        async fn authenticate(
            &self,
            _username: &rusmes_proto::Username,
            _password: &str,
        ) -> anyhow::Result<bool> {
            Ok(true)
        }

        async fn verify_identity(
            &self,
            _username: &rusmes_proto::Username,
        ) -> anyhow::Result<bool> {
            Ok(true)
        }

        async fn list_users(&self) -> anyhow::Result<Vec<rusmes_proto::Username>> {
            Ok(Vec::new())
        }

        async fn create_user(
            &self,
            _username: &rusmes_proto::Username,
            _password: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn delete_user(&self, _username: &rusmes_proto::Username) -> anyhow::Result<()> {
            Ok(())
        }

        async fn change_password(
            &self,
            _username: &rusmes_proto::Username,
            _new_password: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[allow(dead_code)]
    struct DummyMailboxStore;

    #[async_trait::async_trait]
    impl MailboxStore for DummyMailboxStore {
        async fn create_mailbox(
            &self,
            _path: &rusmes_storage::MailboxPath,
        ) -> anyhow::Result<rusmes_storage::MailboxId> {
            Ok(rusmes_storage::MailboxId::new())
        }

        async fn delete_mailbox(&self, _id: &rusmes_storage::MailboxId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn rename_mailbox(
            &self,
            _id: &rusmes_storage::MailboxId,
            _new_path: &rusmes_storage::MailboxPath,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn get_mailbox(
            &self,
            _id: &rusmes_storage::MailboxId,
        ) -> anyhow::Result<Option<rusmes_storage::Mailbox>> {
            Ok(None)
        }

        async fn list_mailboxes(
            &self,
            _user: &Username,
        ) -> anyhow::Result<Vec<rusmes_storage::Mailbox>> {
            Ok(Vec::new())
        }

        async fn get_user_inbox(
            &self,
            _user: &Username,
        ) -> anyhow::Result<Option<rusmes_storage::MailboxId>> {
            Ok(None)
        }

        async fn subscribe_mailbox(
            &self,
            _user: &Username,
            _mailbox_name: String,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn unsubscribe_mailbox(
            &self,
            _user: &Username,
            _mailbox_name: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn list_subscriptions(&self, _user: &Username) -> anyhow::Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    #[allow(dead_code)]
    struct DummyMessageStore;

    #[async_trait::async_trait]
    impl MessageStore for DummyMessageStore {
        async fn append_message(
            &self,
            _mailbox_id: &rusmes_storage::MailboxId,
            _message: rusmes_proto::Mail,
        ) -> anyhow::Result<rusmes_storage::MessageMetadata> {
            Ok(rusmes_storage::MessageMetadata::new(
                rusmes_proto::MessageId::new(),
                rusmes_storage::MailboxId::new(),
                1,
                rusmes_storage::MessageFlags::new(),
                0,
            ))
        }

        async fn get_message(
            &self,
            _message_id: &rusmes_proto::MessageId,
        ) -> anyhow::Result<Option<rusmes_proto::Mail>> {
            Ok(None)
        }

        async fn delete_messages(
            &self,
            _message_ids: &[rusmes_proto::MessageId],
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn set_flags(
            &self,
            _message_ids: &[rusmes_proto::MessageId],
            _flags: rusmes_storage::MessageFlags,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn search(
            &self,
            _mailbox_id: &rusmes_storage::MailboxId,
            _criteria: rusmes_storage::SearchCriteria,
        ) -> anyhow::Result<Vec<rusmes_proto::MessageId>> {
            Ok(Vec::new())
        }

        async fn copy_messages(
            &self,
            _message_ids: &[rusmes_proto::MessageId],
            _dest_mailbox_id: &rusmes_storage::MailboxId,
        ) -> anyhow::Result<Vec<rusmes_storage::MessageMetadata>> {
            Ok(Vec::new())
        }

        async fn get_mailbox_messages(
            &self,
            _mailbox_id: &rusmes_storage::MailboxId,
        ) -> anyhow::Result<Vec<rusmes_storage::MessageMetadata>> {
            Ok(Vec::new())
        }
    }

    #[allow(dead_code)]
    struct DummyMetadataStore;

    #[async_trait::async_trait]
    impl MetadataStore for DummyMetadataStore {
        async fn get_user_quota(&self, _user: &Username) -> anyhow::Result<rusmes_storage::Quota> {
            Ok(rusmes_storage::Quota::new(0, 1024 * 1024 * 1024))
        }

        async fn set_user_quota(
            &self,
            _user: &Username,
            _quota: rusmes_storage::Quota,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn get_mailbox_counters(
            &self,
            _mailbox_id: &rusmes_storage::MailboxId,
        ) -> anyhow::Result<rusmes_storage::MailboxCounters> {
            Ok(rusmes_storage::MailboxCounters::default())
        }
    }

    #[allow(dead_code)]
    struct DummyStorageBackend {
        mailbox_store: Arc<dyn MailboxStore>,
        message_store: Arc<dyn MessageStore>,
        metadata_store: Arc<dyn MetadataStore>,
    }

    impl StorageBackend for DummyStorageBackend {
        fn mailbox_store(&self) -> Arc<dyn MailboxStore> {
            self.mailbox_store.clone()
        }

        fn message_store(&self) -> Arc<dyn MessageStore> {
            self.message_store.clone()
        }

        fn metadata_store(&self) -> Arc<dyn MetadataStore> {
            self.metadata_store.clone()
        }
    }

    #[test]
    fn test_server_creation() {
        let config = SmtpConfig::default();
        let metrics = Arc::new(MetricsCollector::new());
        let router = Arc::new(MailProcessorRouter::new(metrics));
        let auth = Arc::new(DummyAuthBackend);
        let rate_limiter = Arc::new(rusmes_core::RateLimiter::new(
            rusmes_core::RateLimitConfig::default(),
        ));
        let storage: Arc<dyn StorageBackend> = Arc::new(DummyStorageBackend {
            mailbox_store: Arc::new(DummyMailboxStore),
            message_store: Arc::new(DummyMessageStore),
            metadata_store: Arc::new(DummyMetadataStore),
        });

        let server = SmtpServer::new(
            config.clone(),
            "127.0.0.1:2525",
            router,
            auth,
            rate_limiter,
            storage,
        );

        assert_eq!(server.bind_addr, "127.0.0.1:2525");
        assert_eq!(server.config.hostname, config.hostname);
    }
}
