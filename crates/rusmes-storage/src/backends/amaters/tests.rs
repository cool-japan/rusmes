//! Tests for the AmateRS storage backend.

use super::circuit_breaker::{CircuitBreaker, CircuitBreakerState};
use super::client::AmatersClient;
use super::config::{AmatersConfig, ConsistencyLevel};
use super::records::{MailboxRecord, MessageBlob, MessageRecord};
use super::AmatersBackend;
use crate::traits::StorageBackend;
use crate::types::{MailboxId, Quota};
use rusmes_proto::Username;
use std::collections::HashMap;

#[test]
fn test_amaters_config_default() {
    let config = AmatersConfig::default();
    assert_eq!(config.cluster_endpoints.len(), 1);
    assert_eq!(config.replication_factor, 3);
    assert_eq!(config.read_consistency, ConsistencyLevel::Quorum);
    assert_eq!(config.write_consistency, ConsistencyLevel::Quorum);
}

#[test]
fn test_consistency_levels() {
    assert_eq!(ConsistencyLevel::All, ConsistencyLevel::All);
    assert_eq!(ConsistencyLevel::Quorum, ConsistencyLevel::Quorum);
    assert_eq!(ConsistencyLevel::One, ConsistencyLevel::One);
    assert_ne!(ConsistencyLevel::All, ConsistencyLevel::One);
}

#[tokio::test]
async fn test_amaters_client_creation() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);
    assert!(client.connect().await.is_ok());
}

#[tokio::test]
async fn test_amaters_backend_creation() {
    let config = AmatersConfig::default();
    let backend = AmatersBackend::new(config).await;
    assert!(backend.is_ok());
}

#[tokio::test]
async fn test_put_and_get() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);
    client.connect().await.expect("mock connect");

    let key = "test_key".to_string();
    let value = vec![1, 2, 3, 4];

    client
        .put("metadata", key.clone(), value.clone())
        .await
        .expect("mock put");
    let retrieved = client.get("metadata", &key).await.expect("mock get");

    assert_eq!(retrieved, Some(value));
}

#[tokio::test]
async fn test_delete() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);
    client.connect().await.expect("mock connect");

    let key = "delete_key".to_string();
    let value = vec![5, 6, 7, 8];

    client
        .put("metadata", key.clone(), value)
        .await
        .expect("mock put");
    client.delete("metadata", &key).await.expect("mock delete");

    let retrieved = client.get("metadata", &key).await.expect("mock get");
    assert_eq!(retrieved, None);
}

#[tokio::test]
async fn test_list_prefix() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);
    client.connect().await.expect("mock connect");

    client
        .put("metadata", "user:alice:mailbox:1".to_string(), vec![])
        .await
        .expect("put alice 1");
    client
        .put("metadata", "user:alice:mailbox:2".to_string(), vec![])
        .await
        .expect("put alice 2");
    client
        .put("metadata", "user:bob:mailbox:1".to_string(), vec![])
        .await
        .expect("put bob 1");

    let alice_mailboxes = client
        .list_prefix("metadata", "user:alice:")
        .await
        .expect("list prefix alice");
    assert_eq!(alice_mailboxes.len(), 2);
}

#[test]
fn test_mailbox_record_serialization() {
    let record = MailboxRecord {
        id: "test-id".to_string(),
        username: "user@example.com".to_string(),
        path: vec!["INBOX".to_string()],
        uid_validity: 1,
        uid_next: 1,
        special_use: None,
        created_at: 1234567890,
    };

    let serialized = serde_json::to_vec(&record).expect("serialize");
    let deserialized: MailboxRecord = serde_json::from_slice(&serialized).expect("deserialize");

    assert_eq!(record.id, deserialized.id);
    assert_eq!(record.username, deserialized.username);
}

#[test]
fn test_message_record_serialization() {
    let record = MessageRecord {
        id: "msg-id".to_string(),
        mailbox_id: "mailbox-id".to_string(),
        uid: 1,
        sender: Some("sender@example.com".to_string()),
        recipients: vec!["recipient@example.com".to_string()],
        headers: HashMap::new(),
        size: 1024,
        blob_key: "blob:msg-id".to_string(),
        created_at: 1234567890,
    };

    let serialized = serde_json::to_vec(&record).expect("serialize");
    let deserialized: MessageRecord = serde_json::from_slice(&serialized).expect("deserialize");

    assert_eq!(record.id, deserialized.id);
    assert_eq!(record.size, deserialized.size);
}

#[test]
fn test_message_blob_serialization() {
    let blob = MessageBlob {
        message_id: "msg-id".to_string(),
        body: vec![1, 2, 3, 4],
        compressed: false,
    };

    let serialized = serde_json::to_vec(&blob).expect("serialize");
    let deserialized: MessageBlob = serde_json::from_slice(&serialized).expect("deserialize");

    assert_eq!(blob.message_id, deserialized.message_id);
    assert_eq!(blob.body, deserialized.body);
}

#[test]
fn test_amaters_config_custom() {
    let config = AmatersConfig {
        cluster_endpoints: vec![
            "node1.example.com:9042".to_string(),
            "node2.example.com:9042".to_string(),
        ],
        replication_factor: 5,
        read_consistency: ConsistencyLevel::LocalQuorum,
        write_consistency: ConsistencyLevel::All,
        ..Default::default()
    };

    assert_eq!(config.cluster_endpoints.len(), 2);
    assert_eq!(config.replication_factor, 5);
}

#[test]
fn test_keyspace_configuration() {
    let config = AmatersConfig {
        metadata_keyspace: "custom_metadata".to_string(),
        blob_keyspace: "custom_blobs".to_string(),
        ..Default::default()
    };

    assert_eq!(config.metadata_keyspace, "custom_metadata");
    assert_eq!(config.blob_keyspace, "custom_blobs");
}

#[test]
fn test_compression_flag() {
    let config = AmatersConfig {
        enable_compression: true,
        ..Default::default()
    };
    assert!(config.enable_compression);

    let config_no_compression = AmatersConfig {
        enable_compression: false,
        ..Default::default()
    };
    assert!(!config_no_compression.enable_compression);
}

#[test]
fn test_retry_configuration() {
    let config = AmatersConfig {
        max_retries: 5,
        ..Default::default()
    };
    assert_eq!(config.max_retries, 5);
}

#[test]
fn test_timeout_configuration() {
    let config = AmatersConfig {
        timeout_ms: 30000,
        ..Default::default()
    };
    assert_eq!(config.timeout_ms, 30000);
}

#[tokio::test]
async fn test_init_keyspaces() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);
    assert!(client.init_keyspaces().await.is_ok());
}

#[tokio::test]
async fn test_blob_keyspace_separation() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);

    client
        .put("metadata", "key1".to_string(), vec![1])
        .await
        .expect("put metadata");
    client
        .put("blobs", "key2".to_string(), vec![2])
        .await
        .expect("put blobs");

    let meta_val = client.get("metadata", "key1").await.expect("get meta");
    let blob_val = client.get("blobs", "key2").await.expect("get blob");

    assert_eq!(meta_val, Some(vec![1]));
    assert_eq!(blob_val, Some(vec![2]));
}

#[tokio::test]
async fn test_multiple_contact_points() {
    let config = AmatersConfig {
        cluster_endpoints: vec![
            "host1:9042".to_string(),
            "host2:9042".to_string(),
            "host3:9042".to_string(),
        ],
        ..Default::default()
    };

    let client = AmatersClient::new(config);
    assert!(client.connect().await.is_ok());
}

#[test]
fn test_circuit_breaker_creation() {
    let cb = CircuitBreaker::new(5, 60000);
    assert_eq!(cb.threshold, 5);
    assert_eq!(cb.timeout_ms, 60000);
}

#[tokio::test]
async fn test_circuit_breaker_closed_initially() {
    let cb = CircuitBreaker::new(3, 60000);
    assert!(!cb.is_open().await);
}

#[tokio::test]
async fn test_circuit_breaker_opens_after_threshold() {
    let cb = CircuitBreaker::new(3, 60000);

    cb.record_failure().await;
    assert!(!cb.is_open().await);

    cb.record_failure().await;
    assert!(!cb.is_open().await);

    cb.record_failure().await;
    assert!(cb.is_open().await);
}

#[tokio::test]
async fn test_circuit_breaker_reset_on_success() {
    let cb = CircuitBreaker::new(3, 60000);

    cb.record_failure().await;
    cb.record_failure().await;
    assert!(!cb.is_open().await);

    cb.record_success().await;
    let count = cb.failure_count.read().await;
    assert_eq!(*count, 0);
}

#[tokio::test]
async fn test_circuit_breaker_half_open_after_timeout() {
    let cb = CircuitBreaker::new(2, 100); // 100ms timeout

    cb.record_failure().await;
    cb.record_failure().await;
    assert!(cb.is_open().await);

    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    cb.attempt_reset().await;

    let state = cb.state.read().await;
    assert!(matches!(*state, CircuitBreakerState::HalfOpen));
}

#[tokio::test]
async fn test_config_cluster_endpoints() {
    let config = AmatersConfig::default();
    assert_eq!(config.cluster_endpoints.len(), 1);
    assert_eq!(config.cluster_endpoints[0], "localhost:9042");
}

#[tokio::test]
async fn test_config_timeout_ms() {
    let config = AmatersConfig {
        timeout_ms: 5000,
        ..Default::default()
    };
    assert_eq!(config.timeout_ms, 5000);
}

#[tokio::test]
async fn test_config_circuit_breaker_settings() {
    let config = AmatersConfig {
        circuit_breaker_threshold: 10,
        circuit_breaker_timeout_ms: 120000,
        ..Default::default()
    };
    assert_eq!(config.circuit_breaker_threshold, 10);
    assert_eq!(config.circuit_breaker_timeout_ms, 120000);
}

#[tokio::test]
async fn test_put_records_success() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);
    client.connect().await.expect("connect");

    client
        .put("metadata", "key1".to_string(), vec![1, 2, 3])
        .await
        .expect("put");

    // Verify data is actually stored (circuit breaker is at 0 failures = success).
    let result = client.get("metadata", "key1").await.expect("get");
    assert_eq!(result, Some(vec![1, 2, 3]));
}

#[tokio::test]
async fn test_get_nonexistent_key() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);

    let result = client.get("metadata", "nonexistent").await.expect("get");
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_delete_nonexistent_key() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);

    let result = client.delete("metadata", "nonexistent").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_list_prefix_empty() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);

    let keys = client
        .list_prefix("metadata", "empty:")
        .await
        .expect("list prefix");
    assert_eq!(keys.len(), 0);
}

#[tokio::test]
async fn test_blob_and_metadata_separation() {
    let config = AmatersConfig::default();
    let client = AmatersClient::new(config);

    client
        .put("metadata", "key1".to_string(), vec![1])
        .await
        .expect("put meta");
    client
        .put("blob_keyspace", "key1".to_string(), vec![2])
        .await
        .expect("put blob");

    let meta = client.get("metadata", "key1").await.expect("get meta");
    let blob = client.get("blob_keyspace", "key1").await.expect("get blob");

    assert_eq!(meta, Some(vec![1]));
    assert_eq!(blob, Some(vec![2]));
}

#[tokio::test]
async fn test_backend_stores_creation() {
    let config = AmatersConfig::default();
    let backend = AmatersBackend::new(config).await.expect("backend new");

    let _mailbox_store = backend.mailbox_store();
    let _message_store = backend.message_store();
    let _metadata_store = backend.metadata_store();
}

#[tokio::test]
async fn test_init_schema() {
    let config = AmatersConfig::default();
    let backend = AmatersBackend::new(config).await.expect("backend new");
    assert!(backend.init_schema().await.is_ok());
}

#[test]
fn test_consistency_level_all() {
    let level = ConsistencyLevel::All;
    assert_eq!(level, ConsistencyLevel::All);
}

#[test]
fn test_consistency_level_one() {
    let level = ConsistencyLevel::One;
    assert_eq!(level, ConsistencyLevel::One);
}

#[test]
fn test_consistency_level_local_quorum() {
    let level = ConsistencyLevel::LocalQuorum;
    assert_eq!(level, ConsistencyLevel::LocalQuorum);
}

#[tokio::test]
async fn test_mailbox_subscription() {
    let config = AmatersConfig::default();
    let backend = AmatersBackend::new(config).await.expect("backend new");
    let store = backend.mailbox_store();

    let user = Username::new("user@example.com".to_string()).expect("username");
    store
        .subscribe_mailbox(&user, "INBOX".to_string())
        .await
        .expect("subscribe");

    let subs = store.list_subscriptions(&user).await.expect("list subs");
    assert_eq!(subs.len(), 1);
    assert!(subs.contains(&"INBOX".to_string()));
}

#[tokio::test]
async fn test_mailbox_unsubscription() {
    let config = AmatersConfig::default();
    let backend = AmatersBackend::new(config).await.expect("backend new");
    let store = backend.mailbox_store();

    let user = Username::new("user@example.com".to_string()).expect("username");
    store
        .subscribe_mailbox(&user, "INBOX".to_string())
        .await
        .expect("subscribe");
    store
        .unsubscribe_mailbox(&user, "INBOX")
        .await
        .expect("unsubscribe");

    let subs = store.list_subscriptions(&user).await.expect("list subs");
    assert_eq!(subs.len(), 0);
}

#[tokio::test]
async fn test_multiple_subscriptions() {
    let config = AmatersConfig::default();
    let backend = AmatersBackend::new(config).await.expect("backend new");
    let store = backend.mailbox_store();

    let user = Username::new("user@example.com".to_string()).expect("username");
    store
        .subscribe_mailbox(&user, "INBOX".to_string())
        .await
        .expect("subscribe INBOX");
    store
        .subscribe_mailbox(&user, "Sent".to_string())
        .await
        .expect("subscribe Sent");
    store
        .subscribe_mailbox(&user, "Drafts".to_string())
        .await
        .expect("subscribe Drafts");

    let subs = store.list_subscriptions(&user).await.expect("list subs");
    assert_eq!(subs.len(), 3);
}

#[tokio::test]
async fn test_quota_operations() {
    let config = AmatersConfig::default();
    let backend = AmatersBackend::new(config).await.expect("backend new");
    let store = backend.metadata_store();

    let user = Username::new("user@example.com".to_string()).expect("username");
    let quota = Quota::new(1000, 10000);

    store.set_user_quota(&user, quota).await.expect("set quota");
    let retrieved = store.get_user_quota(&user).await.expect("get quota");

    assert_eq!(retrieved.used, 1000);
    assert_eq!(retrieved.limit, 10000);
}

#[tokio::test]
async fn test_mailbox_counters() {
    let config = AmatersConfig::default();
    let backend = AmatersBackend::new(config).await.expect("backend new");
    let store = backend.metadata_store();

    let mailbox_id = MailboxId::new();
    let counters = store
        .get_mailbox_counters(&mailbox_id)
        .await
        .expect("get counters");

    assert_eq!(counters.exists, 0);
    assert_eq!(counters.recent, 0);
    assert_eq!(counters.unseen, 0);
}

#[tokio::test]
async fn test_message_blob_compression_flag() {
    let blob = MessageBlob {
        message_id: "test-id".to_string(),
        body: vec![1, 2, 3, 4, 5],
        compressed: true,
    };

    assert!(blob.compressed);
    assert_eq!(blob.body.len(), 5);
}

#[tokio::test]
async fn test_replication_factor_config() {
    let config = AmatersConfig {
        replication_factor: 5,
        ..Default::default()
    };

    assert_eq!(config.replication_factor, 5);
}

#[tokio::test]
async fn test_custom_keyspace_names() {
    let config = AmatersConfig {
        metadata_keyspace: "custom_meta".to_string(),
        blob_keyspace: "custom_blob".to_string(),
        ..Default::default()
    };

    assert_eq!(config.metadata_keyspace, "custom_meta");
    assert_eq!(config.blob_keyspace, "custom_blob");
}

#[tokio::test]
async fn test_eventual_consistency_with_quorum() {
    let config = AmatersConfig {
        read_consistency: ConsistencyLevel::Quorum,
        write_consistency: ConsistencyLevel::Quorum,
        ..Default::default()
    };

    assert_eq!(config.read_consistency, ConsistencyLevel::Quorum);
    assert_eq!(config.write_consistency, ConsistencyLevel::Quorum);
}

#[tokio::test]
async fn test_eventual_consistency_with_one() {
    let config = AmatersConfig {
        read_consistency: ConsistencyLevel::One,
        write_consistency: ConsistencyLevel::One,
        ..Default::default()
    };

    assert_eq!(config.read_consistency, ConsistencyLevel::One);
    assert_eq!(config.write_consistency, ConsistencyLevel::One);
}

#[tokio::test]
async fn test_eventual_consistency_with_all() {
    let config = AmatersConfig {
        read_consistency: ConsistencyLevel::All,
        write_consistency: ConsistencyLevel::All,
        ..Default::default()
    };

    assert_eq!(config.read_consistency, ConsistencyLevel::All);
    assert_eq!(config.write_consistency, ConsistencyLevel::All);
}

#[test]
fn test_message_record_with_headers() {
    let mut headers = HashMap::new();
    headers.insert("From".to_string(), "sender@example.com".to_string());
    headers.insert("To".to_string(), "recipient@example.com".to_string());

    let record = MessageRecord {
        id: "msg-id".to_string(),
        mailbox_id: "mailbox-id".to_string(),
        uid: 1,
        sender: Some("sender@example.com".to_string()),
        recipients: vec!["recipient@example.com".to_string()],
        headers,
        size: 1024,
        blob_key: "blob:msg-id".to_string(),
        created_at: 1234567890,
    };

    assert_eq!(record.headers.len(), 2);
    assert_eq!(
        record.headers.get("From"),
        Some(&"sender@example.com".to_string())
    );
}

#[tokio::test]
async fn test_failover_retry_backoff() {
    let config = AmatersConfig {
        max_retries: 3,
        ..Default::default()
    };

    let client = AmatersClient::new(config);
    client.connect().await.expect("connect");

    // Put operation should succeed with retries
    let result = client
        .put("metadata", "test-key".to_string(), vec![1, 2, 3])
        .await;
    assert!(result.is_ok());
}

// -----------------------------------------------------------------------
// Plan block #3 — retry tuning
// -----------------------------------------------------------------------

#[test]
fn test_amaters_config_retry_defaults() {
    let cfg = AmatersConfig::default();
    assert_eq!(cfg.max_retries, 3);
    assert_eq!(cfg.initial_backoff_ms, 100);
    assert_eq!(cfg.max_backoff_ms, 5_000);
}

#[test]
fn test_amaters_config_from_url_with_retry_params() {
    let cfg =
        AmatersConfig::from_url("amaters://localhost:2375?max_retries=10&initial_backoff_ms=50")
            .expect("parse failed");
    assert_eq!(cfg.max_retries, 10);
    assert_eq!(cfg.initial_backoff_ms, 50);
    // max_backoff_ms was not in the URL — should keep default.
    assert_eq!(cfg.max_backoff_ms, 5_000);
}

#[test]
fn test_amaters_config_from_url_all_retry_params() {
    let cfg = AmatersConfig::from_url(
        "amaters://localhost:2375?max_retries=7&initial_backoff_ms=200&max_backoff_ms=8000",
    )
    .expect("parse failed");
    assert_eq!(cfg.max_retries, 7);
    assert_eq!(cfg.initial_backoff_ms, 200);
    assert_eq!(cfg.max_backoff_ms, 8_000);
}

#[test]
fn test_amaters_config_from_url_invalid_retry_param_errors() {
    let result = AmatersConfig::from_url("amaters://localhost:2375?max_retries=notanumber");
    assert!(result.is_err(), "should error on non-integer max_retries");
}

#[test]
fn test_amaters_config_retry_field_assignment() {
    let cfg = AmatersConfig {
        max_retries: 7,
        initial_backoff_ms: 200,
        max_backoff_ms: 10_000,
        ..AmatersConfig::default()
    };
    assert_eq!(cfg.max_retries, 7);
    assert_eq!(cfg.initial_backoff_ms, 200);
    assert_eq!(cfg.max_backoff_ms, 10_000);
}

// -----------------------------------------------------------------------
// Plan block #4 — replication/consistency warn
// -----------------------------------------------------------------------

#[test]
fn test_amaters_config_default_replication_factor_const() {
    assert_eq!(AmatersConfig::DEFAULT_REPLICATION_FACTOR, 3);
    let cfg = AmatersConfig::default();
    assert_eq!(
        cfg.replication_factor,
        AmatersConfig::DEFAULT_REPLICATION_FACTOR
    );
}

#[test]
fn test_consistency_level_default_is_quorum() {
    assert_eq!(ConsistencyLevel::default(), ConsistencyLevel::Quorum);
}

#[test]
fn test_amaters_config_default_consistency_levels() {
    let cfg = AmatersConfig::default();
    assert_eq!(cfg.read_consistency, ConsistencyLevel::default());
    assert_eq!(cfg.write_consistency, ConsistencyLevel::default());
}

// NOTE: tracing_test is not in the workspace dev-dependencies, so warn-emission
// integration tests are omitted here.  Add tracing_test to workspace if needed.

// -----------------------------------------------------------------------
// Integration tests — real AmateRS SDK (require AMATERS_TEST_ENDPOINT)
// -----------------------------------------------------------------------

#[cfg(feature = "amaters-backend")]
mod integration {
    use super::*;

    /// Return the test endpoint or skip the test.
    fn test_endpoint() -> Option<String> {
        std::env::var("AMATERS_TEST_ENDPOINT").ok()
    }

    fn make_config(endpoint: &str) -> AmatersConfig {
        AmatersConfig {
            cluster_endpoints: vec![endpoint.to_string()],
            ..AmatersConfig::default()
        }
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var (real amaters server)"]
    async fn test_amaters_set_get_roundtrip() {
        let endpoint = match test_endpoint() {
            Some(ep) => ep,
            None => return,
        };
        let config = make_config(&endpoint);
        let backend = AmatersBackend::connect_real(config)
            .await
            .expect("connect_real");

        let client = &backend.client;
        let key = format!("rusmes_test_set_get_{}", uuid::Uuid::new_v4().as_simple());
        let value = b"hello from rusmes integration test".to_vec();

        client
            .put(
                &backend.config.metadata_keyspace,
                key.clone(),
                value.clone(),
            )
            .await
            .expect("set");

        let retrieved = client
            .get(&backend.config.metadata_keyspace, &key)
            .await
            .expect("get");

        assert_eq!(retrieved, Some(value), "round-trip mismatch");

        // Cleanup
        client
            .delete(&backend.config.metadata_keyspace, &key)
            .await
            .expect("cleanup delete");
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var (real amaters server)"]
    async fn test_amaters_delete() {
        let endpoint = match test_endpoint() {
            Some(ep) => ep,
            None => return,
        };
        let config = make_config(&endpoint);
        let backend = AmatersBackend::connect_real(config)
            .await
            .expect("connect_real");

        let client = &backend.client;
        let key = format!("rusmes_test_delete_{}", uuid::Uuid::new_v4().as_simple());
        let value = b"ephemeral value".to_vec();

        // Store then delete
        client
            .put(&backend.config.metadata_keyspace, key.clone(), value)
            .await
            .expect("set");
        client
            .delete(&backend.config.metadata_keyspace, &key)
            .await
            .expect("delete");

        // Confirm gone
        let retrieved = client
            .get(&backend.config.metadata_keyspace, &key)
            .await
            .expect("get after delete");
        assert_eq!(retrieved, None, "key should be absent after delete");
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var (real amaters server)"]
    async fn test_amaters_list_prefix() {
        let endpoint = match test_endpoint() {
            Some(ep) => ep,
            None => return,
        };
        let config = make_config(&endpoint);
        let backend = AmatersBackend::connect_real(config)
            .await
            .expect("connect_real");

        let client = &backend.client;
        let ns = uuid::Uuid::new_v4().as_simple().to_string();
        let prefix = format!("rusmes_test_prefix_{ns}:");

        let keys: Vec<String> = (0..3).map(|i| format!("{prefix}item_{i}")).collect();

        for k in &keys {
            client
                .put(&backend.config.metadata_keyspace, k.clone(), b"v".to_vec())
                .await
                .expect("put");
        }

        let found = client
            .list_prefix(&backend.config.metadata_keyspace, &prefix)
            .await
            .expect("list_prefix");

        assert_eq!(found.len(), 3, "expected 3 keys under prefix");

        // Cleanup
        for k in &keys {
            client
                .delete(&backend.config.metadata_keyspace, k)
                .await
                .expect("cleanup");
        }
    }

    // -----------------------------------------------------------------------
    // Correctness tests for bug fixes landed 2026-05-05
    // -----------------------------------------------------------------------

    fn make_test_mail(body_content: &str) -> rusmes_proto::Mail {
        use bytes::Bytes;
        use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

        let mut headers = HeaderMap::new();
        headers.insert("from", "sender@example.com");
        headers.insert("to", "recipient@example.com");
        headers.insert("subject", "Test message");
        let body = MessageBody::Small(Bytes::from(body_content.to_string()));
        let mime = MimeMessage::new(headers, body);
        rusmes_proto::Mail::new(
            Some("sender@example.com".parse().expect("sender addr")),
            vec!["recipient@example.com".parse().expect("rcpt addr")],
            mime,
            None,
            None,
        )
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var (real amaters server)"]
    async fn test_amaters_real_message_body_roundtrip() {
        let endpoint = match test_endpoint() {
            Some(ep) => ep,
            None => return,
        };
        let config = make_config(&endpoint);
        let backend = AmatersBackend::connect_real(config)
            .await
            .expect("connect_real");

        use crate::traits::StorageBackend;
        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();

        // Create a test mailbox.
        let user = rusmes_proto::Username::new(format!(
            "testuser_{}@example.com",
            uuid::Uuid::new_v4().as_simple()
        ))
        .expect("username");
        let path = crate::types::MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
        let mailbox_id = mailbox_store
            .create_mailbox(&path)
            .await
            .expect("create mailbox");

        // Build a 4 KiB body.
        let body_content = "x".repeat(4096);
        let mail = make_test_mail(&body_content);
        let mail_id = *mail.message_id();

        // Append.
        let metadata = message_store
            .append_message(&mailbox_id, mail)
            .await
            .expect("append_message");
        assert_eq!(metadata.uid(), 1, "first message must get UID 1");

        // Fetch and compare body.
        let fetched = message_store
            .get_message(&mail_id)
            .await
            .expect("get_message")
            .expect("message must exist");

        let fetched_body = fetched
            .message()
            .extract_text()
            .await
            .expect("extract_text");

        assert_eq!(
            fetched_body, body_content,
            "fetched body must match what was stored"
        );

        // Cleanup.
        mailbox_store
            .delete_mailbox(&mailbox_id)
            .await
            .expect("cleanup mailbox");
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var (real amaters server)"]
    async fn test_amaters_real_uid_monotonic() {
        let endpoint = match test_endpoint() {
            Some(ep) => ep,
            None => return,
        };
        let config = make_config(&endpoint);
        let backend = AmatersBackend::connect_real(config)
            .await
            .expect("connect_real");

        use crate::traits::StorageBackend;
        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();

        let user = rusmes_proto::Username::new(format!(
            "uid_mono_{}@example.com",
            uuid::Uuid::new_v4().as_simple()
        ))
        .expect("username");
        let path = crate::types::MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
        let mailbox_id = mailbox_store
            .create_mailbox(&path)
            .await
            .expect("create mailbox");

        let mut uids = Vec::with_capacity(10);
        for i in 0..10u32 {
            let mail = make_test_mail(&format!("message {}", i));
            let meta = message_store
                .append_message(&mailbox_id, mail)
                .await
                .expect("append_message");
            uids.push(meta.uid());
        }

        // Verify UIDs are 1..=10 in order.
        let expected: Vec<u32> = (1..=10).collect();
        assert_eq!(uids, expected, "UIDs must be strictly monotonic 1..=10");

        mailbox_store
            .delete_mailbox(&mailbox_id)
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var (real amaters server)"]
    async fn test_amaters_real_uid_concurrent() {
        let endpoint = match test_endpoint() {
            Some(ep) => ep,
            None => return,
        };
        let config = make_config(&endpoint);
        let backend = std::sync::Arc::new(
            AmatersBackend::connect_real(config)
                .await
                .expect("connect_real"),
        );

        use crate::traits::StorageBackend;
        let mailbox_store = backend.mailbox_store();

        let user = rusmes_proto::Username::new(format!(
            "uid_conc_{}@example.com",
            uuid::Uuid::new_v4().as_simple()
        ))
        .expect("username");
        let path = crate::types::MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
        let mailbox_id = mailbox_store
            .create_mailbox(&path)
            .await
            .expect("create mailbox");

        const N: usize = 16;
        let mut handles = Vec::with_capacity(N);
        for i in 0..N {
            let backend_ref = backend.clone();
            let mailbox_id_copy = mailbox_id;
            handles.push(tokio::spawn(async move {
                let ms = backend_ref.message_store();
                let mail = make_test_mail(&format!("concurrent message {}", i));
                ms.append_message(&mailbox_id_copy, mail)
                    .await
                    .expect("concurrent append_message")
                    .uid()
            }));
        }

        let uids: Vec<u32> = {
            let mut collected = Vec::with_capacity(N);
            for handle in handles {
                collected.push(handle.await.expect("task panicked"));
            }
            collected
        };

        // Assert: no duplicates (all UIDs are unique).
        let unique: std::collections::HashSet<u32> = uids.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            N,
            "concurrent appends produced duplicate UIDs: {:?}",
            uids
        );

        // Assert: contiguous set [1..=N].
        let mut sorted = uids.clone();
        sorted.sort_unstable();
        let expected: Vec<u32> = (1..=(N as u32)).collect();
        assert_eq!(sorted, expected, "UIDs must be a contiguous set 1..=N");

        // Assert: counters.exists == N (counter RMW was serialised under mutex).
        let metadata_store = backend.metadata_store();
        let counters = metadata_store
            .get_mailbox_counters(&mailbox_id)
            .await
            .expect("get_mailbox_counters");
        assert_eq!(
            counters.exists, N as u32,
            "concurrent appends must yield exists == {N}, got {}",
            counters.exists,
        );

        mailbox_store
            .delete_mailbox(&mailbox_id)
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var (real amaters server)"]
    async fn test_amaters_real_messages_count() {
        let endpoint = match test_endpoint() {
            Some(ep) => ep,
            None => return,
        };
        let config = make_config(&endpoint);
        let backend = AmatersBackend::connect_real(config)
            .await
            .expect("connect_real");

        use crate::traits::StorageBackend;
        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();
        let metadata_store = backend.metadata_store();

        let user = rusmes_proto::Username::new(format!(
            "count_{}@example.com",
            uuid::Uuid::new_v4().as_simple()
        ))
        .expect("username");
        let path = crate::types::MailboxPath::new(user.clone(), vec!["INBOX".to_string()]);
        let mailbox_id = mailbox_store
            .create_mailbox(&path)
            .await
            .expect("create mailbox");

        for i in 0..5u32 {
            let mail = make_test_mail(&format!("count message {}", i));
            message_store
                .append_message(&mailbox_id, mail)
                .await
                .expect("append_message");
        }

        let counters = metadata_store
            .get_mailbox_counters(&mailbox_id)
            .await
            .expect("get_mailbox_counters");

        assert_eq!(
            counters.exists, 5,
            "exists counter must equal 5 after 5 appends"
        );

        mailbox_store
            .delete_mailbox(&mailbox_id)
            .await
            .expect("cleanup");
    }
}

// -----------------------------------------------------------------------
// Plan block #3 (endpoint cycling) — tests
// -----------------------------------------------------------------------

#[cfg(all(test, feature = "amaters-backend"))]
mod endpoint_cycling_tests {
    use super::*;

    /// Verify that the error message when all endpoints are unreachable lists them.
    #[tokio::test]
    #[ignore = "slow / network-dependent (all endpoints unreachable)"]
    async fn test_amaters_initial_connect_all_fail() {
        let config = AmatersConfig {
            cluster_endpoints: vec!["127.0.0.1:19991".to_string(), "127.0.0.1:19992".to_string()],
            timeout_ms: 500, // small timeout to keep test fast
            ..AmatersConfig::default()
        };
        let result = AmatersClient::new_real(&config).await;
        assert!(
            result.is_err(),
            "expected error when all endpoints unreachable"
        );
        let msg = result.err().expect("already checked is_err").to_string();
        assert!(
            msg.contains("19991") || msg.contains("unreachable"),
            "error message should mention endpoints: {msg}"
        );
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var set"]
    async fn test_amaters_initial_connect_first_endpoint_works() {
        let endpoint =
            std::env::var("AMATERS_TEST_ENDPOINT").expect("AMATERS_TEST_ENDPOINT must be set");
        let config = AmatersConfig {
            cluster_endpoints: vec![endpoint, "127.0.0.1:19993".to_string()],
            ..AmatersConfig::default()
        };
        let result = AmatersClient::new_real(&config).await;
        assert!(
            result.is_ok(),
            "expected connection via first endpoint: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    #[ignore = "requires AMATERS_TEST_ENDPOINT env var set"]
    async fn test_amaters_initial_connect_falls_back_to_second() {
        let endpoint =
            std::env::var("AMATERS_TEST_ENDPOINT").expect("AMATERS_TEST_ENDPOINT must be set");
        let config = AmatersConfig {
            cluster_endpoints: vec!["127.0.0.1:19994".to_string(), endpoint],
            timeout_ms: 500,
            ..AmatersConfig::default()
        };
        let result = AmatersClient::new_real(&config).await;
        assert!(
            result.is_ok(),
            "expected fallback to second endpoint: {:?}",
            result.err()
        );
    }
}
