//! Integration tests for POP3 maildrop locking (RFC 1939) and SASL AUTH (RFC 1734).
//!
//! These tests exercise the protocol-level behaviour end-to-end via
//! `Pop3Session::handle_line`-equivalent paths through the public API of the
//! crate. They use:
//! - a tempdir-backed `FilesystemBackend` from `rusmes-storage`
//! - a tempdir-backed `FileAuthBackend` from `rusmes-auth`
//! - the public `MaildropLockManager` from `rusmes-pop3`

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusmes_auth::file::FileAuthBackend;
use rusmes_auth::AuthBackend;
use rusmes_pop3::{MaildropLockManager, Pop3Config, Pop3Response, Pop3Session, Pop3Status};
use rusmes_proto::Username;
use rusmes_storage::backends::filesystem::FilesystemBackend;
use rusmes_storage::{MailboxPath, StorageBackend};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::tempdir;

// -----------------------------------------------------------------------------
// Test fixtures
// -----------------------------------------------------------------------------

/// Build a `FileAuthBackend` populated with a single user `alice` whose
/// password is `s3cret`.  All file I/O lives under `dir`.
async fn make_auth_backend(dir: &std::path::Path) -> Arc<FileAuthBackend> {
    let passwd = dir.join("passwd");
    let backend = FileAuthBackend::new(&passwd)
        .await
        .expect("FileAuthBackend::new");
    let user = Username::new("alice").expect("valid username");
    backend
        .create_user(&user, "s3cret")
        .await
        .expect("create_user");

    // Sanity: the credentials really do verify.
    assert!(
        backend
            .authenticate(&user, "s3cret")
            .await
            .expect("authenticate"),
        "fixture user credentials must verify"
    );
    Arc::new(backend)
}

/// Build a `FilesystemBackend` rooted under `dir`, with an empty INBOX
/// pre-created for `alice`.
async fn make_storage_backend(dir: &std::path::Path) -> Arc<FilesystemBackend> {
    let backend = Arc::new(FilesystemBackend::new(dir));
    let user = Username::new("alice").expect("valid username");
    let inbox_path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    let mailboxes = backend.mailbox_store();
    mailboxes
        .create_mailbox(&inbox_path)
        .await
        .expect("create alice INBOX");
    backend
}

/// Build a default Pop3Session for `alice` using the supplied backends and
/// shared maildrop-lock manager.
fn make_session(
    auth: Arc<FileAuthBackend>,
    storage: Arc<FilesystemBackend>,
    locks: MaildropLockManager,
) -> Pop3Session {
    let addr: SocketAddr = "127.0.0.1:1110".parse().expect("test SocketAddr parse");
    let config = Pop3Config::default();
    Pop3Session::new(addr, config, auth, storage, locks)
}

// Helper: parse a wire-format POP3 response by inspecting status + body.
fn assert_ok(resp: &Pop3Response, msg_part: &str) {
    assert_eq!(
        resp.status(),
        Pop3Status::Ok,
        "expected +OK, got -ERR/{:?}: {}",
        resp.status(),
        resp.message()
    );
    assert!(
        resp.message().contains(msg_part),
        "expected response to contain {:?}, got {:?}",
        msg_part,
        resp.message()
    );
}

fn assert_err(resp: &Pop3Response, msg_part: &str) {
    assert_eq!(
        resp.status(),
        Pop3Status::Err,
        "expected -ERR, got {:?}: {}",
        resp.status(),
        resp.message()
    );
    assert!(
        resp.message()
            .to_lowercase()
            .contains(&msg_part.to_lowercase()),
        "expected error to contain {:?}, got {:?}",
        msg_part,
        resp.message()
    );
}

// -----------------------------------------------------------------------------
// Maildrop locking — RFC 1939 §3
// -----------------------------------------------------------------------------

/// Two concurrent USER/PASS sessions for `alice` must not both reach
/// Transaction state. The second session must receive `-ERR maildrop locked`.
#[tokio::test]
async fn pop3_maildrop_lock_blocks_second_session_for_same_user() {
    let auth_dir = tempdir().expect("auth tempdir");
    let storage_dir = tempdir().expect("storage tempdir");
    let auth = make_auth_backend(auth_dir.path()).await;
    let storage = make_storage_backend(storage_dir.path()).await;
    let locks = MaildropLockManager::new();

    // Session 1: USER + PASS — should succeed and acquire the maildrop lock.
    let mut s1 = make_session(Arc::clone(&auth), Arc::clone(&storage), locks.clone());
    let resp = s1.handle_line_for_test("USER alice\r\n").await;
    assert_ok(&resp, "alice");
    let resp = s1.handle_line_for_test("PASS s3cret\r\n").await;
    assert_ok(&resp, "messages");

    // Session 2 (concurrent): USER + PASS for the *same* user must be rejected.
    let mut s2 = make_session(Arc::clone(&auth), Arc::clone(&storage), locks.clone());
    let resp = s2.handle_line_for_test("USER alice\r\n").await;
    assert_ok(&resp, "alice");
    let resp = s2.handle_line_for_test("PASS s3cret\r\n").await;
    assert_err(&resp, "maildrop locked");
}

/// After the first session releases the lock (QUIT → drop), a fresh session
/// for the same user must be able to authenticate.
#[tokio::test]
async fn pop3_maildrop_lock_released_when_first_session_drops() {
    let auth_dir = tempdir().expect("auth tempdir");
    let storage_dir = tempdir().expect("storage tempdir");
    let auth = make_auth_backend(auth_dir.path()).await;
    let storage = make_storage_backend(storage_dir.path()).await;
    let locks = MaildropLockManager::new();

    {
        let mut s1 = make_session(Arc::clone(&auth), Arc::clone(&storage), locks.clone());
        let _ = s1.handle_line_for_test("USER alice\r\n").await;
        let resp = s1.handle_line_for_test("PASS s3cret\r\n").await;
        assert_ok(&resp, "messages");
        // `s1` goes out of scope here — Drop on its `MaildropGuard` releases
        // the per-user mutex.
    }

    let mut s2 = make_session(Arc::clone(&auth), Arc::clone(&storage), locks.clone());
    let _ = s2.handle_line_for_test("USER alice\r\n").await;
    let resp = s2.handle_line_for_test("PASS s3cret\r\n").await;
    assert_ok(&resp, "messages");
}

// -----------------------------------------------------------------------------
// SASL AUTH — RFC 1734 + RFC 5034
// -----------------------------------------------------------------------------

/// Full single-shot AUTH PLAIN exchange with an initial response. The server
/// must respond `+OK ...` and the session must transition to Transaction.
#[tokio::test]
async fn pop3_sasl_plain_with_initial_response_succeeds() {
    let auth_dir = tempdir().expect("auth tempdir");
    let storage_dir = tempdir().expect("storage tempdir");
    let auth = make_auth_backend(auth_dir.path()).await;
    let storage = make_storage_backend(storage_dir.path()).await;
    let locks = MaildropLockManager::new();

    let mut s = make_session(auth, storage, locks);

    // PLAIN format per RFC 4616: \0authcid\0password
    let plaintext = b"\0alice\0s3cret";
    let ir = BASE64.encode(plaintext);

    let resp = s
        .handle_line_for_test(&format!("AUTH PLAIN {}\r\n", ir))
        .await;
    assert_ok(&resp, "messages");
}

/// Two-step AUTH PLAIN exchange (no initial response): server sends `+ ` then
/// receives the credentials on the next line.
#[tokio::test]
async fn pop3_sasl_plain_two_step_succeeds() {
    let auth_dir = tempdir().expect("auth tempdir");
    let storage_dir = tempdir().expect("storage tempdir");
    let auth = make_auth_backend(auth_dir.path()).await;
    let storage = make_storage_backend(storage_dir.path()).await;
    let locks = MaildropLockManager::new();

    let mut s = make_session(auth, storage, locks);

    // Step 1: the server should respond with a bare continuation `+ `.
    let resp = s.handle_line_for_test("AUTH PLAIN\r\n").await;
    assert_eq!(
        resp.status(),
        Pop3Status::Continue,
        "server must reply with continuation; got {:?}",
        resp.status()
    );

    // Step 2: client sends credentials base64-encoded.
    let plaintext = b"\0alice\0s3cret";
    let ir = BASE64.encode(plaintext);
    let resp = s.handle_line_for_test(&format!("{}\r\n", ir)).await;
    assert_ok(&resp, "messages");
}

/// AUTH PLAIN with the wrong password must return -ERR.
#[tokio::test]
async fn pop3_sasl_plain_rejects_bad_password() {
    let auth_dir = tempdir().expect("auth tempdir");
    let storage_dir = tempdir().expect("storage tempdir");
    let auth = make_auth_backend(auth_dir.path()).await;
    let storage = make_storage_backend(storage_dir.path()).await;
    let locks = MaildropLockManager::new();

    let mut s = make_session(auth, storage, locks);

    let plaintext = b"\0alice\0wrongpass";
    let ir = BASE64.encode(plaintext);

    let resp = s
        .handle_line_for_test(&format!("AUTH PLAIN {}\r\n", ir))
        .await;
    assert_err(&resp, "Authentication failed");
}

/// AUTH with an unknown mechanism must return -ERR.
#[tokio::test]
async fn pop3_sasl_unknown_mechanism_returns_err() {
    let auth_dir = tempdir().expect("auth tempdir");
    let storage_dir = tempdir().expect("storage tempdir");
    let auth = make_auth_backend(auth_dir.path()).await;
    let storage = make_storage_backend(storage_dir.path()).await;
    let locks = MaildropLockManager::new();

    let mut s = make_session(auth, storage, locks);

    let resp = s.handle_line_for_test("AUTH MAGIC-MECHANISM\r\n").await;
    assert_err(&resp, "not supported");
}

/// During an in-flight SASL exchange, the literal token `*` cancels.
#[tokio::test]
async fn pop3_sasl_client_abort_returns_err() {
    let auth_dir = tempdir().expect("auth tempdir");
    let storage_dir = tempdir().expect("storage tempdir");
    let auth = make_auth_backend(auth_dir.path()).await;
    let storage = make_storage_backend(storage_dir.path()).await;
    let locks = MaildropLockManager::new();

    let mut s = make_session(auth, storage, locks);

    let resp = s.handle_line_for_test("AUTH PLAIN\r\n").await;
    assert_eq!(resp.status(), Pop3Status::Continue);

    let resp = s.handle_line_for_test("*\r\n").await;
    assert_err(&resp, "aborted");
}
