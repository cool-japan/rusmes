//! POP3 server implementation

use crate::maildrop_lock::MaildropLockManager;
use crate::session::{Pop3Config, Pop3Session};
use rusmes_auth::AuthBackend;
use rusmes_storage::StorageBackend;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

/// POP3 server
pub struct Pop3Server {
    bind_addr: String,
    config: Pop3Config,
    auth_backend: Arc<dyn AuthBackend>,
    storage_backend: Arc<dyn StorageBackend>,
    /// Shared per-user maildrop lock registry. One `MaildropLockManager` per
    /// `Pop3Server` so that all sessions on this listener participate in the
    /// same exclusive-access protocol (RFC 1939 §3).
    maildrop_locks: MaildropLockManager,
}

impl Pop3Server {
    /// Create a new POP3 server.
    ///
    /// A fresh `MaildropLockManager` is allocated; if you need to share locks
    /// across multiple listeners (for example, plain-text + TLS on different
    /// ports for the same users), use [`Pop3Server::with_maildrop_locks`].
    pub fn new(
        bind_addr: String,
        config: Pop3Config,
        auth_backend: Arc<dyn AuthBackend>,
        storage_backend: Arc<dyn StorageBackend>,
    ) -> Self {
        Self::with_maildrop_locks(
            bind_addr,
            config,
            auth_backend,
            storage_backend,
            MaildropLockManager::new(),
        )
    }

    /// Create a new POP3 server sharing an existing maildrop lock registry.
    pub fn with_maildrop_locks(
        bind_addr: String,
        config: Pop3Config,
        auth_backend: Arc<dyn AuthBackend>,
        storage_backend: Arc<dyn StorageBackend>,
        maildrop_locks: MaildropLockManager,
    ) -> Self {
        Self {
            bind_addr,
            config,
            auth_backend,
            storage_backend,
            maildrop_locks,
        }
    }

    /// Start the server
    pub async fn start(self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        info!("POP3 server listening on {}", self.bind_addr);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let session = Pop3Session::new(
                        addr,
                        self.config.clone(),
                        Arc::clone(&self.auth_backend),
                        Arc::clone(&self.storage_backend),
                        self.maildrop_locks.clone(),
                    );

                    tokio::spawn(async move {
                        if let Err(e) = session.handle(stream).await {
                            error!("Session error from {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}
