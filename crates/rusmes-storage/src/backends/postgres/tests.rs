//! Tests for the PostgreSQL backend module.

use super::*;
use crate::types::{
    Mailbox, MailboxCounters, MailboxId, MailboxPath, MessageFlags, MessageMetadata, Quota,
    SearchCriteria, SpecialUseAttributes,
};
use rusmes_proto::{MessageId, Username};
use std::time::Duration;

#[test]
fn test_postgres_config_default() {
    let config = PostgresConfig::default();
    assert_eq!(config.max_connections, 20);
    assert_eq!(config.min_connections, 5);
    assert_eq!(config.inline_threshold, 100 * 1024);
}

#[test]
fn test_postgres_backend_struct() {
    let _ = std::mem::size_of::<PostgresBackend>();
}

#[test]
fn test_search_criteria_all() {
    let criteria = SearchCriteria::All;
    assert!(matches!(criteria, SearchCriteria::All));
}

#[test]
fn test_search_criteria_unseen() {
    let criteria = SearchCriteria::Unseen;
    assert!(matches!(criteria, SearchCriteria::Unseen));
}

#[test]
fn test_search_criteria_from() {
    let criteria = SearchCriteria::From("test@example.com".to_string());
    assert!(matches!(criteria, SearchCriteria::From(_)));
}

#[test]
fn test_search_criteria_subject() {
    let criteria = SearchCriteria::Subject("test subject".to_string());
    assert!(matches!(criteria, SearchCriteria::Subject(_)));
}

#[test]
fn test_message_flags_default() {
    let flags = MessageFlags::new();
    assert!(!flags.is_seen());
    assert!(!flags.is_answered());
    assert!(!flags.is_flagged());
    assert!(!flags.is_deleted());
    assert!(!flags.is_draft());
}

#[test]
fn test_message_flags_setters() {
    let mut flags = MessageFlags::new();
    flags.set_seen(true);
    flags.set_answered(true);
    flags.set_flagged(true);

    assert!(flags.is_seen());
    assert!(flags.is_answered());
    assert!(flags.is_flagged());
}

#[test]
fn test_quota_new() {
    let quota = Quota::new(1024, 2048);
    assert_eq!(quota.used, 1024);
    assert_eq!(quota.limit, 2048);
}

#[test]
fn test_quota_exceeded() {
    let quota = Quota::new(2048, 1024);
    assert!(quota.is_exceeded());

    let quota_ok = Quota::new(512, 1024);
    assert!(!quota_ok.is_exceeded());
}

#[test]
fn test_quota_remaining() {
    let quota = Quota::new(256, 1024);
    assert_eq!(quota.remaining(), 768);
}

#[test]
fn test_mailbox_counters_default() {
    let counters = MailboxCounters::default();
    assert_eq!(counters.exists, 0);
    assert_eq!(counters.recent, 0);
    assert_eq!(counters.unseen, 0);
}

#[test]
fn test_mailbox_id_new() {
    let id1 = MailboxId::new();
    let id2 = MailboxId::new();
    assert_ne!(id1, id2);
}

#[test]
fn test_mailbox_id_display() {
    let id = MailboxId::new();
    let display = format!("{}", id);
    assert!(!display.is_empty());
}

#[test]
fn test_mailbox_path_creation() {
    let user = Username::new("test@example.com".to_string()).unwrap();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    assert_eq!(path.user(), &user);
    assert_eq!(path.path().len(), 1);
}

#[test]
fn test_mailbox_path_name() {
    let user = Username::new("test@example.com".to_string()).unwrap();
    let path = MailboxPath::new(user, vec!["INBOX".to_string(), "Sent".to_string()]);
    assert_eq!(path.name(), Some("Sent"));
}

#[test]
fn test_mailbox_new() {
    let user = Username::new("test@example.com".to_string()).unwrap();
    let path = MailboxPath::new(user, vec!["INBOX".to_string()]);
    let mailbox = Mailbox::new(path);

    assert_eq!(mailbox.uid_validity(), 1);
    assert_eq!(mailbox.uid_next(), 1);
    assert!(mailbox.special_use().is_none());
}

#[test]
fn test_mailbox_special_use() {
    let user = Username::new("test@example.com".to_string()).unwrap();
    let path = MailboxPath::new(user, vec!["Sent".to_string()]);
    let mut mailbox = Mailbox::new(path);

    mailbox.set_special_use(Some("\\Sent".to_string()));
    assert_eq!(mailbox.special_use(), Some("\\Sent"));
}

#[test]
fn test_message_metadata_new() {
    let msg_id = MessageId::new();
    let mailbox_id = MailboxId::new();
    let flags = MessageFlags::new();

    let metadata = MessageMetadata::new(msg_id, mailbox_id, 1, flags, 1024);

    assert_eq!(metadata.message_id(), &msg_id);
    assert_eq!(metadata.mailbox_id(), &mailbox_id);
    assert_eq!(metadata.uid(), 1);
    assert_eq!(metadata.size(), 1024);
}

#[test]
fn test_message_metadata_getters() {
    let msg_id = MessageId::new();
    let mailbox_id = MailboxId::new();
    let metadata = MessageMetadata::new(msg_id, mailbox_id, 42, MessageFlags::new(), 2048);

    assert_eq!(*metadata.message_id(), msg_id);
    assert_eq!(*metadata.mailbox_id(), mailbox_id);
    assert_eq!(metadata.uid(), 42);
    assert_eq!(metadata.size(), 2048);
}

#[test]
fn test_search_criteria_and() {
    let criteria = SearchCriteria::And(vec![
        SearchCriteria::Unseen,
        SearchCriteria::From("test@example.com".to_string()),
    ]);
    assert!(matches!(criteria, SearchCriteria::And(_)));
}

#[test]
fn test_search_criteria_or() {
    let criteria = SearchCriteria::Or(vec![SearchCriteria::Flagged, SearchCriteria::Deleted]);
    assert!(matches!(criteria, SearchCriteria::Or(_)));
}

#[test]
fn test_search_criteria_not() {
    let criteria = SearchCriteria::Not(Box::new(SearchCriteria::Seen));
    assert!(matches!(criteria, SearchCriteria::Not(_)));
}

#[test]
fn test_mailbox_counters_struct() {
    let counters = MailboxCounters {
        exists: 10,
        recent: 3,
        unseen: 5,
    };
    assert_eq!(counters.exists, 10);
    assert_eq!(counters.recent, 3);
    assert_eq!(counters.unseen, 5);
}

#[test]
fn test_special_use_attributes_new() {
    let attrs = SpecialUseAttributes::new();
    assert!(attrs.is_empty());
}

#[test]
fn test_special_use_attributes_single() {
    let attrs = SpecialUseAttributes::single("\\Drafts".to_string());
    assert!(!attrs.is_empty());
    assert!(attrs.has_attribute("\\Drafts"));
}

#[test]
fn test_special_use_attributes_from_vec() {
    let vec = vec!["\\Drafts".to_string(), "\\Sent".to_string()];
    let attrs = SpecialUseAttributes::from_vec(vec);
    assert_eq!(attrs.len(), 2);
    assert!(attrs.has_attribute("\\Drafts"));
    assert!(attrs.has_attribute("\\Sent"));
}

#[test]
fn test_message_flags_custom() {
    let mut flags = MessageFlags::new();
    flags.add_custom("CustomFlag".to_string());
    assert!(flags.custom().contains("CustomFlag"));
}

#[test]
fn test_message_flags_recent() {
    let mut flags = MessageFlags::new();
    flags.set_recent(true);
    assert!(flags.is_recent());
}

#[test]
fn test_postgres_config_custom() {
    let config = PostgresConfig {
        max_connections: 50,
        min_connections: 10,
        connect_timeout: Duration::from_secs(60),
        idle_timeout: Some(Duration::from_secs(300)),
        max_lifetime: Some(Duration::from_secs(3600)),
        inline_threshold: 200 * 1024,
    };
    assert_eq!(config.max_connections, 50);
    assert_eq!(config.inline_threshold, 200 * 1024);
}

/// Test Item 3 (pattern-level): exercises the watch-channel / tokio::select!
/// shutdown machinery used by the background VACUUM task, WITHOUT requiring a
/// live PostgreSQL instance.  A real end-to-end test of
/// `PostgresBackend::with_config_and_vacuum` + `shutdown()` would require
/// `DATABASE_URL` to be set in the environment; that test would live under a
/// `#[cfg(feature = "postgres-tests")]` gate.  This test asserts only that the
/// channel pattern itself responds to the shutdown signal within 200 ms.
#[tokio::test]
async fn test_vacuum_loop_shutdown_pattern() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio::sync::watch;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let finished = Arc::new(AtomicBool::new(false));
    let finished_clone = finished.clone();

    let handle = tokio::spawn(async move {
        // Simulate the vacuum task loop with a very long interval (we'll
        // shut it down long before the first tick).
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        let mut rx = shutdown_rx;
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // No-op — task would normally run VACUUM here.
                }
                _ = rx.changed() => {
                    if *rx.borrow() {
                        finished_clone.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
        }
    });

    // Give the task a moment to start.
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Signal shutdown.
    let _ = shutdown_tx.send(true);

    // The task should exit within 200 ms.
    let timeout_result = tokio::time::timeout(Duration::from_millis(200), handle).await;

    assert!(
        timeout_result.is_ok(),
        "Vacuum task did not exit within 200 ms of shutdown signal"
    );
    assert!(
        finished.load(Ordering::SeqCst),
        "Vacuum task did not observe the shutdown signal"
    );
}
