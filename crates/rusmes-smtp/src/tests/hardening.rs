//! Connection-time hardening integration tests (Slice A)
//!
//! Tests verify:
//! 1. Blocked-IP connections are silently dropped (no banner bytes sent).
//! 2. Per-IP connection cap sends `421 4.7.0` on the excess connection.
//! 3. Idle timeout sends `421 4.4.2` and closes cleanly.
//! 4. Session spans carry `session_id` and `peer` fields in tracing records.

use crate::{SmtpConfig, SmtpServer};
use async_trait::async_trait;
use ipnetwork::IpNetwork;
use rusmes_auth::AuthBackend;
use rusmes_core::{MailProcessorRouter, RateLimitConfig, RateLimiter};
use rusmes_metrics::MetricsCollector;
use rusmes_proto::Username;
use rusmes_storage::backends::filesystem::FilesystemBackend;
use rusmes_storage::StorageBackend;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------

/// Minimal auth backend that always accepts any credential.
struct AlwaysOkAuth;

#[async_trait]
impl AuthBackend for AlwaysOkAuth {
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

/// Bind an SMTP server on an OS-assigned ephemeral port and return the bound address.
///
/// Caller is responsible for spawning the `server.run()` future in a task.
async fn bind_test_server(
    config: SmtpConfig,
    rate_limiter: Arc<RateLimiter>,
) -> (SmtpServer, SocketAddr) {
    let tmp = std::env::temp_dir().join(format!(
        "rusmes-smtp-hardentest-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmp).expect("create tempdir");

    let metrics = Arc::new(MetricsCollector::new());
    let router = Arc::new(MailProcessorRouter::new(metrics));
    let auth: Arc<dyn rusmes_auth::AuthBackend> = Arc::new(AlwaysOkAuth);
    let storage: Arc<dyn StorageBackend> = Arc::new(FilesystemBackend::new(&tmp));

    let mut server = SmtpServer::new(
        config,
        "127.0.0.1:0", // OS assigns port
        router,
        auth,
        rate_limiter,
        storage,
    );
    server.bind().await.expect("bind ephemeral port");

    // Retrieve the actual bound address from the listener before serve() consumes it.
    let addr = server.local_addr().expect("listener must be bound");
    (server, addr)
}

// ---------------------------------------------------------------------------
// Test 1: blocked IP — connection dropped, zero bytes received
// ---------------------------------------------------------------------------

/// `test_blocked_ip_rejected` — when the server has `127.0.0.1/32` in its
/// blocked list, a TCP connection from 127.0.0.1 produces zero bytes (silent drop).
#[tokio::test]
async fn test_blocked_ip_rejected() {
    let blocked: IpNetwork = "127.0.0.1/32".parse().expect("valid CIDR");
    let config = SmtpConfig {
        blocked_networks: vec![blocked],
        ..SmtpConfig::default()
    };
    let rate_limiter = Arc::new(RateLimiter::new(RateLimitConfig::default()));
    let (server, addr) = bind_test_server(config, rate_limiter).await;

    let before = rusmes_metrics::global_metrics().smtp_connections_rejected_blocked_count();

    tokio::spawn(async move {
        // serve() loops forever; it will be aborted when the task is dropped.
        let _ = server.serve().await;
    });

    // Give server a moment to enter the accept loop
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(addr).await.expect("TCP connect");
    // Allow the server time to accept and drop
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Try to read — should get EOF (zero bytes) or an error, no banner
    let mut buf = [0u8; 256];
    stream.set_nodelay(true).ok();
    // Set a short read timeout equivalent using tokio::time::timeout
    let result = tokio::time::timeout(Duration::from_millis(300), stream.read(&mut buf)).await;
    match result {
        Ok(Ok(0)) | Err(_) => {
            // Connection closed / timed out — expected: blocked IP gets no banner
        }
        Ok(Ok(n)) => {
            // If we got bytes, fail the test
            let received = std::str::from_utf8(&buf[..n]).unwrap_or("<non-utf8>");
            panic!(
                "Expected zero bytes for blocked IP, got {} bytes: {:?}",
                n, received
            );
        }
        Ok(Err(_)) => {
            // Connection error is also acceptable (server-side RST)
        }
    }

    let after = rusmes_metrics::global_metrics().smtp_connections_rejected_blocked_count();
    assert!(after > before, "blocked counter should have incremented");
}

// ---------------------------------------------------------------------------
// Test 2: max connections per IP — third connection gets 421
// ---------------------------------------------------------------------------

/// `test_max_connections_per_ip` — with `max_connections_per_ip: 2`, opening
/// three TCP connections from the same IP causes the third to receive `421`.
#[tokio::test]
async fn test_max_connections_per_ip() {
    let rate_cfg = RateLimitConfig {
        max_connections_per_ip: 2,
        ..RateLimitConfig::default()
    };
    let config = SmtpConfig::default();
    let rate_limiter = Arc::new(RateLimiter::new(rate_cfg));
    let (server, addr) = bind_test_server(config, rate_limiter).await;

    let before = rusmes_metrics::global_metrics().smtp_connections_rejected_overload_count();

    tokio::spawn(async move {
        let _ = server.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Open first two connections (both should get the 220 banner)
    let mut c1 = TcpStream::connect(addr).await.expect("conn 1");
    let mut c2 = TcpStream::connect(addr).await.expect("conn 2");

    // Read banners from both (to ensure connections are established)
    let mut banner_buf = [0u8; 512];
    let _n1 = tokio::time::timeout(Duration::from_millis(500), c1.read(&mut banner_buf))
        .await
        .ok();
    let _n2 = tokio::time::timeout(Duration::from_millis(500), c2.read(&mut banner_buf))
        .await
        .ok();

    // Open third connection — must receive 421 (the server writes it before dropping).
    // Unlike the blocked-IP case (silent drop), the overload path always writes the
    // 421 banner before closing, so we require the bytes to arrive.
    let mut c3 = TcpStream::connect(addr).await.expect("conn 3");
    let mut rbuf = [0u8; 256];
    let read_result = tokio::time::timeout(Duration::from_millis(1000), c3.read(&mut rbuf)).await;
    let response = match read_result {
        Ok(Ok(n)) if n > 0 => String::from_utf8_lossy(&rbuf[..n]).to_string(),
        Ok(Ok(_)) => String::new(), // EOF before any bytes
        Ok(Err(e)) => panic!("read error on third connection: {}", e),
        Err(_) => panic!("timed out waiting for 421 on third connection; the overload path must write 421 before dropping"),
    };

    assert!(
        response.starts_with("421"),
        "third connection must receive 421 (overload), got: {:?}",
        response
    );

    let after = rusmes_metrics::global_metrics().smtp_connections_rejected_overload_count();
    assert!(after > before, "overload counter should have incremented");

    drop(c1);
    drop(c2);
    drop(c3);
}

// ---------------------------------------------------------------------------
// Test 3: idle timeout sends 421 4.4.2
// ---------------------------------------------------------------------------

/// `test_idle_timeout_421` — with `idle_timeout: 100ms`, a client that
/// connects, sends EHLO, then waits silently for 300ms should receive
/// `421 4.4.2` (idle timeout close).
#[tokio::test]
async fn test_idle_timeout_421() {
    let config = SmtpConfig {
        idle_timeout: Duration::from_millis(150),
        ..SmtpConfig::default()
    };
    let rate_limiter = Arc::new(RateLimiter::new(RateLimitConfig::default()));
    let (server, addr) = bind_test_server(config, rate_limiter).await;

    tokio::spawn(async move {
        let _ = server.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(addr).await.expect("TCP connect");
    let mut buf = [0u8; 512];

    // Read greeting banner
    let n = tokio::time::timeout(Duration::from_millis(1000), stream.read(&mut buf))
        .await
        .expect("timeout reading banner")
        .expect("read banner");
    let banner = String::from_utf8_lossy(&buf[..n]);
    assert!(
        banner.starts_with("220"),
        "expected 220 banner, got: {:?}",
        banner
    );

    // Send EHLO
    stream
        .write_all(b"EHLO test.example.com\r\n")
        .await
        .expect("write EHLO");
    // Read EHLO response
    let n = tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf))
        .await
        .expect("timeout reading EHLO response")
        .expect("read EHLO");
    let ehlo_resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        ehlo_resp.contains("250"),
        "expected 250 EHLO response, got: {:?}",
        ehlo_resp
    );

    // Now do nothing — wait for idle timeout to fire (150ms + margin)
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Read the 421 response
    let n = tokio::time::timeout(Duration::from_millis(600), stream.read(&mut buf))
        .await
        .expect("timeout waiting for 421")
        .expect("read 421");

    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.contains("421"),
        "expected 421 idle timeout response, got: {:?}",
        response
    );
    assert!(
        response.contains("4.4.2"),
        "expected RFC enhanced status 4.4.2 in response, got: {:?}",
        response
    );
}

// ---------------------------------------------------------------------------
// Test 4: session span fields
// ---------------------------------------------------------------------------

/// `test_session_span_present` — verify that tracing events emitted during an
/// SMTP session carry both a `session_id` and a `peer` field (from the span
/// created in `server.rs`).
///
/// We use a custom `tracing::Subscriber` that captures field names from every
/// span entered during the test run. Because global tracing is set for the
/// whole process, we restrict our check to spans whose name is `smtp.session`.
#[tokio::test]
async fn test_session_span_present() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    // Shared state to track whether we saw a smtp.session span with the expected fields.
    static SAW_SESSION_ID: AtomicBool = AtomicBool::new(false);
    static SAW_PEER: AtomicBool = AtomicBool::new(false);
    // Field names captured from smtp.session spans
    static CAPTURED_FIELDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    struct CapturingVisitor;

    impl tracing::field::Visit for CapturingVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {
            let name = field.name();
            if name == "session_id" {
                SAW_SESSION_ID.store(true, Ordering::SeqCst);
            } else if name == "peer" {
                SAW_PEER.store(true, Ordering::SeqCst);
            }
            if let Ok(mut v) = CAPTURED_FIELDS.lock() {
                v.push(name.to_string());
            }
        }
        fn record_str(&mut self, field: &tracing::field::Field, _value: &str) {
            let name = field.name();
            if name == "session_id" {
                SAW_SESSION_ID.store(true, Ordering::SeqCst);
            } else if name == "peer" {
                SAW_PEER.store(true, Ordering::SeqCst);
            }
        }
    }

    struct TestSubscriber;

    impl tracing::Subscriber for TestSubscriber {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            if attrs.metadata().name() == "smtp.session" {
                let mut visitor = CapturingVisitor;
                attrs.record(&mut visitor);
            }
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, values: &tracing::span::Record<'_>) {
            let mut visitor = CapturingVisitor;
            values.record(&mut visitor);
        }
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, _event: &tracing::Event<'_>) {}
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    // `set_default` installs a *thread-local* subscriber.  It only captures
    // tracing events produced on the calling OS thread.  Because `#[tokio::test]`
    // defaults to the `current_thread` runtime, the spawned server task and this
    // test function share the same thread and the same thread-local dispatch, so
    // the subscriber will see the `smtp.session` span created in `server.rs`.
    // If you ever switch to `#[tokio::test(flavor = "multi_thread")]` you must
    // replace this with `tracing::subscriber::with_default` + manual coordination
    // to ensure the server task runs on the same thread, or use a global subscriber
    // (e.g., `set_global_default`) with appropriate test isolation.
    let subscriber = TestSubscriber;
    let _guard = tracing::subscriber::set_default(subscriber);

    let config = SmtpConfig {
        idle_timeout: Duration::from_millis(200),
        ..SmtpConfig::default()
    };
    let rate_limiter = Arc::new(RateLimiter::new(RateLimitConfig::default()));
    let (server, addr) = bind_test_server(config, rate_limiter).await;

    tokio::spawn(async move {
        let _ = server.serve().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect and send EHLO so the session span is created and entered
    let mut stream = TcpStream::connect(addr).await.expect("TCP connect");
    let mut buf = [0u8; 512];
    // Read banner
    let _ = tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf)).await;
    // Send EHLO to trigger span instrumentation
    stream.write_all(b"EHLO test.example.com\r\n").await.ok();
    // Allow span creation to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(stream);

    // Allow session task to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The server creates the span synchronously in serve() before spawning the task
    let captured: Vec<String> = CAPTURED_FIELDS
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default();
    assert!(
        SAW_SESSION_ID.load(Ordering::SeqCst),
        "smtp.session span must carry a `session_id` field; captured fields: {:?}",
        captured
    );
    assert!(
        SAW_PEER.load(Ordering::SeqCst),
        "smtp.session span must carry a `peer` field; captured fields: {:?}",
        captured
    );
}
