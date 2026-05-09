//! RusMES server main binary
//!
//! Entry point for the `rusmes-server` orchestrator. Parses CLI arguments,
//! loads + validates configuration, optionally exits early for `--check-config`,
//! constructs the authentication and storage backends via the `bootstrap`
//! module, and then spawns each enabled protocol server (SMTP, IMAP, JMAP,
//! POP3) under a unified signal-handling loop.

mod connection_limits;
mod privileges;

use anyhow::{Context, Result};
use clap::Parser;
use rusmes_config::ServerConfig;
use rusmes_core::factory::{create_mailet_with_storage, create_matcher};
use rusmes_core::{MailProcessorRouter, ProcessingStep, Processor, RateLimitConfig, RateLimiter};
use rusmes_metrics::{set_global_metrics, MetricsCollector};
use rusmes_pop3::{Pop3Config, Pop3Server};
use rusmes_proto::MailState;
use rusmes_server::bootstrap::{
    build_auth_backend, build_storage_backend, load_and_validate, PidFile,
};
use rusmes_server::cli::Cli;
use rusmes_smtp::{SmtpConfig, SmtpServer};
use rusmes_storage::StorageBackend;
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
            max_messages_per_window: rate_limit.max_messages_per_hour as usize,
            window_duration: std::time::Duration::from_secs(window_secs),
            ..Default::default()
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

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = Cli::parse();
    let (config_path, used_positional_fallback) = cli.resolve_config_path();
    if used_positional_fallback {
        eprintln!(
            "warning: passing the config path as a positional argument is deprecated; \
             use `-c {}` instead. The positional fallback will be removed in the next release.",
            config_path.display()
        );
    }

    if cli.check_config {
        return run_check_config(&config_path);
    }

    tracing::info!("Starting RusMES server...");

    // Load + validate the configuration. We deliberately do NOT fall back to a
    // synthesized default — the bootstrap path needs an explicit storage and
    // (optionally) auth section. Operators that previously relied on the
    // implicit default should now ship a `rusmes.toml`.
    let config = load_and_validate(&config_path).with_context(|| {
        format!(
            "failed to load configuration from {}",
            config_path.display()
        )
    })?;

    let config_path_str = config_path.to_string_lossy().to_string();

    tracing::info!("Server domain: {}", config.domain);
    tracing::info!("Postmaster: {}", config.postmaster);
    tracing::info!("Runtime dir: {}", config.runtime_dir);

    // Write the PID file before opening any sockets so external supervisors
    // can attach immediately.
    let pid_file = PidFile::write(&config.runtime_dir).await?;

    // Initialize storage backend via the cluster-3 factory.
    let storage: Arc<dyn StorageBackend> = build_storage_backend(&config.storage).await?;

    // Initialize metrics
    let metrics = Arc::new(MetricsCollector::new());
    // Register the global handle so protocol handlers (SMTP, IMAP, POP3) write
    // into the same instance that the HTTP scrape endpoint reads from.
    if let Err(e) = set_global_metrics((*metrics).clone()) {
        tracing::warn!("Global metrics already initialized: {}", e);
    }

    // Build processor router
    let router = build_processor_router(&config, metrics.clone(), storage.clone()).await?;

    // Initialize authentication backend via the cluster-1A factory.
    let auth_backend = build_auth_backend(&config).await?;

    // Initialize rate limiter (wrapped in RwLock for hot-reload)
    let rate_limit_config = if let Some(rl_config) = &config.smtp.rate_limit {
        let window_secs = rl_config.window_duration_seconds().unwrap_or_else(|e| {
            tracing::warn!("Invalid rate limit window duration, using default: {}", e);
            3600
        });
        RateLimitConfig {
            max_connections_per_ip: rl_config.max_connections_per_ip,
            max_messages_per_window: rl_config.max_messages_per_hour as usize,
            window_duration: std::time::Duration::from_secs(window_secs),
            runtime_dir: Some(std::path::PathBuf::from(&config.runtime_dir)),
            ..Default::default()
        }
    } else {
        RateLimitConfig {
            runtime_dir: Some(std::path::PathBuf::from(&config.runtime_dir)),
            ..Default::default()
        }
    };
    // Capture persistence settings before moving config into the limiter.
    let persist_settings = rate_limit_config
        .runtime_dir
        .clone()
        .map(|dir| (dir, rate_limit_config.persist_interval_secs.unwrap_or(60)));
    let rate_limiter = Arc::new(RateLimiter::new(rate_limit_config));

    // Spawn the periodic snapshot task only when a runtime_dir is configured.
    if let Some((dir, interval_secs)) = persist_settings {
        rate_limiter.start_persistence_task(dir, std::time::Duration::from_secs(interval_secs));
    }

    // Wrap config in RwLock for hot-reload
    let current_config = Arc::new(RwLock::new(config.clone()));

    // -------------------------------------------------------------------------
    // Privilege drop: resolve targets from config and apply before spawning.
    //
    // ORDERING CAVEAT: The current architecture binds all listener sockets
    // *inside* tokio::spawn closures (i.e., after this point).  This means
    // that when `run_as_user`, `run_as_group`, or `chroot` are set, privileged
    // ports (<1024) will fail to bind after the privilege drop has taken effect.
    // Operators using these fields must either:
    //   (a) bind to non-privileged ports (≥1024) and use a port-forwarding
    //       rule (nftables REDIRECT, iptables REDIRECT, CAP_NET_BIND_SERVICE),
    //   (b) wait for the planned listener-pre-bind refactor that hoists all
    //       TcpListener::bind calls above the first tokio::spawn.
    // Tracked in crates/rusmes-server/TODO.md.
    // -------------------------------------------------------------------------
    let _uid = privileges::resolve_uid(&config.run_as_user)?;
    let _gid = privileges::resolve_gid(&config.run_as_group)?;
    let _chroot_dir = if config.chroot {
        Some(std::path::PathBuf::from(&config.runtime_dir))
    } else {
        None
    };
    let privilege_drop = privileges::PrivilegeDrop {
        chroot_dir: _chroot_dir,
        uid: _uid,
        gid: _gid,
    };
    privilege_drop.apply()?;

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
        blocked_networks: config
            .security
            .as_ref()
            .map(|s| {
                s.blocked_ips
                    .iter()
                    .filter_map(|cidr| {
                        cidr.parse::<ipnetwork::IpNetwork>()
                            .map_err(|e| {
                                tracing::warn!(cidr = %cidr, error = %e, "skipping unparseable blocked_ip entry");
                            })
                            .ok()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        data_tempfile_threshold: SmtpConfig::default().data_tempfile_threshold,
        data_spill_dir: SmtpConfig::default().data_spill_dir,
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
                ..Pop3Config::default()
            };
            let server = Pop3Server::new(pop3_bind, pop3_cfg, pop3_auth, pop3_storage);
            if let Err(e) = server.start().await {
                tracing::error!("POP3 server error: {}", e);
            }
        }))
    } else {
        None
    };

    // Spawn metrics HTTP server if configured
    let mut metrics_handle = if let Some(ref metrics_config) = config.metrics {
        if metrics_config.enabled {
            let mc = metrics_config.clone();
            let metrics_clone = (*metrics).clone();
            tracing::info!("Starting metrics HTTP server on {}", mc.bind_address);
            Some(tokio::spawn(async move {
                if let Err(e) = metrics_clone.start_http_server(mc).await {
                    tracing::error!("Metrics HTTP server error: {}", e);
                }
            }))
        } else {
            None
        }
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
                    handle_config_reload(&config_path_str, current_config.clone(), rate_limiter.clone()).await;
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
                _ = async {
                    if let Some(ref mut handle) = metrics_handle {
                        handle.await
                    } else {
                        std::future::pending::<()>().await;
                        Ok(())
                    }
                } => {
                    tracing::error!("Metrics HTTP server exited unexpectedly");
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
                _ = async {
                    if let Some(ref mut handle) = metrics_handle {
                        handle.await
                    } else {
                        std::future::pending::<()>().await;
                        Ok(())
                    }
                } => {
                    tracing::error!("Metrics HTTP server exited unexpectedly");
                    break;
                }
            }
        }
    }

    pid_file.cleanup().await;

    tracing::info!("RusMES server shutdown complete");
    Ok(())
}

/// Implementation of the `--check-config` flag: load + validate, print a
/// human-readable summary, exit 0 / 1 accordingly.
fn run_check_config(config_path: &std::path::Path) -> Result<()> {
    match load_and_validate(config_path) {
        Ok(cfg) => {
            eprintln!(
                "Configuration at {} OK: domain={}, storage={:?}, auth={}",
                config_path.display(),
                cfg.domain,
                cfg.storage,
                cfg.auth
                    .as_ref()
                    .map(|c| match c {
                        rusmes_config::AuthConfig::File { .. } => "file",
                        rusmes_config::AuthConfig::Sql { .. } => "sql",
                        rusmes_config::AuthConfig::Ldap { .. } => "ldap",
                        rusmes_config::AuthConfig::OAuth2 { .. } => "oauth2",
                    })
                    .unwrap_or("(unconfigured — file backend default applies)"),
            );
            Ok(())
        }
        Err(e) => {
            eprintln!(
                "Configuration at {} INVALID: {:#}",
                config_path.display(),
                e
            );
            std::process::exit(1);
        }
    }
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
                timeout_ms: None,
                error_policy: rusmes_core::MailetErrorPolicy::default(),
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
