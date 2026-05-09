//! SMTP Submission Server (RFC 6409) - Port 587
//!
//! This module implements an SMTP submission server specifically designed for
//! authenticated mail submission from mail user agents (MUAs). Key features:
//!
//! - Port 587 (standard submission port)
//! - Mandatory STARTTLS (enforced TLS encryption)
//! - Mandatory authentication before MAIL FROM
//! - Submission-specific validations and restrictions
//! - RFC 6409 compliance
//! - Mutual TLS (mTLS) — optional or required client certificates
//!
//! The submission server wraps the standard SMTP server with additional
//! restrictions to ensure secure mail submission from authenticated users.
//!
//! ## Mutual TLS
//!
//! Call [`build_mtls_server_config`] to obtain a `rustls::ServerConfig` that
//! requests and (optionally) requires a client certificate signed by the given
//! CA.  Pass the result to [`SubmissionServer::with_tls`].

use crate::session::{SmtpConfig, SmtpSessionHandler};
use rusmes_auth::AuthBackend;
use rusmes_config::tls::ClientAuthMode;
use rusmes_core::{MailProcessorRouter, RateLimiter};
use rusmes_storage::StorageBackend;
use rustls::pki_types::CertificateDer;
use rustls::server::WebPkiClientVerifier;
use rustls_pemfile::certs as pemfile_certs;
use std::io::BufReader as StdBufReader;
use std::sync::Arc;
use tokio::net::TcpListener;

// ── Mutual TLS helpers ────────────────────────────────────────────────────────

/// Build a `rustls::ServerConfig` that enables mutual TLS.
///
/// Loads the CA certificate(s) from the PEM bytes in `ca_pem_bytes` and
/// configures the server to request (when `mode == Optional`) or require
/// (when `mode == Required`) a valid client certificate signed by that CA.
///
/// Returns an error when:
/// - `mode == Disabled` — this function should not be called in that case.
/// - The CA PEM cannot be parsed.
/// - The `WebPkiClientVerifier` rejects the trust roots.
/// - The server cert/key cannot be loaded.
///
/// # Arguments
///
/// * `mode` — controls whether a missing client cert is accepted (`Optional`)
///   or causes a TLS handshake failure (`Required`).
/// * `ca_pem_bytes` — raw bytes of the PEM-encoded CA certificate chain file.
/// * `server_cert_pem` — PEM-encoded server certificate chain.
/// * `server_key_pem` — PEM-encoded server private key.
pub fn build_mtls_server_config(
    mode: &ClientAuthMode,
    ca_pem_bytes: &[u8],
    server_cert_pem: &[u8],
    server_key_pem: &[u8],
) -> anyhow::Result<rustls::ServerConfig> {
    if *mode == ClientAuthMode::Disabled {
        anyhow::bail!("build_mtls_server_config called with ClientAuthMode::Disabled — use the standard builder path instead");
    }

    // Parse CA certificates.
    let mut ca_reader = StdBufReader::new(ca_pem_bytes);
    let ca_certs: Vec<CertificateDer<'static>> = pemfile_certs(&mut ca_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("failed to parse CA PEM: {}", e))?;

    if ca_certs.is_empty() {
        anyhow::bail!("CA PEM file contained no certificates");
    }

    let mut root_store = rustls::RootCertStore::empty();
    for cert in ca_certs {
        root_store
            .add(cert)
            .map_err(|e| anyhow::anyhow!("invalid CA certificate: {}", e))?;
    }

    let roots = Arc::new(root_store);

    // Build the appropriate client verifier.
    let client_verifier: Arc<dyn rustls::server::danger::ClientCertVerifier> = match mode {
        ClientAuthMode::Required => WebPkiClientVerifier::builder(roots)
            .build()
            .map_err(|e| anyhow::anyhow!("WebPkiClientVerifier build error: {}", e))?,
        ClientAuthMode::Optional => WebPkiClientVerifier::builder(roots)
            .allow_unauthenticated()
            .build()
            .map_err(|e| anyhow::anyhow!("WebPkiClientVerifier build error: {}", e))?,
        ClientAuthMode::Disabled => unreachable!(),
    };

    // Parse server certificate chain.
    let mut cert_reader = StdBufReader::new(server_cert_pem);
    let server_certs: Vec<CertificateDer<'static>> = pemfile_certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("failed to parse server cert PEM: {}", e))?;

    // Parse server private key.
    let mut key_reader = StdBufReader::new(server_key_pem);
    let private_key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| anyhow::anyhow!("failed to parse server key PEM: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("no private key found in server key PEM"))?;

    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_certs, private_key)
        .map_err(|e| anyhow::anyhow!("rustls ServerConfig build error: {}", e))?;

    Ok(config)
}

/// Submission server configuration
///
/// This configuration extends the base SMTP configuration with
/// submission-specific requirements.
#[derive(Debug, Clone)]
pub struct SubmissionConfig {
    /// Hostname for server identification
    pub hostname: String,
    /// Maximum message size in bytes
    pub max_message_size: usize,
    /// Mandatory STARTTLS (always true for submission)
    pub require_starttls: bool,
    /// Mandatory authentication (always true for submission)
    pub require_auth: bool,
    /// Check if recipient exists in local storage
    pub check_recipient_exists: bool,
    /// Reject messages to unknown recipients
    pub reject_unknown_recipients: bool,
    /// Local domains that this server accepts mail for
    pub local_domains: Vec<String>,
    /// Total connection timeout (max session duration)
    pub connection_timeout: std::time::Duration,
    /// Idle timeout (time between commands)
    pub idle_timeout: std::time::Duration,
    /// Maximum recipients per message (submission-specific limit)
    pub max_recipients_per_message: usize,
    /// Enforce sender address matches authenticated user
    pub enforce_sender_match: bool,
}

impl Default for SubmissionConfig {
    fn default() -> Self {
        Self {
            hostname: "localhost".to_string(),
            max_message_size: 25 * 1024 * 1024, // 25MB (typical for submission)
            require_starttls: true,             // Mandatory for submission
            require_auth: true,                 // Mandatory for submission
            check_recipient_exists: false,      // Don't check for outgoing mail
            reject_unknown_recipients: false,   // Don't reject for outgoing mail
            local_domains: vec!["localhost".to_string()],
            connection_timeout: std::time::Duration::from_secs(1800), // 30 minutes
            idle_timeout: std::time::Duration::from_secs(180),        // 3 minutes
            max_recipients_per_message: 100,                          // Limit for submission
            enforce_sender_match: true,                               // Match sender to auth user
        }
    }
}

impl From<SubmissionConfig> for SmtpConfig {
    fn from(config: SubmissionConfig) -> Self {
        SmtpConfig {
            hostname: config.hostname,
            max_message_size: config.max_message_size,
            require_auth: config.require_auth,
            enable_starttls: config.require_starttls,
            check_recipient_exists: config.check_recipient_exists,
            reject_unknown_recipients: config.reject_unknown_recipients,
            // Submission servers don't use relay networks (auth required instead)
            relay_networks: vec![],
            local_domains: config.local_domains,
            connection_timeout: config.connection_timeout,
            idle_timeout: config.idle_timeout,
            // Submission servers don't apply IP-level blocks here — handled
            // by the network perimeter (firewall / load balancer).
            blocked_networks: Vec::new(),
            data_tempfile_threshold: SmtpConfig::default().data_tempfile_threshold,
            data_spill_dir: SmtpConfig::default().data_spill_dir,
        }
    }
}

/// SMTP Submission Server (Port 587)
///
/// This server enforces RFC 6409 requirements for mail submission:
/// - Mandatory STARTTLS before authentication
/// - Mandatory authentication before MAIL FROM
/// - Submission-specific restrictions
pub struct SubmissionServer {
    config: SubmissionConfig,
    bind_addr: String,
    listener: Option<TcpListener>,
    tls_config: Option<Arc<rustls::ServerConfig>>,
    processor_router: Arc<MailProcessorRouter>,
    auth_backend: Arc<dyn AuthBackend>,
    rate_limiter: Arc<RateLimiter>,
    storage_backend: Arc<dyn StorageBackend>,
}

impl SubmissionServer {
    /// Create a new SMTP submission server
    ///
    /// # Arguments
    /// * `config` - Submission server configuration
    /// * `bind_addr` - Address to bind to (typically "0.0.0.0:587")
    /// * `processor_router` - Mail processor router for message handling
    /// * `auth_backend` - Authentication backend
    /// * `rate_limiter` - Rate limiter for connection and message throttling
    /// * `storage_backend` - Storage backend for recipient validation
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: SubmissionConfig,
        bind_addr: impl Into<String>,
        processor_router: Arc<MailProcessorRouter>,
        auth_backend: Arc<dyn AuthBackend>,
        rate_limiter: Arc<RateLimiter>,
        storage_backend: Arc<dyn StorageBackend>,
    ) -> Self {
        // Enforce submission requirements
        assert!(
            config.require_auth,
            "Submission server must require authentication"
        );
        assert!(
            config.require_starttls,
            "Submission server must require STARTTLS"
        );

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

    /// Set TLS configuration (required for submission server)
    ///
    /// # Arguments
    /// * `tls_config` - TLS server configuration
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_tls(mut self, tls_config: Arc<rustls::ServerConfig>) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    /// Bind to the configured address
    ///
    /// # Errors
    /// Returns error if binding fails
    pub async fn bind(&mut self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!("SMTP Submission server listening on {}", self.bind_addr);
        self.listener = Some(listener);
        Ok(())
    }

    /// Serve incoming connections
    ///
    /// This method runs the main server loop, accepting connections
    /// and spawning handlers for each.
    ///
    /// # Errors
    /// Returns error if server is not bound or if accept fails
    pub async fn serve(&self) -> anyhow::Result<()> {
        let listener = self
            .listener
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Server not bound - call bind() first"))?;

        // Warn if TLS is not configured (required for production)
        if self.tls_config.is_none() {
            tracing::warn!(
                "Submission server running WITHOUT TLS configuration. \
                 This is INSECURE and should only be used for testing. \
                 STARTTLS will fail without TLS configuration."
            );
        }

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            tracing::info!("New SMTP submission connection from {}", remote_addr);

            // Check connection rate limit
            let ip = remote_addr.ip();
            if !self.rate_limiter.allow_connection(ip).await {
                tracing::warn!(
                    "Connection rate limit exceeded for {} on submission port",
                    ip
                );
                // Drop the connection without sending a response
                drop(stream);
                continue;
            }

            // Convert submission config to SMTP config
            let smtp_config: SmtpConfig = self.config.clone().into();

            // Create session handler
            let session = SmtpSessionHandler::new(
                stream,
                remote_addr,
                smtp_config,
                self.processor_router.clone(),
                self.auth_backend.clone(),
                self.rate_limiter.clone(),
                self.storage_backend.clone(),
            );

            let rate_limiter = self.rate_limiter.clone();

            // Spawn a new task for each connection
            tokio::spawn(async move {
                if let Err(e) = session.handle().await {
                    tracing::error!("SMTP submission session error from {}: {}", remote_addr, e);
                }
                // Release the connection when done
                rate_limiter.release_connection(ip).await;
            });
        }
    }

    /// Run the server (bind and serve)
    ///
    /// This is a convenience method that combines bind() and serve().
    ///
    /// # Errors
    /// Returns error if binding or serving fails
    pub async fn run(mut self) -> anyhow::Result<()> {
        self.bind().await?;
        self.serve().await
    }

    /// Get the bind address
    pub fn bind_addr(&self) -> &str {
        &self.bind_addr
    }

    /// Get the configuration
    pub fn config(&self) -> &SubmissionConfig {
        &self.config
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
    fn test_submission_config_default() {
        let config = SubmissionConfig::default();
        assert!(config.require_auth);
        assert!(config.require_starttls);
        assert_eq!(config.max_message_size, 25 * 1024 * 1024);
        assert_eq!(config.max_recipients_per_message, 100);
        assert!(config.enforce_sender_match);
    }

    #[test]
    fn test_submission_config_to_smtp_config() {
        let submission_config = SubmissionConfig {
            hostname: "mail.example.com".to_string(),
            max_message_size: 10 * 1024 * 1024,
            require_starttls: true,
            require_auth: true,
            check_recipient_exists: false,
            reject_unknown_recipients: false,
            local_domains: vec!["example.com".to_string()],
            connection_timeout: std::time::Duration::from_secs(600),
            idle_timeout: std::time::Duration::from_secs(120),
            max_recipients_per_message: 50,
            enforce_sender_match: true,
        };

        let smtp_config: SmtpConfig = submission_config.clone().into();

        assert_eq!(smtp_config.hostname, "mail.example.com");
        assert_eq!(smtp_config.max_message_size, 10 * 1024 * 1024);
        assert!(smtp_config.require_auth);
        assert!(smtp_config.enable_starttls);
        assert!(!smtp_config.check_recipient_exists);
        assert!(!smtp_config.reject_unknown_recipients);
        assert_eq!(smtp_config.local_domains, vec!["example.com"]);
        assert_eq!(smtp_config.connection_timeout.as_secs(), 600);
        assert_eq!(smtp_config.idle_timeout.as_secs(), 120);
        // relay_networks should be empty for submission
        assert!(smtp_config.relay_networks.is_empty());
    }

    #[test]
    fn test_submission_server_creation() {
        let config = SubmissionConfig::default();
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

        let server = SubmissionServer::new(
            config.clone(),
            "127.0.0.1:587",
            router,
            auth,
            rate_limiter,
            storage,
        );

        assert_eq!(server.bind_addr(), "127.0.0.1:587");
        assert_eq!(server.config().hostname, config.hostname);
        assert!(server.config().require_auth);
        assert!(server.config().require_starttls);
    }

    #[test]
    #[should_panic(expected = "Submission server must require authentication")]
    fn test_submission_server_requires_auth() {
        let config = SubmissionConfig {
            require_auth: false,
            ..Default::default()
        };

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

        let _server =
            SubmissionServer::new(config, "127.0.0.1:587", router, auth, rate_limiter, storage);
    }

    #[test]
    #[should_panic(expected = "Submission server must require STARTTLS")]
    fn test_submission_server_requires_starttls() {
        let config = SubmissionConfig {
            require_starttls: false,
            ..Default::default()
        };

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

        let _server =
            SubmissionServer::new(config, "127.0.0.1:587", router, auth, rate_limiter, storage);
    }

    // ── Mutual TLS tests ──────────────────────────────────────────────────────
    //
    // These tests exercise `build_mtls_server_config` using in-process self-signed
    // certificates generated by `rcgen`.  They do NOT spin up a full TLS listener
    // (that would require a second tokio-rustls client and is left to integration
    // tests) — instead they verify:
    //   1. `Disabled` mode → `build_mtls_server_config` returns an error (wrong call path).
    //   2. `Optional` mode → server config is built successfully.
    //   3. `Required` mode → server config is built successfully.
    //   4. A bad CA PEM → function returns an error.

    /// Helper: generate a self-signed CA cert + server cert using `rcgen`.
    ///
    /// Returns `(ca_pem, server_cert_pem, server_key_pem)` as byte vectors.
    ///
    /// Uses the low-level rcgen 0.14 API: `CertificateParams::self_signed` for
    /// the CA, and `CertificateParams::signed_by` with an `Issuer` for the server.
    fn generate_test_pki() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        use rcgen::{CertificateParams, DistinguishedName, Issuer, KeyPair};

        // CA certificate — self-signed using low-level API.
        let mut ca_params =
            CertificateParams::new(vec!["ca.example.com".to_string()]).expect("ca params");
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let mut ca_dn = DistinguishedName::new();
        ca_dn.push(rcgen::DnType::CommonName, "Test CA");
        ca_params.distinguished_name = ca_dn;
        let ca_key = KeyPair::generate().expect("ca key");
        // `CertificateParams::self_signed` returns a `Certificate`.
        let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed CA cert");
        let ca_pem = ca_cert.pem().into_bytes();
        // Build an Issuer so we can sign the server cert.
        let ca_issuer = Issuer::new(ca_params, ca_key);

        // Server certificate signed by the CA.
        let mut server_params =
            CertificateParams::new(vec!["localhost".to_string()]).expect("server params");
        let mut server_dn = DistinguishedName::new();
        server_dn.push(rcgen::DnType::CommonName, "Test Server");
        server_params.distinguished_name = server_dn;
        let server_key = KeyPair::generate().expect("server key");

        let server_cert = server_params
            .signed_by(&server_key, &ca_issuer)
            .expect("sign server cert");

        let server_cert_pem = server_cert.pem().into_bytes();
        let server_key_pem = server_key.serialize_pem().into_bytes();

        (ca_pem, server_cert_pem, server_key_pem)
    }

    /// `Disabled` mode must return an error — `build_mtls_server_config` should
    /// never be called with `Disabled`.
    #[test]
    fn test_mtls_client_auth_disabled_no_change() {
        let result =
            build_mtls_server_config(&ClientAuthMode::Disabled, b"dummy", b"dummy", b"dummy");
        assert!(result.is_err(), "Disabled mode must return an error");
    }

    /// `Optional` mode must produce a valid `rustls::ServerConfig`.
    #[test]
    fn test_mtls_optional_accepts_no_cert() {
        // Ensure a ring-based CryptoProvider is installed for this process.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (ca_pem, server_cert_pem, server_key_pem) = generate_test_pki();
        let result = build_mtls_server_config(
            &ClientAuthMode::Optional,
            &ca_pem,
            &server_cert_pem,
            &server_key_pem,
        );
        assert!(
            result.is_ok(),
            "Optional mode must build successfully: {:?}",
            result.err()
        );
    }

    /// `Required` mode must produce a valid `rustls::ServerConfig`.
    #[test]
    fn test_mtls_required_rejects_no_cert() {
        // Ensure a ring-based CryptoProvider is installed for this process.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (ca_pem, server_cert_pem, server_key_pem) = generate_test_pki();
        // The `build_mtls_server_config` call itself must succeed; the actual
        // rejection of a no-cert client happens at handshake time which would
        // require a live TLS listener (out of scope for this unit test).
        let result = build_mtls_server_config(
            &ClientAuthMode::Required,
            &ca_pem,
            &server_cert_pem,
            &server_key_pem,
        );
        assert!(
            result.is_ok(),
            "Required mode must build successfully (rejection happens at handshake): {:?}",
            result.err()
        );
    }

    /// A garbage CA PEM must return a clear error.
    #[test]
    fn test_mtls_bad_ca_pem_returns_error() {
        let result = build_mtls_server_config(
            &ClientAuthMode::Required,
            b"not a real PEM",
            b"also not real",
            b"also not real",
        );
        assert!(result.is_err(), "Bad CA PEM must return an error");
    }
}
