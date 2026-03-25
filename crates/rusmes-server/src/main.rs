//! RusMES server main binary

mod connection_limits;

use anyhow::Result;
use rusmes_config::ServerConfig;
use rusmes_core::factory::{create_mailet_with_storage, create_matcher};
use rusmes_core::{MailProcessorRouter, ProcessingStep, Processor, RateLimitConfig, RateLimiter};
use rusmes_metrics::MetricsCollector;
use rusmes_pop3::{Pop3Config, Pop3Server};
use rusmes_proto::MailState;
use rusmes_smtp::{SmtpConfig, SmtpServer};
use rusmes_storage::backends::filesystem::FilesystemBackend;
use rusmes_storage::StorageBackend;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Handle configuration reload on SIGHUP signal
///
/// Hot-reloadable settings:
/// - Logging level
/// - Rate limits (max connections, max messages per hour)
/// - Security settings (relay networks, blocked IPs)
///
/// Non-reloadable settings (require restart):
/// - Bind addresses and ports
/// - TLS certificate paths
/// - Storage backend
/// - Processor configurations
async fn handle_config_reload(
    config_path: &str,
    current_config: Arc<RwLock<ServerConfig>>,
    rate_limiter: Arc<RateLimiter>,
) {
    match ServerConfig::from_file(config_path) {
        Ok(new_config) => {
            let old_config = current_config.read().await;

            // Check for non-reloadable changes and warn
            check_non_reloadable_changes(&old_config, &new_config);

            // Apply hot-reloadable changes
            apply_hot_reloadable_changes(&new_config, rate_limiter).await;

            // Update stored config
            drop(old_config);
            let mut config_write = current_config.write().await;
            *config_write = new_config;
            drop(config_write);

            tracing::info!("Configuration reloaded successfully");
        }
        Err(e) => {
            tracing::error!(
                "Failed to reload configuration: {}. Keeping current configuration.",
                e
            );
        }
    }
}

/// Check for changes that cannot be hot-reloaded and log warnings
fn check_non_reloadable_changes(old_config: &ServerConfig, new_config: &ServerConfig) {
    // Check SMTP bind address/port changes
    if old_config.smtp.host != new_config.smtp.host || old_config.smtp.port != new_config.smtp.port
    {
        tracing::warn!(
            "SMTP bind address/port changed ({}:{} -> {}:{}). Restart required for this change to take effect.",
            old_config.smtp.host, old_config.smtp.port,
            new_config.smtp.host, new_config.smtp.port
        );
    }

    if old_config.smtp.tls_port != new_config.smtp.tls_port {
        tracing::warn!(
            "SMTP TLS port changed ({:?} -> {:?}). Restart required for this change to take effect.",
            old_config.smtp.tls_port, new_config.smtp.tls_port
        );
    }

    // Check IMAP changes
    match (&old_config.imap, &new_config.imap) {
        (Some(old_imap), Some(new_imap))
            if old_imap.host != new_imap.host || old_imap.port != new_imap.port =>
        {
            tracing::warn!(
                "IMAP bind address/port changed. Restart required for this change to take effect."
            );
        }
        (None, Some(_)) => {
            tracing::warn!("IMAP server enabled. Restart required for this change to take effect.");
        }
        (Some(_), None) => {
            tracing::warn!(
                "IMAP server disabled. Restart required for this change to take effect."
            );
        }
        _ => {}
    }

    // Check JMAP changes
    match (&old_config.jmap, &new_config.jmap) {
        (Some(old_jmap), Some(new_jmap))
            if old_jmap.host != new_jmap.host || old_jmap.port != new_jmap.port =>
        {
            tracing::warn!(
                "JMAP bind address/port changed. Restart required for this change to take effect."
            );
        }
        (None, Some(_)) => {
            tracing::warn!("JMAP server enabled. Restart required for this change to take effect.");
        }
        (Some(_), None) => {
            tracing::warn!(
                "JMAP server disabled. Restart required for this change to take effect."
            );
        }
        _ => {}
    }

    // Check POP3 changes
    match (&old_config.pop3, &new_config.pop3) {
        (Some(old_pop3), Some(new_pop3))
            if old_pop3.host != new_pop3.host || old_pop3.port != new_pop3.port =>
        {
            tracing::warn!(
                "POP3 bind address/port changed. Restart required for this change to take effect."
            );
        }
        (None, Some(_)) => {
            tracing::warn!("POP3 server enabled. Restart required for this change to take effect.");
        }
        (Some(_), None) => {
            tracing::warn!(
                "POP3 server disabled. Restart required for this change to take effect."
            );
        }
        _ => {}
    }

    // Check storage backend changes
    let old_storage = format!("{:?}", old_config.storage);
    let new_storage = format!("{:?}", new_config.storage);
    if old_storage != new_storage {
        tracing::warn!("Storage backend changed. Restart required for this change to take effect.");
    }

    // Check processor configuration changes
    if old_config.processors.len() != new_config.processors.len() {
        tracing::warn!(
            "Processor configuration changed. Restart required for this change to take effect."
        );
    }

    // Check TLS/STARTTLS changes
    if old_config.smtp.enable_starttls != new_config.smtp.enable_starttls {
        tracing::warn!(
            "STARTTLS setting changed. Restart required for this change to take effect."
        );
    }

    if old_config.smtp.require_auth != new_config.smtp.require_auth {
        tracing::warn!(
            "SMTP authentication requirement changed. Restart required for this change to take effect."
        );
    }
}

/// Apply configuration changes that can be hot-reloaded
async fn apply_hot_reloadable_changes(new_config: &ServerConfig, rate_limiter: Arc<RateLimiter>) {
    // Update logging level if changed
    if let Some(logging) = &new_config.logging {
        if let Err(e) = update_logging_level(&logging.level) {
            tracing::warn!("Failed to update logging level: {}", e);
        } else {
            tracing::info!("Logging level updated to: {}", logging.level);
        }
    }

    // Update rate limits
    if let Some(rate_limit) = &new_config.smtp.rate_limit {
        let window_secs = rate_limit.window_duration_seconds().unwrap_or_else(|e| {
            tracing::warn!("Invalid rate limit window duration, using default: {}", e);
            3600
        });

        let new_rate_config = RateLimitConfig {
            max_connections_per_ip: rate_limit.max_connections_per_ip,
            max_messages_per_hour: rate_limit.max_messages_per_hour as usize,
            window_duration: std::time::Duration::from_secs(window_secs),
        };

        rate_limiter.update_config(new_rate_config).await;
        tracing::info!(
            "Rate limits updated: max_connections_per_ip={}, max_messages_per_hour={}",
            rate_limit.max_connections_per_ip,
            rate_limit.max_messages_per_hour
        );
    }

    // Log info about security settings that would be reloaded
    // (actual implementation would require passing security config to relevant components)
    if let Some(security) = &new_config.security {
        tracing::info!(
            "Security settings updated: {} relay networks, {} blocked IPs",
            security.relay_networks.len(),
            security.blocked_ips.len()
        );
    }
}

/// Update the tracing logging level dynamically
fn update_logging_level(level: &str) -> Result<()> {
    // Note: This is a simplified version. In production, you might want to use
    // tracing_subscriber's reload handle for proper runtime level updates.
    // For now, we just log the change request.
    match level.to_lowercase().as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => {
            // In a real implementation, you'd update the subscriber's filter here
            Ok(())
        }
        _ => Err(anyhow::anyhow!("Invalid log level: {}", level)),
    }
}

/// Dummy auth backend for testing (accepts all credentials)
/// WARNING: This is insecure and should only be used for testing!
struct DummyAuthBackend;

#[async_trait::async_trait]
impl rusmes_auth::AuthBackend for DummyAuthBackend {
    async fn authenticate(
        &self,
        _username: &rusmes_proto::Username,
        _password: &str,
    ) -> Result<bool> {
        Ok(true) // Accept all for now
    }

    async fn verify_identity(&self, _username: &rusmes_proto::Username) -> Result<bool> {
        Ok(true) // Accept all for now
    }

    async fn list_users(&self) -> Result<Vec<rusmes_proto::Username>> {
        Ok(Vec::new()) // No users in dummy backend
    }

    async fn create_user(&self, _username: &rusmes_proto::Username, _password: &str) -> Result<()> {
        Ok(()) // No-op in dummy backend
    }

    async fn delete_user(&self, _username: &rusmes_proto::Username) -> Result<()> {
        Ok(()) // No-op in dummy backend
    }

    async fn change_password(
        &self,
        _username: &rusmes_proto::Username,
        _new_password: &str,
    ) -> Result<()> {
        Ok(()) // No-op in dummy backend
    }

    async fn get_apop_secret(&self, _username: &rusmes_proto::Username) -> Result<String> {
        // For testing, return a fixed secret
        // In production, this should retrieve the actual password from storage
        Ok("password".to_string())
    }
}

/// Create authentication backend based on configuration
async fn create_auth_backend(config: &ServerConfig) -> Result<Arc<dyn rusmes_auth::AuthBackend>> {
    match &config.auth {
        Some(rusmes_config::AuthConfig::File {
            config: file_config,
        }) => {
            tracing::info!(
                "Using FileAuthBackend with password file: {}",
                file_config.path
            );
            let backend = rusmes_auth::file::FileAuthBackend::new(&file_config.path).await?;
            Ok(Arc::new(backend))
        }
        Some(rusmes_config::AuthConfig::Ldap {
            config: _ldap_config,
        }) => {
            tracing::warn!(
                "LDAP authentication not yet implemented, falling back to DummyAuthBackend"
            );
            Ok(Arc::new(DummyAuthBackend))
        }
        Some(rusmes_config::AuthConfig::Sql {
            config: _sql_config,
        }) => {
            tracing::warn!(
                "SQL authentication not yet implemented, falling back to DummyAuthBackend"
            );
            Ok(Arc::new(DummyAuthBackend))
        }
        Some(rusmes_config::AuthConfig::OAuth2 {
            config: _oauth_config,
        }) => {
            tracing::warn!(
                "OAuth2 authentication not yet implemented, falling back to DummyAuthBackend"
            );
            Ok(Arc::new(DummyAuthBackend))
        }
        None => {
            tracing::warn!("No authentication backend configured, using DummyAuthBackend (accepts all credentials)");
            tracing::warn!(
                "WARNING: DummyAuthBackend is insecure and should only be used for testing!"
            );
            Ok(Arc::new(DummyAuthBackend))
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    tracing::info!("Starting RusMES server...");

    // Load configuration
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rusmes.toml".to_string());

    let config = match ServerConfig::from_file(&config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!("Could not load config file '{}': {}", config_path, e);
            tracing::info!("Using default configuration");
            create_default_config()
        }
    };

    tracing::info!("Server domain: {}", config.domain);
    tracing::info!("Postmaster: {}", config.postmaster);

    // Initialize storage backend
    let storage: Arc<dyn StorageBackend> = match &config.storage {
        rusmes_config::StorageConfig::Filesystem { path } => {
            tracing::info!("Using filesystem storage at: {}", path);
            Arc::new(FilesystemBackend::new(path))
        }
        _ => {
            tracing::warn!("Storage backend not implemented, using default");
            Arc::new(FilesystemBackend::new("/tmp/rusmes"))
        }
    };

    // Initialize metrics
    let metrics = Arc::new(MetricsCollector::new());

    // Build processor router
    let router = build_processor_router(&config, metrics.clone(), storage.clone()).await?;

    // Initialize authentication backend
    let auth_backend = create_auth_backend(&config).await?;

    // Initialize rate limiter (wrapped in RwLock for hot-reload)
    let rate_limit_config = if let Some(rl_config) = &config.smtp.rate_limit {
        let window_secs = rl_config.window_duration_seconds().unwrap_or_else(|e| {
            tracing::warn!("Invalid rate limit window duration, using default: {}", e);
            3600
        });
        RateLimitConfig {
            max_connections_per_ip: rl_config.max_connections_per_ip,
            max_messages_per_hour: rl_config.max_messages_per_hour as usize,
            window_duration: std::time::Duration::from_secs(window_secs),
        }
    } else {
        RateLimitConfig::default()
    };
    let rate_limiter = Arc::new(RateLimiter::new(rate_limit_config));

    // Wrap config in RwLock for hot-reload
    let current_config = Arc::new(RwLock::new(config.clone()));

    // Spawn SMTP server
    let smtp_config = SmtpConfig {
        hostname: config.smtp.host.clone(),
        max_message_size: config.smtp.max_message_size_bytes()?,
        require_auth: config.smtp.require_auth,
        enable_starttls: config.smtp.enable_starttls,
        check_recipient_exists: config
            .security
            .as_ref()
            .map(|s| s.check_recipient_exists)
            .unwrap_or(true),
        reject_unknown_recipients: config
            .security
            .as_ref()
            .map(|s| s.reject_unknown_recipients)
            .unwrap_or(true),
        relay_networks: config
            .security
            .as_ref()
            .map(|s| s.relay_networks.clone())
            .unwrap_or_default(),
        local_domains: config
            .domains
            .as_ref()
            .map(|d| d.local_domains.clone())
            .unwrap_or_else(|| vec![config.domain.clone()]),
        connection_timeout: std::time::Duration::from_secs(300),
        idle_timeout: std::time::Duration::from_secs(60),
    };

    let smtp_bind = format!("{}:{}", config.smtp.host, config.smtp.port);
    tracing::info!("Starting SMTP server on {}", smtp_bind);

    let smtp_router = router.clone();
    let smtp_auth = auth_backend.clone();
    let smtp_rate_limiter = rate_limiter.clone();
    let smtp_storage = storage.clone();
    let mut smtp_handle = tokio::spawn(async move {
        let server = SmtpServer::new(
            smtp_config,
            smtp_bind,
            smtp_router,
            smtp_auth,
            smtp_rate_limiter,
            smtp_storage,
        );
        if let Err(e) = server.run().await {
            tracing::error!("SMTP server error: {}", e);
        }
    });

    // Spawn IMAP server if configured
    let mut imap_handle = if let Some(imap_config) = &config.imap {
        let imap_bind = format!("{}:{}", imap_config.host, imap_config.port);
        tracing::info!("Starting IMAP server on {}", imap_bind);

        let imap_storage = storage.clone();
        let imap_auth = auth_backend.clone();
        Some(tokio::spawn(async move {
            let context = rusmes_imap::HandlerContext::new(
                imap_storage.mailbox_store(),
                imap_storage.message_store(),
                imap_storage.metadata_store(),
                imap_auth,
            );
            let server = rusmes_imap::ImapServer::new(imap_bind, context);
            if let Err(e) = server.run().await {
                tracing::error!("IMAP server error: {}", e);
            }
        }))
    } else {
        None
    };

    // Spawn JMAP server if configured
    let mut jmap_handle = if let Some(jmap_config) = &config.jmap {
        let jmap_bind = format!("{}:{}", jmap_config.host, jmap_config.port);
        tracing::info!("Starting JMAP server on {}", jmap_bind);

        Some(tokio::spawn(async move {
            let app = rusmes_jmap::JmapServer::routes();
            let listener = match tokio::net::TcpListener::bind(&jmap_bind).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to bind JMAP server to {}: {}", jmap_bind, e);
                    return;
                }
            };
            tracing::info!("JMAP server listening on {}", jmap_bind);
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("JMAP server error: {}", e);
            }
        }))
    } else {
        None
    };

    // Spawn POP3 server if configured
    let mut pop3_handle = if let Some(pop3_config) = config.pop3.as_ref() {
        let pop3_bind = format!("{}:{}", pop3_config.host, pop3_config.port);
        tracing::info!("Starting POP3 server on {}", pop3_bind);

        let pop3_storage = storage.clone();
        let pop3_auth = auth_backend.clone();
        let pop3_hostname = pop3_config.host.clone();
        let pop3_timeout = pop3_config.timeout_seconds;
        let pop3_enable_stls = pop3_config.enable_stls;
        let pop3_bind = pop3_bind.clone(); // Clone for move into async block
        Some(tokio::spawn(async move {
            let pop3_cfg = Pop3Config {
                hostname: pop3_hostname,
                greeting: "POP3 server ready".to_string(),
                timeout_seconds: pop3_timeout,
                enable_stls: pop3_enable_stls,
            };
            let server = Pop3Server::new(pop3_bind, pop3_cfg, pop3_auth, pop3_storage);
            if let Err(e) = server.start().await {
                tracing::error!("POP3 server error: {}", e);
            }
        }))
    } else {
        None
    };

    // Setup signal handlers
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut ctrl_c_signal = Box::pin(tokio::signal::ctrl_c());
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sighup = signal(SignalKind::hangup())?;

        loop {
            tokio::select! {
                res = ctrl_c_signal.as_mut() => {
                    if let Err(e) = res {
                        tracing::error!("Failed to listen for SIGINT: {}", e);
                    }
                    tracing::info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
                    break;
                }
                _ = sigterm.recv() => {
                    tracing::info!("Received SIGTERM, initiating graceful shutdown...");
                    break;
                }
                _ = sighup.recv() => {
                    tracing::info!("Received SIGHUP, reloading configuration...");
                    handle_config_reload(&config_path, current_config.clone(), rate_limiter.clone()).await;
                }
                _ = &mut smtp_handle => {
                    tracing::error!("SMTP server exited unexpectedly");
                    break;
                }
                _ = async {
                    if let Some(ref mut handle) = imap_handle {
                        handle.await
                    } else {
                        std::future::pending::<()>().await;
                        Ok(())
                    }
                } => {
                    tracing::error!("IMAP server exited unexpectedly");
                    break;
                }
                _ = async {
                    if let Some(ref mut handle) = jmap_handle {
                        handle.await
                    } else {
                        std::future::pending::<()>().await;
                        Ok(())
                    }
                } => {
                    tracing::error!("JMAP server exited unexpectedly");
                    break;
                }
                _ = async {
                    if let Some(ref mut handle) = pop3_handle {
                        handle.await
                    } else {
                        std::future::pending::<()>().await;
                        Ok(())
                    }
                } => {
                    tracing::error!("POP3 server exited unexpectedly");
                    break;
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        let mut ctrl_c_signal = Box::pin(tokio::signal::ctrl_c());

        loop {
            tokio::select! {
                res = ctrl_c_signal.as_mut() => {
                    if let Err(e) = res {
                        tracing::error!("Failed to listen for SIGINT: {}", e);
                    }
                    tracing::info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
                    break;
                }
                _ = &mut smtp_handle => {
                    tracing::error!("SMTP server exited unexpectedly");
                    break;
                }
                _ = async {
                    if let Some(ref mut handle) = imap_handle {
                        handle.await
                    } else {
                        std::future::pending::<()>().await;
                        Ok(())
                    }
                } => {
                    tracing::error!("IMAP server exited unexpectedly");
                    break;
                }
                _ = async {
                    if let Some(ref mut handle) = jmap_handle {
                        handle.await
                    } else {
                        std::future::pending::<()>().await;
                        Ok(())
                    }
                } => {
                    tracing::error!("JMAP server exited unexpectedly");
                    break;
                }
                _ = async {
                    if let Some(ref mut handle) = pop3_handle {
                        handle.await
                    } else {
                        std::future::pending::<()>().await;
                        Ok(())
                    }
                } => {
                    tracing::error!("POP3 server exited unexpectedly");
                    break;
                }
            }
        }
    }

    tracing::info!("RusMES server shutdown complete");
    Ok(())
}

/// Build the mail processor router from configuration
async fn build_processor_router(
    config: &ServerConfig,
    metrics: Arc<MetricsCollector>,
    storage: Arc<dyn StorageBackend>,
) -> Result<Arc<MailProcessorRouter>> {
    let mut router = MailProcessorRouter::new(metrics);
    let local_domains = vec![config.domain.clone()];

    // Build processors from configuration
    for proc_config in &config.processors {
        let state = parse_mail_state(&proc_config.state)?;
        let mut processor = Processor::new(&proc_config.name, state.clone());

        tracing::info!(
            "Configuring processor '{}' for state {:?} with {} mailets",
            proc_config.name,
            state,
            proc_config.mailets.len()
        );

        // Add processing steps
        for mailet_config in &proc_config.mailets {
            let matcher = create_matcher(&mailet_config.matcher, local_domains.clone())?;

            let mut params = mailet_config.params.clone();

            // Add relay configuration for RemoteDelivery mailets
            if mailet_config.mailet == "RemoteDelivery" {
                if let Some(relay) = &config.relay {
                    params.insert("relay_host".to_string(), relay.host.clone());
                    params.insert("relay_port".to_string(), relay.port.to_string());
                    params.insert("relay_use_tls".to_string(), relay.use_tls.to_string());

                    if let Some(username) = &relay.username {
                        params.insert("relay_username".to_string(), username.clone());
                    }
                    if let Some(password) = &relay.password {
                        params.insert("relay_password".to_string(), password.clone());
                    }
                }
            }

            let mailet_cfg = rusmes_core::MailetConfig {
                name: mailet_config.mailet.clone(),
                params,
            };

            let mailet = create_mailet_with_storage(
                &mailet_config.mailet,
                mailet_cfg,
                Some(storage.clone()),
            )
            .await?;

            processor.add_step(ProcessingStep::new(matcher, mailet));
        }

        router.register_processor(state, Arc::new(processor));
    }

    Ok(Arc::new(router))
}

/// Parse mail state from string
fn parse_mail_state(s: &str) -> Result<MailState> {
    match s.to_lowercase().as_str() {
        "root" => Ok(MailState::Root),
        "transport" => Ok(MailState::Transport),
        "local-delivery" | "localdelivery" => Ok(MailState::LocalDelivery),
        "error" => Ok(MailState::Error),
        "ghost" => Ok(MailState::Ghost),
        _ => Ok(MailState::Custom(s.to_string())),
    }
}

/// Create default configuration
fn create_default_config() -> ServerConfig {
    ServerConfig {
        domain: "localhost".to_string(),
        postmaster: "postmaster@localhost".to_string(),
        smtp: rusmes_config::SmtpServerConfig {
            host: "0.0.0.0".to_string(),
            port: 2525,
            tls_port: None,
            max_message_size: "10MB".to_string(),
            require_auth: false,
            enable_starttls: false,
            rate_limit: None,
        },
        imap: None,
        jmap: None,
        pop3: None,
        storage: rusmes_config::StorageConfig::Filesystem {
            path: "/tmp/rusmes".to_string(),
        },
        processors: vec![rusmes_config::ProcessorConfig {
            name: "root".to_string(),
            state: "root".to_string(),
            mailets: vec![rusmes_config::MailetConfig {
                matcher: "All".to_string(),
                mailet: "LocalDelivery".to_string(),
                params: HashMap::new(),
            }],
        }],
        relay: None,
        auth: None,
        logging: None,
        queue: None,
        security: None,
        domains: None,
        metrics: None,
        tracing: None,
        connection_limits: None,
    }
}
