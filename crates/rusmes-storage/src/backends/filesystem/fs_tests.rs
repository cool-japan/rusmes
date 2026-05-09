//! Integration tests for the filesystem storage backend.
//!
//! Extracted from `mod.rs` to keep that file under the 2 000-line limit.

use super::*;

#[tokio::test]
async fn test_filesystem_backend() {
    let backend = FilesystemBackend::new("/tmp/rusmes-test");
    let mailbox_store = backend.mailbox_store();

    let user: Username = "testuser".parse().unwrap();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

    let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();
    let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();

    assert!(mailbox.is_some());
    assert_eq!(mailbox.unwrap().path().user(), &user);
}

#[tokio::test]
async fn test_get_mailbox_messages() {
    use rusmes_proto::{MailAddress, MimeMessage};

    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
    let backend = FilesystemBackend::new(&temp_dir);
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "testuser".parse().unwrap();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

    // Create mailbox
    let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

    // Create and append a test message
    let headers = rusmes_proto::HeaderMap::new();
    let body = rusmes_proto::MessageBody::Small(bytes::Bytes::from("Test message body"));
    let mime_message = MimeMessage::new(headers, body);

    let sender = Some("sender@example.com".parse::<MailAddress>().unwrap());
    let recipients = vec!["testuser@localhost".parse::<MailAddress>().unwrap()];
    let mail = rusmes_proto::Mail::new(sender, recipients, mime_message, None, None);

    // Append message
    let metadata = message_store
        .append_message(&mailbox_id, mail)
        .await
        .unwrap();
    assert_eq!(metadata.mailbox_id(), &mailbox_id);

    // Get mailbox messages
    let messages = message_store
        .get_mailbox_messages(&mailbox_id)
        .await
        .unwrap();

    // Verify we got the message back
    assert_eq!(messages.len(), 1, "Should have exactly 1 message");
    let msg = &messages[0];
    assert_eq!(msg.mailbox_id(), &mailbox_id);
    assert_eq!(msg.uid(), 1, "First message should have UID 1");
    assert!(msg.size() > 0, "Message should have non-zero size");

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_get_mailbox_messages_multiple() {
    use rusmes_proto::{MailAddress, MimeMessage};

    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
    let backend = FilesystemBackend::new(&temp_dir);
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "testuser".parse().unwrap();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

    // Create mailbox
    let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

    // Append multiple messages
    for i in 0..5 {
        let headers = rusmes_proto::HeaderMap::new();
        let body = rusmes_proto::MessageBody::Small(bytes::Bytes::from(format!(
            "Test message body {}",
            i
        )));
        let mime_message = MimeMessage::new(headers, body);

        let sender = Some(
            format!("sender{}@example.com", i)
                .parse::<MailAddress>()
                .unwrap(),
        );
        let recipients = vec!["testuser@localhost".parse::<MailAddress>().unwrap()];
        let mail = rusmes_proto::Mail::new(sender, recipients, mime_message, None, None);

        message_store
            .append_message(&mailbox_id, mail)
            .await
            .unwrap();
    }

    // Get mailbox messages
    let messages = message_store
        .get_mailbox_messages(&mailbox_id)
        .await
        .unwrap();

    // Verify we got all messages
    assert_eq!(messages.len(), 5, "Should have exactly 5 messages");

    // Verify UIDs are sequential
    for (i, msg) in messages.iter().enumerate() {
        assert_eq!(
            msg.uid(),
            (i + 1) as u32,
            "Message {} should have UID {}",
            i,
            i + 1
        );
        assert_eq!(msg.mailbox_id(), &mailbox_id);
        assert!(msg.size() > 0, "Message should have non-zero size");
    }

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_get_mailbox_messages_with_flags() {
    use rusmes_proto::{MailAddress, MimeMessage};

    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
    let backend = FilesystemBackend::new(&temp_dir);
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "testuser".parse().unwrap();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

    // Create mailbox
    let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

    // Append a message
    let headers = rusmes_proto::HeaderMap::new();
    let body = rusmes_proto::MessageBody::Small(bytes::Bytes::from("Test message with flags"));
    let mime_message = MimeMessage::new(headers, body);

    let sender = Some("sender@example.com".parse::<MailAddress>().unwrap());
    let recipients = vec!["testuser@localhost".parse::<MailAddress>().unwrap()];
    let mail = rusmes_proto::Mail::new(sender, recipients, mime_message, None, None);

    let _metadata = message_store
        .append_message(&mailbox_id, mail)
        .await
        .unwrap();

    // Initially, message should be in new/ directory with no flags
    let messages = message_store
        .get_mailbox_messages(&mailbox_id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    let initial_flags = messages[0].flags();
    assert!(
        !initial_flags.is_seen(),
        "New message should not be marked as seen"
    );

    // Manually move the message to cur/ with flags to simulate IMAP flag setting
    let mailbox_dir = temp_dir.join("mailboxes").join(mailbox_id.to_string());
    let new_dir = mailbox_dir.join("new");
    let cur_dir = mailbox_dir.join("cur");

    // Find the message file
    let mut entries = tokio::fs::read_dir(&new_dir).await.unwrap();
    if let Some(entry) = entries.next_entry().await.unwrap() {
        let old_filename = entry.file_name();
        let old_path = new_dir.join(&old_filename);

        // Create new filename with Seen flag (:2,S)
        let base_name = old_filename.to_str().unwrap();
        let new_filename = format!("{}:2,S", base_name.split(":2,").next().unwrap());
        let new_path = cur_dir.join(&new_filename);

        // Move the file
        tokio::fs::rename(&old_path, &new_path).await.unwrap();
    }

    // Re-read messages - should now see the Seen flag
    let messages = message_store
        .get_mailbox_messages(&mailbox_id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    let updated_flags = messages[0].flags();
    assert!(
        updated_flags.is_seen(),
        "Message should now be marked as seen"
    );

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_get_message_from_disk() {
    use rusmes_proto::{MailAddress, MimeMessage};

    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
    let backend = FilesystemBackend::new(&temp_dir);
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "testuser".parse().unwrap();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);

    // Create mailbox
    let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

    // Create and append a test message
    let headers = rusmes_proto::HeaderMap::new();
    let body =
        rusmes_proto::MessageBody::Small(bytes::Bytes::from("Test message for disk retrieval"));
    let mime_message = MimeMessage::new(headers, body);

    let sender = Some("sender@example.com".parse::<MailAddress>().unwrap());
    let recipients = vec!["testuser@localhost".parse::<MailAddress>().unwrap()];
    let mail = rusmes_proto::Mail::new(sender, recipients, mime_message, None, None);

    // Store the message ID before appending
    let message_id = *mail.message_id();

    // Append message
    let _metadata = message_store
        .append_message(&mailbox_id, mail)
        .await
        .unwrap();

    // Create a new backend instance to simulate a fresh start (empty cache)
    let backend2 = FilesystemBackend::new(&temp_dir);
    let message_store2 = backend2.message_store();

    // Try to retrieve the message - should load from disk
    let retrieved_mail = message_store2.get_message(&message_id).await.unwrap();

    // Verify we got the message back
    assert!(
        retrieved_mail.is_some(),
        "Should retrieve message from disk"
    );
    let retrieved = retrieved_mail.unwrap();
    assert_eq!(
        retrieved.message_id(),
        &message_id,
        "Message ID should match"
    );
    assert!(
        retrieved.size() > 0,
        "Retrieved message should have non-zero size"
    );

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_mailbox_metadata_persistence() {
    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));

    let user: Username = "testuser".parse().unwrap();
    let inbox_path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    let sent_path = MailboxPath::new(user.clone(), vec!["Sent".to_string()]);

    let mailbox_id;
    let sent_id;

    // Create mailboxes in first backend instance
    {
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();

        mailbox_id = mailbox_store.create_mailbox(&inbox_path).await.unwrap();
        sent_id = mailbox_store.create_mailbox(&sent_path).await.unwrap();

        // Verify mailboxes exist
        let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();
        assert!(mailbox.is_some());
        assert_eq!(mailbox.unwrap().path().name(), Some("INBOX"));

        let sent_mailbox = mailbox_store.get_mailbox(&sent_id).await.unwrap();
        assert!(sent_mailbox.is_some());
        assert_eq!(sent_mailbox.unwrap().path().name(), Some("Sent"));

        // Verify metadata file was created
        let metadata_file = temp_dir
            .join("users")
            .join(user.as_str())
            .join("mailboxes.json");
        assert!(tokio::fs::try_exists(&metadata_file).await.unwrap());
    }

    // Create new backend instance (simulates server restart)
    {
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();

        // Verify mailboxes still exist after "restart"
        let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();
        assert!(mailbox.is_some(), "INBOX should be restored from disk");
        assert_eq!(mailbox.unwrap().path().name(), Some("INBOX"));

        let sent_mailbox = mailbox_store.get_mailbox(&sent_id).await.unwrap();
        assert!(sent_mailbox.is_some(), "Sent should be restored from disk");
        assert_eq!(sent_mailbox.unwrap().path().name(), Some("Sent"));

        // List mailboxes should return both
        let mailboxes = mailbox_store.list_mailboxes(&user).await.unwrap();
        assert_eq!(mailboxes.len(), 2, "Should have 2 mailboxes after restart");

        // Test delete and verify persistence
        mailbox_store.delete_mailbox(&sent_id).await.unwrap();
        let deleted_mailbox = mailbox_store.get_mailbox(&sent_id).await.unwrap();
        assert!(deleted_mailbox.is_none(), "Sent mailbox should be deleted");
    }

    // Create third backend instance to verify deletion was persisted
    {
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();

        // INBOX should still exist
        let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();
        assert!(mailbox.is_some(), "INBOX should still exist");

        // Sent should not exist
        let sent_mailbox = mailbox_store.get_mailbox(&sent_id).await.unwrap();
        assert!(sent_mailbox.is_none(), "Sent should still be deleted");

        // List should only return INBOX
        let mailboxes = mailbox_store.list_mailboxes(&user).await.unwrap();
        assert_eq!(mailboxes.len(), 1, "Should have 1 mailbox after restart");
        assert_eq!(mailboxes[0].path().name(), Some("INBOX"));
    }

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_mailbox_metadata_persistence_multiple_users() {
    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));

    let user1: Username = "user1".parse().unwrap();
    let user2: Username = "user2".parse().unwrap();

    let user1_inbox = MailboxPath::new(user1.clone(), vec!["INBOX".to_string()]);
    let user2_inbox = MailboxPath::new(user2.clone(), vec!["INBOX".to_string()]);

    // Create mailboxes for both users
    {
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();

        mailbox_store.create_mailbox(&user1_inbox).await.unwrap();
        mailbox_store.create_mailbox(&user2_inbox).await.unwrap();
    }

    // Verify both users' mailboxes are restored
    {
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();

        let user1_mailboxes = mailbox_store.list_mailboxes(&user1).await.unwrap();
        assert_eq!(user1_mailboxes.len(), 1);
        assert_eq!(user1_mailboxes[0].path().user(), &user1);

        let user2_mailboxes = mailbox_store.list_mailboxes(&user2).await.unwrap();
        assert_eq!(user2_mailboxes.len(), 1);
        assert_eq!(user2_mailboxes[0].path().user(), &user2);
    }

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_mailbox_metadata_rename_persistence() {
    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));

    let user: Username = "testuser".parse().unwrap();
    let original_path = MailboxPath::new(user.clone(), vec!["OldName".to_string()]);
    let new_path = MailboxPath::new(user.clone(), vec!["NewName".to_string()]);

    let mailbox_id;

    // Create and rename mailbox
    {
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();

        mailbox_id = mailbox_store.create_mailbox(&original_path).await.unwrap();
        mailbox_store
            .rename_mailbox(&mailbox_id, &new_path)
            .await
            .unwrap();
    }

    // Verify rename was persisted
    {
        let backend = FilesystemBackend::new(&temp_dir);
        let mailbox_store = backend.mailbox_store();

        let mailbox = mailbox_store.get_mailbox(&mailbox_id).await.unwrap();
        assert!(mailbox.is_some());
        assert_eq!(mailbox.unwrap().path().name(), Some("NewName"));
    }

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_append_message_has_thread_id() {
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};

    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
    let backend = FilesystemBackend::new(&temp_dir);
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "threaduser".parse().unwrap();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

    // Build a message with a Message-ID header so threading can produce a stable ID.
    let mut headers = HeaderMap::new();
    headers.insert("message-id", "<test-thread-msg@example.com>".to_string());
    headers.insert("subject", "Threading Test".to_string());
    let body = MessageBody::Small(bytes::Bytes::from("Thread test body"));
    let mime = MimeMessage::new(headers, body);
    let mail = rusmes_proto::Mail::new(
        Some("sender@example.com".parse::<MailAddress>().unwrap()),
        vec!["threaduser@localhost".parse::<MailAddress>().unwrap()],
        mime,
        None,
        None,
    );

    let metadata = message_store
        .append_message(&mailbox_id, mail)
        .await
        .unwrap();

    assert!(
        metadata.thread_id.is_some(),
        "Appended message must have a non-None thread_id"
    );
    let tid = metadata.thread_id.as_ref().unwrap();
    assert_eq!(tid.len(), 16, "thread_id must be 16 hex chars");

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_reply_thread_id_matches() {
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};

    let temp_dir = std::env::temp_dir().join(format!("rusmes-test-{}", uuid::Uuid::new_v4()));
    let backend = FilesystemBackend::new(&temp_dir);
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "threadreply".parse().unwrap();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    let mailbox_id = mailbox_store.create_mailbox(&path).await.unwrap();

    // Append original message.
    let mut orig_headers = HeaderMap::new();
    orig_headers.insert("message-id", "<original-thread@example.com>".to_string());
    orig_headers.insert("subject", "Original thread message".to_string());
    let orig_body = MessageBody::Small(bytes::Bytes::from("Original body"));
    let orig_mime = MimeMessage::new(orig_headers, orig_body);
    let orig_mail = rusmes_proto::Mail::new(
        Some("sender@example.com".parse::<MailAddress>().unwrap()),
        vec!["threadreply@localhost".parse::<MailAddress>().unwrap()],
        orig_mime,
        None,
        None,
    );
    let orig_meta = message_store
        .append_message(&mailbox_id, orig_mail)
        .await
        .unwrap();
    let original_tid = orig_meta
        .thread_id
        .as_ref()
        .expect("Original must have thread_id")
        .clone();

    // Append a reply referencing the original.
    let mut reply_headers = HeaderMap::new();
    reply_headers.insert("message-id", "<reply-thread@example.com>".to_string());
    reply_headers.insert("in-reply-to", "<original-thread@example.com>".to_string());
    reply_headers.insert("subject", "Re: Original thread message".to_string());
    let reply_body = MessageBody::Small(bytes::Bytes::from("Reply body"));
    let reply_mime = MimeMessage::new(reply_headers, reply_body);
    let reply_mail = rusmes_proto::Mail::new(
        Some("replier@example.com".parse::<MailAddress>().unwrap()),
        vec!["threadreply@localhost".parse::<MailAddress>().unwrap()],
        reply_mime,
        None,
        None,
    );
    let reply_meta = message_store
        .append_message(&mailbox_id, reply_mail)
        .await
        .unwrap();
    let reply_tid = reply_meta
        .thread_id
        .as_ref()
        .expect("Reply must have thread_id")
        .clone();

    assert_eq!(
        original_tid, reply_tid,
        "Original and reply must share the same thread_id"
    );

    // Verify round-trip via get_mailbox_messages.
    let messages = message_store
        .get_mailbox_messages(&mailbox_id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2, "Mailbox must have exactly 2 messages");
    for msg in &messages {
        assert!(
            msg.thread_id.is_some(),
            "All messages returned by get_mailbox_messages must have thread_id"
        );
        assert_eq!(
            msg.thread_id.as_ref().unwrap(),
            &original_tid,
            "Both messages must share the same thread_id"
        );
    }

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

/// Test Item 1: 16 truly-concurrent `append_message` calls to the same mailbox
/// must produce 16 unique UIDs with no duplicates or lost deliveries.
///
/// All 16 tasks are spawned simultaneously.  The in-process per-MailboxId
/// `tokio::sync::Mutex` in `FilesystemMessageStore::per_mailbox_mutex` serialises
/// them at the Tokio level, so only one task at a time reaches the `fs2` file
/// lock.  This keeps every task well within the 2-second retry budget regardless
/// of how many concurrent callers there are.
#[tokio::test]
async fn test_concurrent_deliver_no_duplicate_uids() {
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};
    use std::collections::HashSet;
    use std::sync::Arc;

    let temp_dir = std::env::temp_dir().join(format!("rusmes-concurrent-{}", uuid::Uuid::new_v4()));
    let backend = Arc::new(FilesystemBackend::new(&temp_dir));
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "concurrentuser".parse().expect("username");
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    let mailbox_id = mailbox_store
        .create_mailbox(&path)
        .await
        .expect("create_mailbox");

    // 16 truly-concurrent deliveries — all spawned at once.
    const N: usize = 16;
    let mut handles = Vec::with_capacity(N);

    for i in 0..N {
        let ms = message_store.clone();
        let mid = mailbox_id;
        handles.push(tokio::spawn(async move {
            let mut headers = HeaderMap::new();
            headers.insert("subject", format!("Concurrent message {}", i));
            let body = MessageBody::Small(bytes::Bytes::from(format!("Body {}", i)));
            let mime = MimeMessage::new(headers, body);
            let mail = rusmes_proto::Mail::new(
                Some("sender@example.com".parse::<MailAddress>().expect("addr")),
                vec!["concurrentuser@localhost"
                    .parse::<MailAddress>()
                    .expect("addr")],
                mime,
                None,
                None,
            );
            ms.append_message(&mid, mail).await
        }));
    }

    let mut uids = HashSet::new();
    for handle in handles {
        let metadata = handle
            .await
            .expect("task did not panic")
            .expect("append_message succeeded");
        assert!(
            uids.insert(metadata.uid()),
            "Duplicate UID detected: {}",
            metadata.uid()
        );
    }

    assert_eq!(
        uids.len(),
        N,
        "Expected {} unique UIDs, got {}",
        N,
        uids.len()
    );

    // Verify on-disk count matches.
    let messages = message_store
        .get_mailbox_messages(&mailbox_id)
        .await
        .expect("get_mailbox_messages");
    assert_eq!(
        messages.len(),
        N,
        "Expected {} messages on disk, got {}",
        N,
        messages.len()
    );

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

/// Test Item 4: `build_storage(Filesystem)` round-trips a single message.
#[tokio::test]
async fn test_build_storage_filesystem() {
    use crate::{build_storage, BackendKind, MailboxPath};
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};

    let temp_dir = std::env::temp_dir().join(format!("rusmes-factory-{}", uuid::Uuid::new_v4()));
    let kind = BackendKind::Filesystem {
        path: temp_dir.to_string_lossy().to_string(),
    };

    let backend = build_storage(&kind)
        .await
        .expect("build_storage(Filesystem)");
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "factoryuser".parse().expect("username");
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    let mailbox_id = mailbox_store
        .create_mailbox(&path)
        .await
        .expect("create_mailbox");

    let headers = HeaderMap::new();
    let body = MessageBody::Small(bytes::Bytes::from("Factory test body"));
    let mime = MimeMessage::new(headers, body);
    let mail = rusmes_proto::Mail::new(
        Some("sender@example.com".parse::<MailAddress>().expect("addr")),
        vec!["factoryuser@localhost"
            .parse::<MailAddress>()
            .expect("addr")],
        mime,
        None,
        None,
    );

    let metadata = message_store
        .append_message(&mailbox_id, mail)
        .await
        .expect("append_message via factory backend");

    assert_eq!(metadata.mailbox_id(), &mailbox_id);
    assert!(metadata.uid() > 0);

    let messages = message_store
        .get_mailbox_messages(&mailbox_id)
        .await
        .expect("get_mailbox_messages via factory backend");
    assert_eq!(messages.len(), 1);

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

/// Test Item 5: backup → clear → restore → message count matches.
#[tokio::test]
async fn test_backup_restore_roundtrip_full() {
    use crate::{backup, restore};
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};

    let temp_dir = std::env::temp_dir().join(format!("rusmes-backup-src-{}", uuid::Uuid::new_v4()));
    let restore_dir =
        std::env::temp_dir().join(format!("rusmes-backup-dst-{}", uuid::Uuid::new_v4()));
    let archive_path =
        std::env::temp_dir().join(format!("rusmes-backup-{}.zip", uuid::Uuid::new_v4()));

    let backend = FilesystemBackend::new(&temp_dir);
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let user: Username = "backupuser".parse().expect("username");
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    let mailbox_id = mailbox_store
        .create_mailbox(&path)
        .await
        .expect("create_mailbox");

    // Deliver 5 messages.
    for i in 0..5u32 {
        let mut headers = HeaderMap::new();
        headers.insert("subject", format!("Backup message {}", i));
        let body = MessageBody::Small(bytes::Bytes::from(format!("Body content {}", i)));
        let mime = MimeMessage::new(headers, body);
        let mail = rusmes_proto::Mail::new(
            Some("sender@example.com".parse::<MailAddress>().expect("addr")),
            vec!["backupuser@localhost".parse::<MailAddress>().expect("addr")],
            mime,
            None,
            None,
        );
        message_store
            .append_message(&mailbox_id, mail)
            .await
            .expect("append_message for backup test");
    }

    // Backup.
    backup(&backend, &archive_path)
        .await
        .expect("backup should succeed");
    assert!(
        archive_path.exists(),
        "Archive file must exist after backup"
    );

    // Restore into a fresh directory.
    let restore_backend = FilesystemBackend::new(&restore_dir);
    restore(&restore_backend, &archive_path)
        .await
        .expect("restore should succeed");

    // Verify: the restored directory should contain 5 message files.
    let mailboxes_dir = restore_dir.join("mailboxes");
    assert!(
        mailboxes_dir.exists(),
        "Restored mailboxes/ directory must exist"
    );

    let mut total_messages = 0usize;
    let mut mailbox_dirs = tokio::fs::read_dir(&mailboxes_dir)
        .await
        .expect("read_dir mailboxes");
    while let Some(entry) = mailbox_dirs
        .next_entry()
        .await
        .expect("next_entry mailboxes")
    {
        if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            for subdir in &["new", "cur"] {
                let msg_dir = entry.path().join(subdir);
                if tokio::fs::try_exists(&msg_dir).await.unwrap_or(false) {
                    let mut msgs = tokio::fs::read_dir(&msg_dir)
                        .await
                        .expect("read_dir subdir");
                    while let Some(msg) = msgs.next_entry().await.expect("msg entry") {
                        if msg.file_type().await.map(|t| t.is_file()).unwrap_or(false) {
                            total_messages += 1;
                        }
                    }
                }
            }
        }
    }

    assert_eq!(
        total_messages, 5,
        "Expected 5 messages after restore, found {}",
        total_messages
    );

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    let _ = tokio::fs::remove_dir_all(&restore_dir).await;
    let _ = tokio::fs::remove_file(&archive_path).await;
}
