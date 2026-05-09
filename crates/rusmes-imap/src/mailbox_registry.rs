//! Cross-session mailbox notification registry (Cluster 10 — RFC 4978 + concurrent access).
//!
//! A single [`MailboxRegistry`] is shared (via `Arc`) across all IMAP sessions. When a session
//! enters the `SELECTED` state it subscribes to the per-mailbox broadcast channel. When another
//! session mutates that mailbox (APPEND / STORE / EXPUNGE / MOVE), it publishes a
//! [`MailboxEvent`]. Subscribed sessions drain their [`tokio::sync::broadcast::Receiver`] and
//! emit the corresponding untagged IMAP responses.
//!
//! # Lifecycle
//!
//! - **Subscribe**: `registry.subscribe(mailbox_id)` → `broadcast::Receiver<MailboxEvent>`
//! - **Publish**: `registry.publish(mailbox_id, event)` (fire-and-forget; lagging receivers
//!   lose the oldest event — acceptable because the next command re-syncs state)
//! - **Cleanup**: when the session drops its `Receiver`, the broadcast channel shrinks the
//!   internal reference count automatically. If the last receiver drops, the `Sender` in the
//!   `DashMap` is the only remaining reference; it will be cleaned up on the next
//!   `cleanup_closed` call (or at the next subscribe for the same mailbox).

use dashmap::DashMap;
use rusmes_storage::MailboxId;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Capacity of each per-mailbox broadcast channel.
/// Lagging receivers drop the oldest event.
const CHANNEL_CAPACITY: usize = 256;

/// An event that can happen in a mailbox and needs to be broadcast to all sessions
/// that have it open.
#[derive(Debug, Clone)]
pub enum MailboxEvent {
    /// A new message was added; the mailbox now has `count` messages total.
    Exists { count: u32 },
    /// The RECENT count changed.
    Recent { count: u32 },
    /// A message at sequence number `seq` was expunged (1-based).
    Expunge { seq: u32 },
    /// A message's flags changed.
    FlagsChanged {
        /// Message UID.
        uid: u32,
        /// New flags as IMAP flag strings (e.g. `\\Seen`).
        flags: Vec<String>,
    },
}

/// Shared, `Arc`-cloneable registry mapping mailbox IDs to their broadcast senders.
#[derive(Clone, Default)]
pub struct MailboxRegistry {
    channels: Arc<DashMap<MailboxId, broadcast::Sender<MailboxEvent>>>,
}

impl MailboxRegistry {
    /// Create a new, empty registry.
    pub fn new() -> Self {
        Self {
            channels: Arc::new(DashMap::new()),
        }
    }

    /// Subscribe to events for `mailbox_id`.
    ///
    /// Returns a `Receiver` that will receive future events.
    /// If this is the first subscriber for the mailbox, a fresh channel is created.
    pub fn subscribe(&self, mailbox_id: MailboxId) -> broadcast::Receiver<MailboxEvent> {
        // Fast path: channel already exists.
        if let Some(sender) = self.channels.get(&mailbox_id) {
            return sender.subscribe();
        }
        // Slow path: create channel, but handle the race between multiple concurrent
        // first-subscribers by using `entry`.
        let sender = self.channels.entry(mailbox_id).or_insert_with(|| {
            let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
            tx
        });
        sender.subscribe()
    }

    /// Publish `event` to all sessions currently subscribed to `mailbox_id`.
    ///
    /// If there are no subscribers the event is silently dropped.
    pub fn publish(&self, mailbox_id: MailboxId, event: MailboxEvent) {
        if let Some(sender) = self.channels.get(&mailbox_id) {
            // Ignore the error: it means there are no active receivers.
            let _ = sender.send(event);
        }
    }

    /// Remove channels that have no remaining receivers, reclaiming memory.
    ///
    /// This is a best-effort sweep; correctness does not depend on it.
    pub fn cleanup_closed(&self) {
        self.channels
            .retain(|_, sender| sender.receiver_count() > 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribe_and_receive_exists() {
        let registry = MailboxRegistry::new();
        let mailbox_id = MailboxId::new();
        let mut rx = registry.subscribe(mailbox_id);

        registry.publish(mailbox_id, MailboxEvent::Exists { count: 5 });

        let event = rx.recv().await.expect("expected event");
        assert!(matches!(event, MailboxEvent::Exists { count: 5 }));
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_same_event() {
        let registry = MailboxRegistry::new();
        let mailbox_id = MailboxId::new();
        let mut rx1 = registry.subscribe(mailbox_id);
        let mut rx2 = registry.subscribe(mailbox_id);

        registry.publish(mailbox_id, MailboxEvent::Expunge { seq: 3 });

        let e1 = rx1.recv().await.expect("rx1 event");
        let e2 = rx2.recv().await.expect("rx2 event");
        assert!(matches!(e1, MailboxEvent::Expunge { seq: 3 }));
        assert!(matches!(e2, MailboxEvent::Expunge { seq: 3 }));
    }

    #[tokio::test]
    async fn no_receivers_publish_is_silent() {
        let registry = MailboxRegistry::new();
        let mailbox_id = MailboxId::new();
        // No subscriber — publish should not panic.
        registry.publish(mailbox_id, MailboxEvent::Exists { count: 1 });
    }

    #[tokio::test]
    async fn cleanup_closed_removes_empty_channels() {
        let registry = MailboxRegistry::new();
        let mailbox_id = MailboxId::new();
        {
            let _rx = registry.subscribe(mailbox_id);
        }
        // Receiver dropped, channel should be empty.
        registry.cleanup_closed();
        // After cleanup the entry should be gone.
        assert!(!registry.channels.contains_key(&mailbox_id));
    }

    // -------------------------------------------------------------------------
    // Cross-session notification integration tests
    // -------------------------------------------------------------------------
    //
    // These tests exercise the full publish → subscribe → drain pipeline using
    // the handler functions directly (no TCP, no server loop).  The sessions
    // ("session A" and "session B") are in-process, sharing a single
    // HandlerContext.  The FilesystemBackend is used with a temp directory so
    // the tests are self-contained and clean up after themselves.

    #[cfg(test)]
    mod cross_session {
        use crate::handler::{handle_command, HandlerContext};
        use crate::handler_mailbox::handle_select;
        use crate::mailbox_registry::MailboxRegistry;
        use crate::session::{ImapSession, ImapState};
        use async_trait::async_trait;
        use rusmes_proto::{HeaderMap, Mail, MailAddress, MailState, MessageBody, MimeMessage};
        use rusmes_storage::backends::filesystem::FilesystemBackend;
        use rusmes_storage::StorageBackend;
        use rusmes_storage::{MailboxPath, MailboxStore, MessageFlags, MessageStore};
        use std::sync::Arc;

        // Minimal auth backend for tests that just needs to exist in HandlerContext.
        struct NoopAuthBackend;

        #[async_trait]
        impl rusmes_auth::AuthBackend for NoopAuthBackend {
            async fn authenticate(
                &self,
                _user: &rusmes_proto::Username,
                _password: &str,
            ) -> anyhow::Result<bool> {
                Ok(false)
            }
            async fn verify_identity(
                &self,
                _user: &rusmes_proto::Username,
            ) -> anyhow::Result<bool> {
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

        /// Build a minimal test `Mail` object.
        fn make_test_mail() -> Mail {
            let headers = HeaderMap::new();
            let body = MessageBody::Small(bytes::Bytes::from(
                "From: a@b.com\r\nTo: c@d.com\r\n\r\nHello\r\n",
            ));
            let mime = MimeMessage::new(headers, body);
            let sender: Option<MailAddress> = "a@b.com".parse().ok();
            let recipients: Vec<MailAddress> = vec!["c@d.com".parse().expect("valid addr")];
            let mut mail = Mail::new(sender, recipients, mime, None, None);
            mail.state = MailState::LocalDelivery;
            mail
        }

        /// Create a HandlerContext backed by a FilesystemBackend in `dir`.
        async fn make_ctx_with_registry(
            dir: &std::path::Path,
            registry: Arc<MailboxRegistry>,
        ) -> (HandlerContext, Arc<dyn MailboxStore>, Arc<dyn MessageStore>) {
            let backend = FilesystemBackend::new(dir);
            let mb_store = backend.mailbox_store();
            let msg_store = backend.message_store();
            let meta_store = backend.metadata_store();
            let ctx = HandlerContext::with_registry(
                mb_store.clone(),
                msg_store.clone(),
                meta_store,
                Arc::new(NoopAuthBackend),
                registry,
            );
            (ctx, mb_store, msg_store)
        }

        /// Build an authenticated `ImapSession` for `user`.
        fn make_session(user: &str) -> ImapSession {
            let mut s = ImapSession::new();
            s.username = Some(user.parse().expect("valid username"));
            s.state = ImapState::Authenticated;
            s
        }

        // ------------------------------------------------------------------
        // test_cross_session_append_notification
        // ------------------------------------------------------------------
        /// Session A APPENDs to INBOX. Session B (which has INBOX selected) drains
        /// its broadcast receiver and sees `* N EXISTS` within 200 ms.
        #[tokio::test]
        async fn test_cross_session_append_notification() {
            let dir = std::env::temp_dir()
                .join(format!("rusmes-imap-test-append-{}", uuid::Uuid::new_v4()));
            tokio::fs::create_dir_all(&dir)
                .await
                .expect("create temp dir");

            let registry = Arc::new(MailboxRegistry::new());
            let (ctx, mb_store, _msg_store) = make_ctx_with_registry(&dir, registry.clone()).await;

            // Create INBOX for the test user.
            let user = "testuser@localhost";
            let user_obj: rusmes_proto::Username = user.parse().expect("valid username");
            let path = MailboxPath::new(user_obj.clone(), vec!["INBOX".to_string()]);
            mb_store
                .create_mailbox(&path)
                .await
                .expect("create mailbox");

            // Session B — SELECT INBOX (subscribes to broadcast channel).
            let mut session_b = make_session(user);
            handle_select(&ctx, &mut session_b, "A1", "INBOX", false)
                .await
                .expect("SELECT");
            assert!(
                session_b.mailbox_event_rx.is_some(),
                "session_b must have a broadcast receiver after SELECT"
            );

            // Session A — APPEND a message to INBOX.
            let mut session_a = make_session(user);
            session_a.state = ImapState::Authenticated;
            let raw_msg = b"From: a@b.com\r\nTo: testuser@localhost\r\n\r\nHello\r\n";
            let response = handle_command(
                &ctx,
                &mut session_a,
                "A2",
                crate::command::ImapCommand::Append {
                    mailbox: "INBOX".to_string(),
                    flags: vec![],
                    date_time: None,
                    message_literal: raw_msg.to_vec(),
                },
            )
            .await
            .expect("APPEND command");
            assert!(
                response.format().contains("APPENDUID") || response.format().contains("OK"),
                "APPEND should succeed: {}",
                response.format()
            );

            // The broadcast send is synchronous — no sleep needed; yield once.
            tokio::task::yield_now().await;

            // Session B drains its inbox.
            let untagged = session_b.drain_mailbox_events();
            assert!(
                !untagged.is_empty(),
                "session_b should have received at least one notification, got: {:?}",
                untagged
            );
            let has_exists = untagged.iter().any(|l| l.contains("EXISTS"));
            assert!(
                has_exists,
                "expected an EXISTS notification, got: {:?}",
                untagged
            );

            let _ = tokio::fs::remove_dir_all(&dir).await;
        }

        // ------------------------------------------------------------------
        // test_cross_session_expunge_notification
        // ------------------------------------------------------------------
        /// Session A EXPUNGEs a \Deleted message. Session B sees `* N EXPUNGE`.
        #[tokio::test]
        async fn test_cross_session_expunge_notification() {
            let dir = std::env::temp_dir()
                .join(format!("rusmes-imap-test-expunge-{}", uuid::Uuid::new_v4()));
            tokio::fs::create_dir_all(&dir)
                .await
                .expect("create temp dir");

            let registry = Arc::new(MailboxRegistry::new());
            let (ctx, mb_store, msg_store) = make_ctx_with_registry(&dir, registry.clone()).await;

            let user = "exptest@localhost";
            let user_obj: rusmes_proto::Username = user.parse().expect("valid username");
            let path = MailboxPath::new(user_obj.clone(), vec!["INBOX".to_string()]);
            let mailbox_id = mb_store
                .create_mailbox(&path)
                .await
                .expect("create mailbox");

            // Pre-append a message to INBOX so there is something to expunge.
            let meta = msg_store
                .append_message(&mailbox_id, make_test_mail())
                .await
                .expect("append");

            // Mark it as \Deleted.
            let mut del_flags = MessageFlags::new();
            del_flags.set_deleted(true);
            msg_store
                .set_flags(&[*meta.message_id()], del_flags)
                .await
                .expect("set deleted");

            // Session B — SELECT INBOX.
            let mut session_b = make_session(user);
            handle_select(&ctx, &mut session_b, "B1", "INBOX", false)
                .await
                .expect("SELECT");
            assert!(session_b.mailbox_event_rx.is_some());

            // Session A — EXPUNGE.
            let mut session_a = make_session(user);
            session_a.state = ImapState::Selected { mailbox_id };
            handle_command(
                &ctx,
                &mut session_a,
                "A1",
                crate::command::ImapCommand::Expunge,
            )
            .await
            .expect("EXPUNGE command");

            tokio::task::yield_now().await;

            let untagged = session_b.drain_mailbox_events();
            let has_expunge = untagged.iter().any(|l| l.contains("EXPUNGE"));
            assert!(
                has_expunge,
                "expected an EXPUNGE notification, got: {:?}",
                untagged
            );

            let _ = tokio::fs::remove_dir_all(&dir).await;
        }

        // ------------------------------------------------------------------
        // test_cross_session_flags_notification
        // ------------------------------------------------------------------
        /// Session A issues UID STORE +FLAGS (\Seen) on a real message.
        /// Session B (which has the mailbox SELECTed) should see
        /// `* N FETCH (FLAGS (...))` via the broadcast channel.
        #[tokio::test]
        async fn test_cross_session_flags_notification() {
            let dir = std::env::temp_dir()
                .join(format!("rusmes-imap-test-flags-{}", uuid::Uuid::new_v4()));
            tokio::fs::create_dir_all(&dir)
                .await
                .expect("create temp dir");

            let registry = Arc::new(MailboxRegistry::new());
            let (ctx, mb_store, msg_store) = make_ctx_with_registry(&dir, registry.clone()).await;

            let user = "flagstest@localhost";
            let user_obj: rusmes_proto::Username = user.parse().expect("valid username");
            let path = MailboxPath::new(user_obj.clone(), vec!["INBOX".to_string()]);
            let mailbox_id = mb_store
                .create_mailbox(&path)
                .await
                .expect("create mailbox");

            // Pre-append a message so we have a real UID to target.
            let meta = msg_store
                .append_message(&mailbox_id, make_test_mail())
                .await
                .expect("append message");
            let msg_uid = meta.uid();

            // Session B — SELECT INBOX (subscribes to broadcast channel).
            let mut session_b = make_session(user);
            handle_select(&ctx, &mut session_b, "B1", "INBOX", false)
                .await
                .expect("SELECT");
            assert!(
                session_b.mailbox_event_rx.is_some(),
                "session_b must have a broadcast receiver after SELECT"
            );

            // Verify the message is retrievable before issuing STORE.
            {
                let messages = ctx
                    .message_store
                    .get_mailbox_messages(&mailbox_id)
                    .await
                    .expect("get_mailbox_messages");
                assert!(
                    !messages.is_empty(),
                    "message must be visible via ctx.message_store before UID STORE; uid={msg_uid}"
                );
                assert!(
                    messages.iter().any(|m| m.uid() == msg_uid),
                    "message uid {msg_uid} not found; found uids: {:?}",
                    messages.iter().map(|m| m.uid()).collect::<Vec<_>>()
                );
            }

            // Session A — UID STORE +FLAGS (\Seen) on that message's UID.
            let mut session_a = make_session(user);
            session_a.state = ImapState::Selected { mailbox_id };
            handle_command(
                &ctx,
                &mut session_a,
                "A1",
                crate::command::ImapCommand::Uid {
                    subcommand: Box::new(crate::command::UidSubcommand::Store {
                        sequence: msg_uid.to_string(),
                        mode: crate::command::StoreMode::Add,
                        flags: vec!["\\Seen".to_string()],
                    }),
                },
            )
            .await
            .expect("UID STORE command");

            // The broadcast send is synchronous — yield once to let Tokio schedule.
            tokio::task::yield_now().await;

            // Session B drains its receiver — should see a FETCH FLAGS notification.
            let untagged = session_b.drain_mailbox_events();
            let has_fetch_flags = untagged
                .iter()
                .any(|l| l.contains("FETCH") && l.contains("FLAGS"));
            assert!(
                has_fetch_flags,
                "expected a FETCH FLAGS notification from UID STORE, got: {:?}",
                untagged
            );
            let has_seen = untagged
                .iter()
                .any(|l| l.contains("\\Seen") || l.contains("Seen"));
            assert!(
                has_seen,
                "expected \\Seen flag in notification, got: {:?}",
                untagged
            );

            let _ = tokio::fs::remove_dir_all(&dir).await;
        }

        // ------------------------------------------------------------------
        // COMPRESS=DEFLATE streaming tests (oxiarc-deflate 0.2.7 — landed 2026-05-06)
        // ------------------------------------------------------------------
        //
        // oxiarc-deflate 0.2.7 ships `RawDeflateWriter` and `RawInflateReader`
        // (behind the `async-io` feature) which implement `AsyncWrite`/`AsyncRead`
        // respectively.  Both preserve the LZ77 sliding window across flush
        // boundaries (RFC 4978 §3), enabling cross-frame back-references.

        /// Streaming multi-frame roundtrip: write three IMAP response frames
        /// with explicit `.flush()` between each, then read back through
        /// `RawInflateReader` and assert the decompressed output is identical.
        #[tokio::test]
        async fn test_compress_deflate_roundtrip_streaming() {
            use oxiarc_deflate::raw_stream::{RawDeflateWriter, RawInflateReader};
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let frames: [&[u8]; 3] = [
                b"* OK [CAPABILITY IMAP4rev1 COMPRESS=DEFLATE] RusMES ready\r\n",
                b"A001 OK [COMPRESSIONACTIVE] Begin DEFLATE compression\r\n",
                b"* 42 EXISTS\r\n",
            ];

            // Compress all three frames with explicit sync-flushes between them.
            let mut compressed_buf = Vec::<u8>::new();
            {
                let mut writer = RawDeflateWriter::new(&mut compressed_buf, 6);
                for frame in &frames {
                    writer.write_all(frame).await.expect("write frame");
                    writer.flush().await.expect("flush frame");
                }
            }

            // Decompress via RawInflateReader.
            let mut reader = RawInflateReader::new(std::io::Cursor::new(compressed_buf));
            let mut output = Vec::new();
            reader.read_to_end(&mut output).await.expect("read_to_end");

            let expected: Vec<u8> = frames.iter().flat_map(|f| f.iter().copied()).collect();
            assert_eq!(output, expected, "roundtrip mismatch");
        }

        /// LZ77 cross-frame back-reference: frame 2 repeats the unique payload from
        /// frame 1; the combined compressed output of two identical frames through
        /// a single writer must be smaller than two independent writes (each starting
        /// from a fresh compressor), proving the LZ77 sliding window persists across
        /// sync-flush boundaries (RFC 4978 §3).
        #[tokio::test]
        async fn test_compress_deflate_lz77_window_persists() {
            use oxiarc_deflate::raw_stream::RawDeflateWriter;
            use tokio::io::AsyncWriteExt;

            // 200-byte payload: long enough that LZ77 savings dominate the 5-byte
            // sync-flush marker overhead when back-referencing from frame 2.
            let unique_payload = b"XQVZ_IMAP_RFC4978_LZ77_WINDOW_TEST_7f3a8b2e9d1c \
                                   unique-non-compressible-preamble followed by more \
                                   data to ensure the payload is large enough for LZ77 \
                                   back-references to dominate sync-flush overhead!!";

            // Baseline: two independent writers (fresh LZ77 window each time).
            // Total bytes = frame1 + frame2, no cross-frame back-references.
            let independent_total;
            {
                let mut b1 = Vec::<u8>::new();
                let mut w1 = RawDeflateWriter::new(&mut b1, 6);
                w1.write_all(unique_payload).await.expect("w1 write");
                w1.flush().await.expect("w1 flush");
                drop(w1);

                let mut b2 = Vec::<u8>::new();
                let mut w2 = RawDeflateWriter::new(&mut b2, 6);
                w2.write_all(unique_payload).await.expect("w2 write");
                w2.flush().await.expect("w2 flush");
                drop(w2);

                independent_total = b1.len() + b2.len();
            }

            // Shared window: both frames through a SINGLE writer.
            // Frame 2 can back-reference frame 1's LZ77 dictionary.
            let shared_total;
            {
                let mut buf = Vec::<u8>::new();
                let mut w = RawDeflateWriter::new(&mut buf, 6);
                w.write_all(unique_payload)
                    .await
                    .expect("shared frame 1 write");
                w.flush().await.expect("shared frame 1 flush");
                w.write_all(unique_payload)
                    .await
                    .expect("shared frame 2 write");
                w.flush().await.expect("shared frame 2 flush");
                drop(w);
                shared_total = buf.len();
            }

            assert!(
                shared_total < independent_total,
                "shared-window total ({shared_total} B) should be less than \
                 independent total ({independent_total} B): LZ77 back-refs expected"
            );
        }
    }
}
