//! Cluster 9 integration tests for rusmes-search.
//!
//! Covers: full rebuild from storage, incremental indexing on storage events,
//! result-cache hit on repeat query, and on-disk index size monitoring.

use bytes::Bytes;
use rusmes_proto::{HeaderMap, Mail, MailAddress, MessageBody, MessageId, MimeMessage, Username};
use rusmes_search::{
    spawn_incremental_indexer, IncrementalConfig, SearchIndex, TantivySearchIndex,
};
use rusmes_storage::backends::filesystem::FilesystemBackend;
use rusmes_storage::{MailboxPath, StorageBackend};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

fn make_mail(subject: &str, body: &str, from: &str, to: &str) -> Mail {
    let mut headers = HeaderMap::new();
    headers.insert("From", from);
    headers.insert("To", to);
    headers.insert("Subject", subject);
    let message = MimeMessage::new(
        headers,
        MessageBody::Small(Bytes::copy_from_slice(body.as_bytes())),
    );
    let from_addr = MailAddress::from_str(from).ok();
    let to_addr = MailAddress::from_str(to).ok();
    Mail::new(
        from_addr,
        to_addr.into_iter().collect(),
        message,
        None,
        None,
    )
}

async fn make_user_with_inbox(
    backend: &Arc<FilesystemBackend>,
    user_str: &str,
) -> (Username, rusmes_storage::MailboxId) {
    let user = Username::from_str(user_str).expect("valid username");
    let mb_store = backend.mailbox_store();
    let path = MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
    let id = mb_store
        .create_mailbox(&path)
        .await
        .expect("create mailbox");
    (user, id)
}

#[tokio::test]
async fn rebuild_indexes_all_messages() {
    let storage_dir = TempDir::new().expect("storage tempdir");
    let index_dir = TempDir::new().expect("index tempdir");

    let backend = Arc::new(FilesystemBackend::new(storage_dir.path()));
    let (_user, inbox_id) = make_user_with_inbox(&backend, "alice@example.com").await;

    // Append 20 messages with a known body word so search can find them.
    let msg_store = backend.message_store();
    for i in 0..20 {
        let mail = make_mail(
            &format!("hello {}", i),
            "needle body content here",
            "alice@example.com",
            "bob@example.com",
        );
        msg_store
            .append_message(&inbox_id, mail)
            .await
            .expect("append message");
    }

    let idx = TantivySearchIndex::new(index_dir.path()).expect("build index");

    let store: Arc<dyn StorageBackend> = backend.clone();
    let (n, _elapsed) = idx.rebuild(store.as_ref()).await.expect("rebuild");
    assert_eq!(n, 20, "rebuild should index all 20 messages");

    let results = idx.search("needle", 100).await.expect("search");
    assert_eq!(results.len(), 20, "all 20 messages should match");
}

#[tokio::test]
async fn incremental_indexing_on_event() {
    let storage_dir = TempDir::new().expect("storage tempdir");
    let index_dir = TempDir::new().expect("index tempdir");

    let backend = Arc::new(FilesystemBackend::new(storage_dir.path()));
    let (_user, inbox_id) = make_user_with_inbox(&backend, "carol@example.com").await;

    let idx = Arc::new(TantivySearchIndex::new(index_dir.path()).expect("build index"));
    let store: Arc<dyn StorageBackend> = backend.clone();

    // Aggressive commit cadence so the test sees the change quickly.
    let cfg = IncrementalConfig {
        commit_every_n: 1,
        commit_every: Duration::from_millis(100),
    };
    let _handle =
        rusmes_search::spawn_incremental_indexer_with_config(idx.clone(), store.clone(), cfg);

    // Give the subscriber a moment to register before we publish events.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let msg_store = backend.message_store();
    let mail = make_mail(
        "incremental",
        "uniquetokenxyz body",
        "carol@example.com",
        "dave@example.com",
    );
    msg_store
        .append_message(&inbox_id, mail)
        .await
        .expect("append");

    // Wait up to ~2 s for the indexer to commit.
    let mut found = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let r = idx.search("uniquetokenxyz", 10).await.expect("search");
        if !r.is_empty() {
            found = true;
            break;
        }
    }
    assert!(
        found,
        "incremental indexer should have indexed the message within 2s"
    );

    // Reference the public spawn fn so it stays exercised by name in tests.
    let _: fn(Arc<TantivySearchIndex>, Arc<dyn StorageBackend>) -> tokio::task::JoinHandle<()> =
        spawn_incremental_indexer;
}

#[tokio::test]
async fn result_cache_hit() {
    let index_dir = TempDir::new().expect("index tempdir");
    let idx = TantivySearchIndex::new(index_dir.path()).expect("build index");

    // Index two messages with the same word so the search has something to find.
    let mail_a = make_mail("a", "cachetoken alpha", "u@x", "v@x");
    let mail_b = make_mail("b", "cachetoken beta", "u@x", "v@x");
    let id_a = MessageId::new();
    let id_b = MessageId::new();
    idx.index_message(&id_a, &mail_a).await.expect("idx a");
    idx.index_message(&id_b, &mail_b).await.expect("idx b");
    idx.commit().await.expect("commit");

    // First call: cache miss -> real search -> populates cache. Real BM25
    // scores are nonzero for matched documents.
    let r1 = idx.search("cachetoken", 10).await.expect("search 1");
    assert_eq!(r1.len(), 2);
    assert!(
        r1.iter().any(|r| r.score > 0.0),
        "first call (cache miss) must return real BM25 scores; got {:?}",
        r1.iter().map(|r| r.score).collect::<Vec<_>>()
    );

    // Cache should now hold an entry.
    let cache = idx.cache();
    assert_eq!(
        cache.len(),
        1,
        "cache should hold one entry after first search"
    );
    let version_before = cache.version();

    // Second call: cache hit — short-circuits the searcher and returns
    // sentinel score=0.0 results. This is the discriminator: a real search
    // would have produced nonzero scores again, just like r1.
    let r2 = idx.search("cachetoken", 10).await.expect("search 2");
    assert_eq!(r2.len(), 2);
    assert!(
        r2.iter().all(|r| r.score == 0.0),
        "second call (cache hit) must return sentinel score=0.0; got {:?}",
        r2.iter().map(|r| r.score).collect::<Vec<_>>()
    );
    assert_eq!(
        cache.version(),
        version_before,
        "cache version must not change on a hit"
    );

    // The two calls must agree on which message IDs they returned.
    let mut r1_ids: Vec<_> = r1.iter().map(|r| r.message_uuid).collect();
    let mut r2_ids: Vec<_> = r2.iter().map(|r| r.message_uuid).collect();
    r1_ids.sort();
    r2_ids.sort();
    assert_eq!(
        r1_ids, r2_ids,
        "cache hit must return the same IDs as the cold search"
    );

    // Belt-and-suspenders: insert a third matching document via the
    // low-level path that does NOT invalidate the cache, commit it, and
    // confirm the next `search` still returns the cached two (not three).
    let id_c = MessageId::new();
    let mail_c = make_mail("c", "cachetoken gamma", "u@x", "v@x");
    idx.add_document_for_test(&id_c, &mail_c)
        .expect("low-level add c");
    idx.commit().await.expect("commit after silent add");
    let r3 = idx.search("cachetoken", 10).await.expect("search 3");
    assert_eq!(
        r3.len(),
        2,
        "cache hit must keep returning the original 2 IDs even though a 3rd document is now in the index"
    );

    // Now the public `index_message` path — version stamp should bump and
    // the old entry becomes stale, so the next search reflects all 3 docs.
    let id_d = MessageId::new();
    let mail_d = make_mail("d", "cachetoken delta", "u@x", "v@x");
    idx.index_message(&id_d, &mail_d).await.expect("idx d");
    idx.commit().await.expect("commit after invalidating add");
    assert!(
        cache.version() > version_before,
        "cache version must bump after a write through index_message"
    );
    let r4 = idx.search("cachetoken", 10).await.expect("search 4");
    assert_eq!(
        r4.len(),
        4,
        "after invalidation, search must see all 4 documents"
    );
}

#[tokio::test]
async fn index_size_bytes_grows() {
    let index_dir = TempDir::new().expect("index tempdir");
    let idx = TantivySearchIndex::new(index_dir.path()).expect("build index");

    // Empty (or near-empty) index — meta.json exists.
    let baseline = idx.index_size_bytes();

    // Index 10 messages and commit so files are flushed to disk.
    for i in 0..10 {
        let mail = make_mail(
            &format!("size {}", i),
            "filler body content for size monitoring test",
            "z@x",
            "y@x",
        );
        let id = MessageId::new();
        idx.index_message(&id, &mail).await.expect("index");
    }
    idx.commit().await.expect("commit");

    let after = idx.index_size_bytes();
    assert!(
        after > 0,
        "index_size_bytes must be > 0 after writes (got {})",
        after
    );
    assert!(
        after >= baseline,
        "index_size_bytes must not shrink after writes (baseline={}, after={})",
        baseline,
        after
    );
}
