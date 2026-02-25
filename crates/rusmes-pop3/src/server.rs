//! POP3 server implementation

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
}

impl Pop3Server {
    /// Create a new POP3 server
    pub fn new(
        bind_addr: String,
        config: Pop3Config,
        auth_backend: Arc<dyn AuthBackend>,
        storage_backend: Arc<dyn StorageBackend>,
    ) -> Self {
        Self {
            bind_addr,
            config,
            auth_backend,
            storage_backend,
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
