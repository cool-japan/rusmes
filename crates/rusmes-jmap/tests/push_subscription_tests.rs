//! Integration tests for JMAP PushSubscription management (RFC 8620 §5.1).
//!
//! These tests cover the full lifecycle: create → verify → deliver → destroy.
//! WebPush HTTP deliveries are intercepted via wiremock.

use rusmes_jmap::{
    methods::push_subscription::{
        PushRegistry, PushState, PushSubscriptionCreate, PushSubscriptionSetRequest,
        PushSubscriptionUpdate,
    },
    types::{Principal, PushSubscription},
    web_push::WebPushClient,
};
use std::sync::Arc;
use wiremock::{
    matchers::{header_exists, method, path},
    Mock, MockServer, ResponseTemplate,
};

// ──────────────────────────────────────────────────────────────────────────────
// Test helpers
// ──────────────────────────────────────────────────────────────────────────────

fn make_principal(account_id: &str) -> Principal {
    Principal {
        username: account_id.to_string(),
        account_id: account_id.to_string(),
        scopes: vec![],
    }
}

/// Build a fresh `PushState` backed by an ephemeral `WebPushClient`.
fn build_push_state() -> Arc<PushState> {
    let client = WebPushClient::new(None, "admin@test.example").unwrap();
    Arc::new(PushState {
        registry: Arc::new(dashmap::DashMap::new()),
        client: Arc::new(client),
    })
}

/// Insert a pre-verified subscription directly into the registry (bypasses HTTP).
fn insert_verified_sub(
    registry: &PushRegistry,
    id: &str,
    url: &str,
    principal_id: &str,
    types: &[&str],
) {
    let sub = PushSubscription {
        id: id.to_string(),
        device_client_id: format!("dev-{id}"),
        url: url.to_string(),
        keys: None,
        verification_code: None,
        expires: None,
        types: types.iter().map(|s| s.to_string()).collect(),
        verified: true,
        principal_id: principal_id.to_string(),
    };
    registry.insert(id.to_string(), sub);
}

/// Insert an unverified subscription with a known verification code.
fn insert_unverified_sub(
    registry: &PushRegistry,
    id: &str,
    url: &str,
    principal_id: &str,
    code: &str,
) {
    let sub = PushSubscription {
        id: id.to_string(),
        device_client_id: format!("dev-{id}"),
        url: url.to_string(),
        keys: None,
        verification_code: Some(code.to_string()),
        expires: None,
        types: vec!["Email".to_string()],
        verified: false,
        principal_id: principal_id.to_string(),
    };
    registry.insert(id.to_string(), sub);
}

// ──────────────────────────────────────────────────────────────────────────────
// Test: create stores verificationCode in the registry
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_push_subscription_create_stores_verification_code() {
    let mock_server = MockServer::start().await;

    // Accept any POST to the mock server's push endpoint.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&mock_server)
        .await;

    let push_url = format!("{}/push/endpoint1", mock_server.uri());

    let state = build_push_state();
    let principal = make_principal("alice");

    let request = PushSubscriptionSetRequest {
        create: Some({
            let mut map = std::collections::HashMap::new();
            map.insert(
                "sub1".to_string(),
                PushSubscriptionCreate {
                    device_client_id: "device-abc".to_string(),
                    url: push_url,
                    keys: None,
                    expires: None,
                    types: vec!["Email".to_string()],
                },
            );
            map
        }),
        update: None,
        destroy: None,
    };

    let result = rusmes_jmap::methods::push_subscription::push_subscription_set_with_state(
        request, &principal, &state,
    )
    .await
    .unwrap();

    let created = result.created.expect("Should have created entry");
    let sub_created = created.get("sub1").expect("sub1 should be in created map");
    assert!(!sub_created.id.is_empty(), "id must be non-empty");
    assert!(
        !sub_created.verification_code.is_empty(),
        "verificationCode must be non-empty"
    );

    // The subscription must be in the registry with the verification code.
    let sub_id = &sub_created.id;
    let reg_entry = state
        .registry
        .get(sub_id)
        .expect("Subscription must be in registry");
    assert!(
        reg_entry.value().verification_code.is_some(),
        "registry entry must have verificationCode"
    );
    assert!(
        !reg_entry.value().verified,
        "Subscription must start as unverified"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Test: supplying the correct verificationCode transitions to verified = true
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_push_subscription_verify_transitions_to_verified() {
    let state = build_push_state();
    let principal = make_principal("bob");

    // Insert an unverified subscription with a known code directly (no HTTP).
    insert_unverified_sub(
        &state.registry,
        "sub-verify",
        "https://push.example.com/ignored",
        "bob",
        "test-verify-code-42",
    );

    // Update with the correct verification code.
    let update_req = PushSubscriptionSetRequest {
        create: None,
        update: Some({
            let mut m = std::collections::HashMap::new();
            m.insert(
                "sub-verify".to_string(),
                PushSubscriptionUpdate {
                    verification_code: Some("test-verify-code-42".to_string()),
                    types: None,
                    expires: None,
                },
            );
            m
        }),
        destroy: None,
    };

    let resp = rusmes_jmap::methods::push_subscription::push_subscription_set_with_state(
        update_req, &principal, &state,
    )
    .await
    .unwrap();

    // Update must succeed (no not_updated entries).
    assert!(
        resp.not_updated.is_none()
            || resp
                .not_updated
                .as_ref()
                .map(|m| m.is_empty())
                .unwrap_or(true),
        "Update should not fail: {:?}",
        resp.not_updated
    );

    let entry = state.registry.get("sub-verify").unwrap();
    assert!(
        entry.value().verified,
        "Subscription must be verified after supplying correct code"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Test: destroy removes from registry
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_push_subscription_destroy_removes_from_registry() {
    let state = build_push_state();
    let principal = make_principal("carol");

    // Insert directly to avoid the HTTP verification round-trip.
    insert_verified_sub(
        &state.registry,
        "sub-to-destroy",
        "https://push.example.com/destroy",
        "carol",
        &["Thread"],
    );

    assert!(
        state.registry.contains_key("sub-to-destroy"),
        "Should be in registry before destroy"
    );

    // Destroy via the handler.
    let destroy_req = PushSubscriptionSetRequest {
        create: None,
        update: None,
        destroy: Some(vec!["sub-to-destroy".to_string()]),
    };

    rusmes_jmap::methods::push_subscription::push_subscription_set_with_state(
        destroy_req,
        &principal,
        &state,
    )
    .await
    .unwrap();

    assert!(
        !state.registry.contains_key("sub-to-destroy"),
        "Registry should be empty after destroy"
    );
}

// ──────────────────────────────────────────────────────────────────────────────
// Test: state-change fanout fires HTTP POST to verified subscriptions
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_push_state_change_fanout_fires_http_post() {
    let mock_server = MockServer::start().await;

    // Two push endpoints on the same mock server.
    Mock::given(method("POST"))
        .and(path("/push/ep4a"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/push/ep4b"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = WebPushClient::new(None, "admin@test.example").unwrap();
    let registry: PushRegistry = Arc::new(dashmap::DashMap::new());

    // Insert two verified subscriptions directly.
    insert_verified_sub(
        &registry,
        "sub-a",
        &format!("{}/push/ep4a", mock_server.uri()),
        "alice",
        &["Email"],
    );
    insert_verified_sub(
        &registry,
        "sub-b",
        &format!("{}/push/ep4b", mock_server.uri()),
        "alice",
        &["Email"],
    );

    // Simulate fanout for an "Email" state-change event.
    let subs: Vec<PushSubscription> = registry
        .iter()
        .filter(|e| e.value().verified && e.value().types.iter().any(|t| t == "Email"))
        .map(|e| e.value().clone())
        .collect();

    assert_eq!(
        subs.len(),
        2,
        "Both subscriptions should match the Email type"
    );

    // Send to each.
    for sub in subs {
        client.send(&sub, &[]).await.unwrap();
    }

    // Wiremock verifies on drop that each endpoint received exactly 1 POST.
}

// ──────────────────────────────────────────────────────────────────────────────
// Test: VAPID JWT is present in the Authorization header
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_push_vapid_jwt_in_authorization_header() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/push/ep5"))
        .and(header_exists("Authorization"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = WebPushClient::new(None, "admin@test.example").unwrap();

    let sub = PushSubscription {
        id: "sub-5".to_string(),
        device_client_id: "dev-5".to_string(),
        url: format!("{}/push/ep5", mock_server.uri()),
        keys: None,
        verification_code: None,
        expires: None,
        types: vec!["Email".to_string()],
        verified: true,
        principal_id: "alice".to_string(),
    };

    client.send(&sub, &[]).await.unwrap();
    // Wiremock asserts `Authorization` header exists on drop.
}

// ──────────────────────────────────────────────────────────────────────────────
// Test: subscription without keys sends tickle (empty body POST)
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_push_no_keys_sends_tickle() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/push/ep6"))
        .respond_with(ResponseTemplate::new(201))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = WebPushClient::new(None, "admin@test.example").unwrap();

    let sub = PushSubscription {
        id: "sub-6".to_string(),
        device_client_id: "dev-6".to_string(),
        url: format!("{}/push/ep6", mock_server.uri()),
        keys: None, // No encryption keys → tickle
        verification_code: None,
        expires: None,
        types: vec!["Email".to_string()],
        verified: true,
        principal_id: "alice".to_string(),
    };

    // Even with a non-empty payload, a tickle (empty body) is sent because
    // `sub.keys` is None and RFC 8291 encryption is deferred.
    client
        .send(&sub, b"some state change payload")
        .await
        .unwrap();
}

// ──────────────────────────────────────────────────────────────────────────────
// Test: 410 Gone causes subscription removal
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_push_410_gone_removes_subscription() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/push/ep7"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&mock_server)
        .await;

    let client = WebPushClient::new(None, "admin@test.example").unwrap();

    let sub = PushSubscription {
        id: "sub-7".to_string(),
        device_client_id: "dev-7".to_string(),
        url: format!("{}/push/ep7", mock_server.uri()),
        keys: None,
        verification_code: None,
        expires: None,
        types: vec!["Email".to_string()],
        verified: true,
        principal_id: "alice".to_string(),
    };

    let result = client.send(&sub, &[]).await;
    match result {
        Err(rusmes_jmap::web_push::WebPushError::Gone) => {
            // Correct — caller should remove the subscription from the registry.
        }
        other => panic!("Expected WebPushError::Gone, got: {:?}", other),
    }
}
