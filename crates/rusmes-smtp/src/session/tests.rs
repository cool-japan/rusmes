use super::*;
use async_trait::async_trait;
use rusmes_core::{MailProcessorRouter, RateLimitConfig, RateLimiter};
use rusmes_metrics::MetricsCollector;
use rusmes_storage::backends::filesystem::FilesystemBackend;

/// Auth backend that always reports `Ok(None)` for SCRAM lookups so we can
/// exercise the "mechanism not available" branch of the SMTP SCRAM handler.
struct ScramMissingBackend;

#[async_trait]
impl AuthBackend for ScramMissingBackend {
    async fn authenticate(&self, _username: &Username, _password: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn verify_identity(&self, _username: &Username) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
        Ok(Vec::new())
    }

    async fn create_user(&self, _username: &Username, _password: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn delete_user(&self, _username: &Username) -> anyhow::Result<()> {
        Ok(())
    }

    async fn change_password(
        &self,
        _username: &Username,
        _new_password: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    // Inherits the trait default: `fetch_scram_credentials -> Ok(None)`.
}

fn make_session(
    auth_backend: Arc<dyn AuthBackend>,
    storage_backend: Arc<dyn StorageBackend>,
) -> SmtpSession {
    let metrics = Arc::new(MetricsCollector::new());
    let processor_router = Arc::new(MailProcessorRouter::new(metrics));
    let rate_limiter = Arc::new(RateLimiter::new(RateLimitConfig::default()));
    let remote_addr: SocketAddr = "127.0.0.1:54321"
        .parse()
        .expect("static socket addr literal must parse");
    SmtpSession {
        remote_addr,
        state: SmtpState::Authenticated,
        transaction: SmtpTransaction::new(),
        config: SmtpConfig {
            require_auth: true,
            ..SmtpConfig::default()
        },
        authenticated: false,
        username: None,
        relaying_allowed: false,
        processor_router,
        auth_backend,
        rate_limiter,
        storage_backend,
        recipient_cache: Arc::new(RwLock::new(HashMap::new())),
        cram_md5_challenge: None,
        scram_state: None,
        ehlo_used: false,
        peer_certificates: None,
    }
}

/// Cluster 1D: when the auth backend reports `Ok(None)` for a user's SCRAM
/// credentials, the SMTP server must respond `504 5.5.4 ... mechanism not
/// available` per RFC 4954 §4 and decline the SCRAM exchange (PLAIN/LOGIN
/// remain available to the client).
#[tokio::test]
async fn scram_rejected_when_credentials_missing() {
    let tmp = std::env::temp_dir().join(format!("rusmes-smtp-scram-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir for filesystem backend");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));
    let auth: Arc<dyn AuthBackend> = Arc::new(ScramMissingBackend);

    let mut session = make_session(auth, storage);

    // Build a valid SCRAM client-first message: GS2 header `n,,` + n=user,r=nonce.
    // The server should fetch credentials, get `Ok(None)`, and respond 504.
    let client_first = "n,,n=alice,r=fyko+d2lbbFgONRv9qkxdawL";
    let initial = BASE64.encode(client_first.as_bytes());

    let response = session
        .handle_auth_scram_sha256(Some(initial))
        .await
        .expect("handle_auth_scram_sha256 must not error on Ok(None)");

    assert_eq!(
        response.code(),
        504,
        "missing SCRAM credentials must yield 504 (mechanism not available)"
    );
    assert!(
        !session.authenticated,
        "session must not be marked authenticated when SCRAM is declined"
    );
    assert!(
        session.scram_state.is_none(),
        "no SCRAM state should be retained after a 504 reply"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Metrics counter tests ──────────────────────────────────────────────────
//
// These tests drive protocol-layer methods directly and assert on *delta* values
// so they are safe under parallel nextest workers that share the global singleton.

/// `test_smtp_connection_counter_increments` — the MetricsCollector counter
/// `smtp_connections_total` increments by 1 when `inc_smtp_connections()` is called.
/// This mirrors what `SmtpSessionHandler::handle()` does at session start.
#[test]
fn test_smtp_connection_counter_increments() {
    let m = MetricsCollector::new();
    assert_eq!(m.smtp_connections_count(), 0);
    m.inc_smtp_connections();
    assert_eq!(m.smtp_connections_count(), 1);
    m.inc_smtp_connections();
    assert_eq!(m.smtp_connections_count(), 2);
}

/// `test_smtp_auth_success_counter` — a successful PLAIN AUTH increments
/// `smtp_auth_success_total` by 1 (delta check on global metrics).
#[tokio::test]
async fn test_smtp_auth_success_counter() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-auth-ok-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));

    // AlwaysOkBackend: authenticate always returns Ok(true).
    struct AlwaysOkBackend;
    #[async_trait]
    impl AuthBackend for AlwaysOkBackend {
        async fn authenticate(&self, _u: &Username, _p: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn verify_identity(&self, _u: &Username) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
            Ok(vec![])
        }
        async fn create_user(&self, _u: &Username, _p: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_user(&self, _u: &Username) -> anyhow::Result<()> {
            Ok(())
        }
        async fn change_password(&self, _u: &Username, _p: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    let auth: Arc<dyn AuthBackend> = Arc::new(AlwaysOkBackend);
    let mut session = make_session(auth, storage);

    let before = rusmes_metrics::global_metrics().smtp_auth_success_count();
    // Build a valid PLAIN credential: \0username\0password (base64-encoded).
    let plain = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        b"\0testuser\0testpass",
    );
    let resp = session
        .handle_auth_plain(plain)
        .await
        .expect("handle_auth_plain must not error");
    assert_eq!(resp.code(), 235, "expected 235 Authentication successful");
    let after = rusmes_metrics::global_metrics().smtp_auth_success_count();
    assert_eq!(
        after - before,
        1,
        "smtp_auth_success_total should increment by 1 on successful PLAIN auth"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// `test_smtp_auth_failure_counter` — a rejected PLAIN AUTH increments
/// `smtp_auth_failure_total` by 1 (delta check on global metrics).
#[tokio::test]
async fn test_smtp_auth_failure_counter() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-auth-fail-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));

    // AlwaysFailBackend: authenticate always returns Ok(false).
    struct AlwaysFailBackend;
    #[async_trait]
    impl AuthBackend for AlwaysFailBackend {
        async fn authenticate(&self, _u: &Username, _p: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn verify_identity(&self, _u: &Username) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
            Ok(vec![])
        }
        async fn create_user(&self, _u: &Username, _p: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_user(&self, _u: &Username) -> anyhow::Result<()> {
            Ok(())
        }
        async fn change_password(&self, _u: &Username, _p: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    let auth: Arc<dyn AuthBackend> = Arc::new(AlwaysFailBackend);
    let mut session = make_session(auth, storage);

    let before = rusmes_metrics::global_metrics().smtp_auth_failure_count();
    let plain = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        b"\0wronguser\0wrongpass",
    );
    let resp = session
        .handle_auth_plain(plain)
        .await
        .expect("handle_auth_plain must not error");
    assert_eq!(resp.code(), 535, "expected 535 Authentication failed");
    let after = rusmes_metrics::global_metrics().smtp_auth_failure_count();
    assert_eq!(
        after - before,
        1,
        "smtp_auth_failure_total should increment by 1 on failed PLAIN auth"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// `test_smtp_message_accepted_counter` — completing a DATA transaction
/// increments `smtp_messages_received` (the "accepted" counter) by 1.
///
/// We drive `handle_data_input` directly with an in-memory reader/writer so we
/// don't need a full TCP stack.
#[tokio::test]
async fn test_smtp_message_accepted_counter() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-msg-ok-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));
    let auth: Arc<dyn AuthBackend> = Arc::new(ScramMissingBackend);
    let mut session = make_session(auth, storage);

    // Set up a valid transaction (sender + recipient).
    session.transaction.sender = Some("sender@example.com".parse().expect("valid sender address"));
    session
        .transaction
        .recipients
        .push("rcpt@example.com".parse().expect("valid recipient address"));

    // Craft DATA stream: header + blank line + body + terminator dot.
    // `&[u8]` implements `AsyncRead` so `BufReader<&[u8]>` works directly.
    let data_stream: &[u8] = b"From: sender@example.com\r\nSubject: test\r\n\r\nHello\r\n.\r\n";
    let mut async_reader = tokio::io::BufReader::new(data_stream);

    let mut writer_buf: Vec<u8> = Vec::new();
    let remote_addr: SocketAddr = "127.0.0.1:54321"
        .parse()
        .expect("static socket addr literal must parse");

    let before = rusmes_metrics::global_metrics().smtp_messages_accepted_count();

    SmtpSessionHandler::handle_data_input(
        &mut session,
        &mut async_reader,
        &mut writer_buf,
        &remote_addr,
    )
    .await
    .expect("handle_data_input must succeed");

    let after = rusmes_metrics::global_metrics().smtp_messages_accepted_count();
    assert_eq!(
        after - before,
        1,
        "smtp_messages_received should increment by 1 on DATA acceptance"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_transaction_validity() {
    let mut tx = SmtpTransaction::new();
    assert!(!tx.is_valid());

    tx.sender = Some(
        "sender@example.com"
            .parse()
            .expect("valid email address literal"),
    );
    assert!(!tx.is_valid());

    tx.recipients.push(
        "rcpt@example.com"
            .parse()
            .expect("valid email address literal"),
    );
    assert!(tx.is_valid());

    tx.reset();
    assert!(!tx.is_valid());
}

#[test]
fn test_smtp_config_default() {
    let config = SmtpConfig::default();
    assert_eq!(config.hostname, "localhost");
    assert_eq!(config.max_message_size, 10 * 1024 * 1024);
    assert!(!config.require_auth);
    assert!(!config.enable_starttls);
}

// ── SMTPUTF8 / RFC 6531 session-layer tests ────────────────────────────

/// The EHLO capability list must include `SMTPUTF8`.
#[tokio::test]
async fn test_ehlo_advertises_smtputf8() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-ehlo-smtputf8-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));
    let auth: Arc<dyn AuthBackend> = Arc::new(ScramMissingBackend);
    let mut session = make_session(auth, storage);

    let resp = session
        .handle_ehlo("client.example.com".to_string())
        .await
        .expect("handle_ehlo must not error");

    let formatted = resp.format();
    assert!(
        formatted.contains("SMTPUTF8"),
        "EHLO response must advertise SMTPUTF8; got:\n{}",
        formatted
    );
    assert!(session.ehlo_used, "ehlo_used flag must be set after EHLO");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// After HELO (not EHLO), the SMTPUTF8 mail parameter must be rejected.
#[tokio::test]
async fn test_smtputf8_requires_ehlo_not_helo() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-smtputf8-helo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));
    let auth: Arc<dyn AuthBackend> = Arc::new(ScramMissingBackend);
    let mut session = make_session(auth, storage);

    // HELO — no ESMTP extensions.
    session
        .handle_helo("client.example.com".to_string())
        .await
        .expect("handle_helo must not error");
    assert!(!session.ehlo_used, "ehlo_used must be false after HELO");

    // MAIL FROM with SMTPUTF8 parameter — must be rejected.
    let from = "sender@example.com"
        .parse::<MailAddress>()
        .expect("valid address");
    let params = vec![crate::command::MailParam::new("SMTPUTF8".to_string(), None)];

    let resp = session
        .handle_mail(from, params)
        .await
        .expect("handle_mail must not error internally");

    assert!(
        resp.code() >= 500,
        "SMTPUTF8 after HELO must yield a 5xx error; got {}",
        resp.code()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// A non-ASCII local-part sent without the SMTPUTF8 parameter must be
/// rejected with 501 5.5.4 even after EHLO (RFC 6531 §3.4).
#[tokio::test]
async fn test_smtputf8_param_required_for_unicode_address() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-smtputf8-param-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));
    let auth: Arc<dyn AuthBackend> = Arc::new(ScramMissingBackend);
    let mut session = make_session(auth, storage);
    // Mark as authenticated so handle_mail's require_auth guard is satisfied.
    session.authenticated = true;

    // EHLO first — ESMTP extensions available.
    session
        .handle_ehlo("client.example.com".to_string())
        .await
        .expect("handle_ehlo must not error");

    // Construct a non-ASCII address via the SMTPUTF8 constructor,
    // then pass it WITHOUT the SMTPUTF8 parameter in MAIL FROM.
    let domain = rusmes_proto::Domain::new("example.com").expect("valid domain");
    let from = MailAddress::new_smtputf8("münchen", domain)
        .expect("SMTPUTF8 address must be constructable");

    // No SMTPUTF8 parameter — should trigger 501 5.5.4.
    let resp = session
        .handle_mail(from, vec![])
        .await
        .expect("handle_mail must not error internally");

    assert_eq!(
        resp.code(),
        501,
        "Non-ASCII address without SMTPUTF8 param must yield 501; got {}",
        resp.code()
    );
    assert!(
        resp.format().contains("5.5.4"),
        "Response must contain enhanced status 5.5.4; got:\n{}",
        resp.format()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── DATA tempfile spill threshold tests ───────────────────────────────────

/// Collect the set of file paths currently present in `dir`.
///
/// Only entries that are plain files (not directories) are included.  Errors
/// reading individual entries are silently ignored so that transient system
/// tempfiles do not derail the test.
fn snapshot_dir_files(dir: &std::path::Path) -> std::collections::HashSet<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|entry| {
                let entry = entry.ok()?;
                let ft = entry.file_type().ok()?;
                if ft.is_file() {
                    Some(entry.path())
                } else {
                    None
                }
            })
            .collect()
        })
        .unwrap_or_default()
}

/// Below the threshold, DATA should stay in memory (MessageBody::Small).
///
/// Verified using an isolated `data_spill_dir`: no files appear in that
/// directory when the in-memory path is taken.
#[tokio::test]
async fn test_data_input_stays_in_memory_below_threshold() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-data-mem-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    // Dedicated spill dir — isolated from other concurrent tests.
    let spill_dir = tmp.join("spill");
    std::fs::create_dir_all(&spill_dir).expect("create spill dir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));
    let auth: Arc<dyn AuthBackend> = Arc::new(ScramMissingBackend);
    let mut session = make_session(auth, storage);

    // Set threshold to 1 MiB; payload is 64 KiB, well below threshold.
    session.config.data_tempfile_threshold = 1024 * 1024;
    session.config.data_spill_dir = spill_dir.clone();

    session.transaction.sender = Some("sender@example.com".parse().expect("valid sender address"));
    session
        .transaction
        .recipients
        .push("rcpt@example.com".parse().expect("valid recipient address"));

    // Build a 64 KiB body (line-based, dot-terminated).
    let body_line = "X".repeat(78) + "\r\n"; // 80 bytes per line
    let line_count = (64 * 1024) / 80; // ~819 lines for ~64 KiB
    let mut data = String::from("From: sender@example.com\r\nSubject: mem-test\r\n\r\n");
    for _ in 0..line_count {
        data.push_str(&body_line);
    }
    data.push_str(".\r\n");

    let data_bytes = data.into_bytes();
    let mut async_reader = tokio::io::BufReader::new(data_bytes.as_slice());
    let mut writer_buf: Vec<u8> = Vec::new();
    let remote_addr: SocketAddr = "127.0.0.1:54321"
        .parse()
        .expect("static socket addr literal must parse");

    SmtpSessionHandler::handle_data_input(
        &mut session,
        &mut async_reader,
        &mut writer_buf,
        &remote_addr,
    )
    .await
    .expect("handle_data_input must succeed for small payload");

    // Verify the response is 250 OK (message accepted).
    let response_str = String::from_utf8_lossy(&writer_buf);
    assert!(
        response_str.contains("250"),
        "Expected 250 OK response for small payload; got: {}",
        response_str
    );

    // In-memory path must not create any files in the isolated spill dir.
    let spill_files = snapshot_dir_files(&spill_dir);
    assert!(
        spill_files.is_empty(),
        "In-memory path must not create tempfiles in spill_dir; found: {:?}",
        spill_files
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Above the threshold, DATA should spill to a tempfile (MessageBody::Large).
///
/// Verified using an isolated `data_spill_dir` in two stages:
/// 1. Immediately after `handle_data_input` returns, at least one file exists
///    in the isolated spill dir (the spill file was created and kept).
/// 2. After yielding briefly so the spawned cleanup task can run, the spill
///    file is deleted (the isolated dir is empty again).
#[tokio::test]
async fn test_data_input_spills_above_threshold() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-data-spill-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    // Dedicated spill dir — isolated from other concurrent tests.
    let spill_dir = tmp.join("spill");
    std::fs::create_dir_all(&spill_dir).expect("create spill dir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));
    let auth: Arc<dyn AuthBackend> = Arc::new(ScramMissingBackend);
    let mut session = make_session(auth, storage);

    // Set threshold to 64 KiB; payload is ~2 MiB, triggering spill.
    session.config.data_tempfile_threshold = 64 * 1024;
    session.config.data_spill_dir = spill_dir.clone();
    // Raise max_message_size so the 2 MiB message is accepted.
    session.config.max_message_size = 10 * 1024 * 1024;

    session.transaction.sender = Some("sender@example.com".parse().expect("valid sender address"));
    session
        .transaction
        .recipients
        .push("rcpt@example.com".parse().expect("valid recipient address"));

    // Build a ~2 MiB body.
    let body_line = "Y".repeat(78) + "\r\n"; // 80 bytes per line
    let line_count = (2 * 1024 * 1024) / 80; // ~26214 lines for ~2 MiB
    let mut data = String::from("From: sender@example.com\r\nSubject: spill-test\r\n\r\n");
    for _ in 0..line_count {
        data.push_str(&body_line);
    }
    data.push_str(".\r\n");

    let data_bytes = data.into_bytes();
    let mut async_reader = tokio::io::BufReader::new(data_bytes.as_slice());
    let mut writer_buf: Vec<u8> = Vec::new();
    let remote_addr: SocketAddr = "127.0.0.1:54321"
        .parse()
        .expect("static socket addr literal must parse");

    SmtpSessionHandler::handle_data_input(
        &mut session,
        &mut async_reader,
        &mut writer_buf,
        &remote_addr,
    )
    .await
    .expect("handle_data_input must succeed for large payload");

    // Verify the response is 250 OK.
    let response_str = String::from_utf8_lossy(&writer_buf);
    assert!(
        response_str.contains("250"),
        "Expected 250 OK response for large payload; got: {}",
        response_str
    );

    // Immediately after the call, the spill file must exist in the isolated dir.
    let spill_files_after_call = snapshot_dir_files(&spill_dir);
    assert!(
        !spill_files_after_call.is_empty(),
        "Spill path must have created at least one file in spill_dir immediately after the call"
    );

    // Yield to the Tokio runtime so the spawned cleanup task gets a chance to run
    // and delete the spill file.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // After cleanup the spill dir must be empty.
    let spill_files_after_cleanup = snapshot_dir_files(&spill_dir);
    assert!(
        spill_files_after_cleanup.is_empty(),
        "Spill tempfile must be deleted by the cleanup task; still present: {:?}",
        spill_files_after_cleanup
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Exactly at the threshold (not one byte over), message stays in memory.
///
/// Verified using an isolated `data_spill_dir`: at exactly threshold bytes,
/// the strict `>` condition in the spill logic is not triggered, so the
/// isolated spill directory remains empty.
#[tokio::test]
async fn test_data_input_threshold_boundary() {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-data-boundary-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    // Dedicated spill dir — isolated from other concurrent tests.
    let spill_dir = tmp.join("spill");
    std::fs::create_dir_all(&spill_dir).expect("create spill dir");

    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));
    let auth: Arc<dyn AuthBackend> = Arc::new(ScramMissingBackend);
    let mut session = make_session(auth, storage);

    // Threshold: 1024 bytes. Payload: exactly 1024 bytes of body.
    // The strict `>` semantics mean at exactly threshold, it stays in memory.
    let threshold = 1024usize;
    session.config.data_tempfile_threshold = threshold;
    session.config.data_spill_dir = spill_dir.clone();
    session.config.max_message_size = 10 * 1024 * 1024;

    session.transaction.sender = Some("sender@example.com".parse().expect("valid sender address"));
    session
        .transaction
        .recipients
        .push("rcpt@example.com".parse().expect("valid recipient address"));

    // Build header; then fill body to reach threshold exactly.
    let header = "From: sender@example.com\r\nSubject: boundary\r\n\r\n";
    // body_line is 80 bytes (78 'Z' + \r\n), we add lines until we're at/near threshold.
    let body_line = "Z".repeat(78) + "\r\n";
    let lines_needed = threshold / 80; // bytes in full lines up to threshold
    let mut data = String::from(header);
    for _ in 0..lines_needed {
        data.push_str(&body_line);
    }
    data.push_str(".\r\n");

    let data_bytes = data.into_bytes();
    let mut async_reader = tokio::io::BufReader::new(data_bytes.as_slice());
    let mut writer_buf: Vec<u8> = Vec::new();
    let remote_addr: SocketAddr = "127.0.0.1:54321"
        .parse()
        .expect("static socket addr literal must parse");

    SmtpSessionHandler::handle_data_input(
        &mut session,
        &mut async_reader,
        &mut writer_buf,
        &remote_addr,
    )
    .await
    .expect("handle_data_input must succeed at threshold boundary");

    let response_str = String::from_utf8_lossy(&writer_buf);
    assert!(
        response_str.contains("250"),
        "Expected 250 OK at threshold boundary; got: {}",
        response_str
    );

    // At exactly threshold bytes, the isolated spill dir must remain empty.
    let spill_files = snapshot_dir_files(&spill_dir);
    assert!(
        spill_files.is_empty(),
        "At-threshold boundary must stay in memory (no files in spill_dir); found: {:?}",
        spill_files
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
