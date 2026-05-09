//! End-to-end COMPRESS=DEFLATE handshake tests (RFC 4978).
//!
//! These tests verify that the IMAP session loop correctly negotiates
//! `COMPRESS DEFLATE`, replies with OK, flushes, and then transparently
//! wraps the reader/writer with `RawInflateReader`/`RawDeflateWriter` so
//! that subsequent IMAP traffic is compressed in both directions.
//!
//! # Test approach
//!
//! We drive the `imap_session_loop` via the public
//! `handle_command` + `ImapSession` API without starting a TCP listener.
//! The "network stream" is a `tokio::io::duplex` pair.  After the
//! `COMPRESS DEFLATE` handshake the client side wraps its half with the
//! same oxiarc-deflate adapters.

use oxiarc_deflate::raw_stream::{RawDeflateWriter, RawInflateReader};
use rusmes_imap::command::ImapCommand;
use rusmes_imap::handler::{handle_command, HandlerContext};
use rusmes_imap::session::{ImapSession, ImapState};
use tokio::io::{AsyncBufReadExt, BufReader};

// ── Minimal no-op auth backend ─────────────────────────────────────────────

struct NoopAuth;

#[async_trait::async_trait]
impl rusmes_auth::AuthBackend for NoopAuth {
    async fn authenticate(
        &self,
        _user: &rusmes_proto::Username,
        _password: &str,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn verify_identity(&self, _user: &rusmes_proto::Username) -> anyhow::Result<bool> {
        Ok(false)
    }
    async fn list_users(&self) -> anyhow::Result<Vec<rusmes_proto::Username>> {
        Ok(vec![])
    }
    async fn create_user(
        &self,
        _user: &rusmes_proto::Username,
        _password: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn delete_user(&self, _user: &rusmes_proto::Username) -> anyhow::Result<()> {
        Ok(())
    }
    async fn change_password(
        &self,
        _user: &rusmes_proto::Username,
        _password: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Build a `HandlerContext` backed by an in-memory filesystem backend.
async fn make_ctx(dir: &std::path::Path) -> HandlerContext {
    use rusmes_storage::backends::filesystem::FilesystemBackend;
    use rusmes_storage::StorageBackend;
    use std::sync::Arc;

    let backend = FilesystemBackend::new(dir);
    HandlerContext::new(
        backend.mailbox_store(),
        backend.message_store(),
        backend.metadata_store(),
        Arc::new(NoopAuth),
    )
}

/// Build an authenticated `ImapSession`.
fn make_auth_session(user: &str) -> ImapSession {
    let mut s = ImapSession::new();
    s.username = Some(user.parse().expect("valid username"));
    s.state = ImapState::Authenticated;
    s
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// The handler returns OK for `COMPRESS DEFLATE` and sets `compress_pending`.
#[tokio::test]
async fn test_compress_deflate_handler_returns_ok() {
    let dir = std::env::temp_dir().join(format!(
        "rusmes-imap-e2e-compress-ok-{}",
        uuid::Uuid::new_v4()
    ));
    tokio::fs::create_dir_all(&dir)
        .await
        .expect("create temp dir");

    let ctx = make_ctx(&dir).await;
    let mut session = make_auth_session("alice");

    let resp = handle_command(
        &ctx,
        &mut session,
        "C1",
        ImapCommand::Compress {
            mechanism: "DEFLATE".to_string(),
        },
    )
    .await
    .expect("handle_command");

    let formatted = resp.format();
    assert!(
        formatted.contains("OK"),
        "expected OK response, got: {formatted:?}"
    );
    assert!(
        session.compress_pending,
        "compress_pending should be set after COMPRESS DEFLATE"
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

/// `COMPRESS` with an unsupported mechanism returns NO.
#[tokio::test]
async fn test_compress_unknown_mechanism_returns_no() {
    let dir = std::env::temp_dir().join(format!(
        "rusmes-imap-e2e-compress-no-{}",
        uuid::Uuid::new_v4()
    ));
    tokio::fs::create_dir_all(&dir)
        .await
        .expect("create temp dir");

    let ctx = make_ctx(&dir).await;
    let mut session = make_auth_session("bob");

    let resp = handle_command(
        &ctx,
        &mut session,
        "C2",
        ImapCommand::Compress {
            mechanism: "LZ4".to_string(),
        },
    )
    .await
    .expect("handle_command");

    let formatted = resp.format();
    assert!(
        formatted.contains("NO"),
        "expected NO for unknown mechanism, got: {formatted:?}"
    );
    assert!(
        !session.compress_pending,
        "compress_pending must NOT be set for unknown mechanism"
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

/// A second `COMPRESS DEFLATE` on an already-compressed session returns NO.
#[tokio::test]
async fn test_compress_double_returns_no() {
    let dir = std::env::temp_dir().join(format!(
        "rusmes-imap-e2e-compress-double-{}",
        uuid::Uuid::new_v4()
    ));
    tokio::fs::create_dir_all(&dir)
        .await
        .expect("create temp dir");

    let ctx = make_ctx(&dir).await;
    let mut session = make_auth_session("carol");

    // First COMPRESS DEFLATE — should succeed.
    let resp1 = handle_command(
        &ctx,
        &mut session,
        "C3",
        ImapCommand::Compress {
            mechanism: "DEFLATE".to_string(),
        },
    )
    .await
    .expect("first compress");
    assert!(resp1.format().contains("OK"), "first COMPRESS must be OK");

    // `compress_pending` is set; simulate the server loop clearing it after the
    // stream swap so the session stays in the "already compressed" state.
    // We use `compress_pending = true` (not cleared) to mimic "already active":
    // the second call should see `compress_pending == true` and return NO.
    let resp2 = handle_command(
        &ctx,
        &mut session,
        "C4",
        ImapCommand::Compress {
            mechanism: "DEFLATE".to_string(),
        },
    )
    .await
    .expect("second compress");
    assert!(
        resp2.format().contains("NO"),
        "second COMPRESS must be NO (already active), got: {:?}",
        resp2.format()
    );

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

/// After the COMPRESS handshake, wrap a duplex with the oxiarc-deflate
/// adapters and verify a CAPABILITY command roundtrips through them
/// correctly (IMAP line preserved after compress → decompress).
///
/// This test does NOT exercise the full server loop (which requires
/// spawning a real TCP listener and IMAP parser), but it validates
/// the transport layer that `imap_session_loop` switches to.
#[tokio::test]
async fn test_compress_deflate_transport_roundtrip_via_duplex() {
    use tokio::io::AsyncWriteExt;

    // Simulate a post-handshake compressed stream using a duplex.
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);

    // "Server side": writes compressed IMAP response lines.
    let server_task = tokio::spawn(async move {
        let mut writer = RawDeflateWriter::new(server_half, 6);
        let lines: [&[u8]; 3] = [
            b"* OK [CAPABILITY IMAP4rev1 COMPRESS=DEFLATE] ready\r\n",
            b"A001 OK [COMPRESSIONACTIVE] Begin DEFLATE compression\r\n",
            b"* 5 EXISTS\r\n",
        ];
        for line in &lines {
            writer.write_all(line).await.expect("server write");
            writer.flush().await.expect("server flush");
        }
        // shutdown signals EOF to the reader
        writer.shutdown().await.expect("server shutdown");
    });

    // "Client side": reads + decompresses.
    let mut reader = BufReader::new(RawInflateReader::new(client_half));
    let mut received_lines: Vec<String> = Vec::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await.expect("client read_line");
        if n == 0 {
            break; // EOF
        }
        received_lines.push(line.trim_end_matches("\r\n").to_string());
    }

    server_task.await.expect("server task");

    assert_eq!(received_lines.len(), 3, "expected 3 response lines");
    assert!(
        received_lines[0].contains("CAPABILITY"),
        "line 0: {:?}",
        received_lines[0]
    );
    assert!(
        received_lines[1].contains("COMPRESSIONACTIVE"),
        "line 1: {:?}",
        received_lines[1]
    );
    assert!(
        received_lines[2].contains("EXISTS"),
        "line 2: {:?}",
        received_lines[2]
    );
}
