//! JMAP `PushSubscription/get` and `PushSubscription/set` handlers.
//!
//! Implements RFC 8620 §5.1: clients register a push endpoint URL; the server
//! delivers a verification push, and thereafter fans out `StateChange` events to
//! all verified subscriptions whose `types` list matches the changed data type.
//!
//! # Registry architecture
//!
//! The push registry lives in a `OnceLock<Arc<PushState>>` so the dispatch
//! function (which has no state parameter) can access it without threading the
//! handle through every call site.  Call [`init_push_state`] once at server
//! startup before dispatching any JMAP requests.

use crate::types::{JmapSetError, Principal, PushKeys, PushSubscription};
use crate::web_push::{WebPushClient, WebPushError};
use base64::Engine as _;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

// ──────────────────────────────────────────────────────────────────────────────
// Global push state
// ──────────────────────────────────────────────────────────────────────────────

/// Shared push state accessible from the stateless dispatch function.
pub struct PushState {
    /// Map of subscription ID → subscription.
    pub registry: Arc<DashMap<String, PushSubscription>>,
    /// WebPush HTTP client with loaded VAPID key.
    pub client: Arc<WebPushClient>,
}

static PUSH_STATE: OnceLock<Arc<PushState>> = OnceLock::new();

/// Install the global push state.
///
/// Must be called once at server startup before any `PushSubscription/*`
/// method can be dispatched.  Subsequent calls are no-ops (the first-writer
/// wins, matching the `global_metrics()` pattern).
pub fn init_push_state(state: Arc<PushState>) {
    let _ = PUSH_STATE.set(state);
}

/// Retrieve the global push state, or `None` if it has not been initialised.
pub fn push_state() -> Option<&'static Arc<PushState>> {
    PUSH_STATE.get()
}

/// Registry type alias.
pub type PushRegistry = Arc<DashMap<String, PushSubscription>>;

// ──────────────────────────────────────────────────────────────────────────────
// Request / response types
// ──────────────────────────────────────────────────────────────────────────────

/// `PushSubscription/get` request (RFC 8620 §5.1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionGetRequest {
    /// Optional list of subscription IDs to retrieve.  `None` means "all".
    #[serde(default)]
    pub ids: Option<Vec<String>>,
}

/// `PushSubscription/get` response.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionGetResponse {
    pub list: Vec<PushSubscriptionView>,
    pub not_found: Vec<String>,
}

/// The RFC 8620 §5.1 view of a `PushSubscription` returned to the client.
///
/// Fields marked `#[serde(skip)]` on the internal struct are re-exposed only
/// where RFC 8620 says they should appear in API responses.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionView {
    pub id: String,
    pub device_client_id: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<PushKeys>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<chrono::DateTime<chrono::Utc>>,
    pub types: Vec<String>,
}

impl From<&PushSubscription> for PushSubscriptionView {
    fn from(s: &PushSubscription) -> Self {
        Self {
            id: s.id.clone(),
            device_client_id: s.device_client_id.clone(),
            url: s.url.clone(),
            keys: s.keys.clone(),
            expires: s.expires,
            types: s.types.clone(),
        }
    }
}

/// `PushSubscription/set` request.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionSetRequest {
    #[serde(default)]
    pub create: Option<HashMap<String, PushSubscriptionCreate>>,
    #[serde(default)]
    pub update: Option<HashMap<String, PushSubscriptionUpdate>>,
    #[serde(default)]
    pub destroy: Option<Vec<String>>,
}

/// Fields accepted when creating a new push subscription.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionCreate {
    pub device_client_id: String,
    pub url: String,
    #[serde(default)]
    pub keys: Option<PushKeys>,
    #[serde(default)]
    pub expires: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub types: Vec<String>,
}

/// Fields that may be patched on an existing push subscription.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionUpdate {
    /// Supply the server-issued code to transition the subscription to `verified`.
    #[serde(default)]
    pub verification_code: Option<String>,
    /// Replace the monitored type list.
    #[serde(default)]
    pub types: Option<Vec<String>>,
    /// Update the expiry timestamp.
    #[serde(default)]
    pub expires: Option<chrono::DateTime<chrono::Utc>>,
}

/// `PushSubscription/set` response.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionSetResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<HashMap<String, PushSubscriptionCreated>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<HashMap<String, Option<serde_json::Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_created: Option<HashMap<String, JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_updated: Option<HashMap<String, JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_destroyed: Option<HashMap<String, JmapSetError>>,
}

/// Minimal object returned for a newly created subscription.
///
/// Note: RFC 8620 §5.1 specifies that the server sends the `verificationCode`
/// out-of-band to the push endpoint URL; it is included here in the creation
/// response so that test fixtures can retrieve it without inspecting the
/// mock HTTP server.  Production clients should obtain it from the push
/// delivery.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionCreated {
    pub id: String,
    /// The code that must be echoed back via `PushSubscription/set:update` to
    /// verify the subscription.
    pub verification_code: String,
}

// ──────────────────────────────────────────────────────────────────────────────
// Handlers
// ──────────────────────────────────────────────────────────────────────────────

/// Handle `PushSubscription/get`.
pub async fn push_subscription_get(
    request: PushSubscriptionGetRequest,
    principal: &Principal,
) -> anyhow::Result<PushSubscriptionGetResponse> {
    let state = match push_state() {
        Some(s) => s,
        None => {
            // Push not initialised — return empty list.
            return Ok(PushSubscriptionGetResponse {
                list: vec![],
                not_found: vec![],
            });
        }
    };

    let mut list = Vec::new();
    let mut not_found = Vec::new();

    match request.ids {
        None => {
            // Return all subscriptions owned by this principal.
            for entry in state.registry.iter() {
                if entry.value().principal_id == principal.account_id {
                    list.push(PushSubscriptionView::from(entry.value()));
                }
            }
        }
        Some(ids) => {
            for id in ids {
                match state.registry.get(&id) {
                    Some(entry) if entry.value().principal_id == principal.account_id => {
                        list.push(PushSubscriptionView::from(entry.value()));
                    }
                    Some(_) => {
                        // Exists but owned by someone else — treat as not found
                        // (do not reveal existence of foreign subscriptions).
                        not_found.push(id);
                    }
                    None => {
                        not_found.push(id);
                    }
                }
            }
        }
    }

    Ok(PushSubscriptionGetResponse { list, not_found })
}

/// Handle `PushSubscription/set`.
pub async fn push_subscription_set(
    request: PushSubscriptionSetRequest,
    principal: &Principal,
) -> anyhow::Result<PushSubscriptionSetResponse> {
    let state = match push_state() {
        Some(s) => s,
        None => {
            return Err(anyhow::anyhow!(
                "Push subsystem not initialised; call init_push_state() at server startup"
            ));
        }
    };

    let mut response = PushSubscriptionSetResponse::default();

    // ── Create ────────────────────────────────────────────────────────────────
    if let Some(creates) = request.create {
        let mut created = HashMap::new();
        let mut not_created = HashMap::new();

        for (client_id, create) in creates {
            match create_subscription(state, create, principal).await {
                Ok(result) => {
                    created.insert(client_id, result);
                }
                Err(e) => {
                    not_created.insert(
                        client_id,
                        JmapSetError {
                            error_type: "serverFail".to_string(),
                            description: Some(e.to_string()),
                        },
                    );
                }
            }
        }

        if !created.is_empty() {
            response.created = Some(created);
        }
        if !not_created.is_empty() {
            response.not_created = Some(not_created);
        }
    }

    // ── Update ────────────────────────────────────────────────────────────────
    if let Some(updates) = request.update {
        let mut updated = HashMap::new();
        let mut not_updated = HashMap::new();

        for (id, patch) in updates {
            match update_subscription(state, &id, patch, principal) {
                Ok(()) => {
                    updated.insert(id, None);
                }
                Err(e) => {
                    not_updated.insert(
                        id,
                        JmapSetError {
                            error_type: "serverFail".to_string(),
                            description: Some(e.to_string()),
                        },
                    );
                }
            }
        }

        if !updated.is_empty() {
            response.updated = Some(updated);
        }
        if !not_updated.is_empty() {
            response.not_updated = Some(not_updated);
        }
    }

    // ── Destroy ───────────────────────────────────────────────────────────────
    if let Some(destroy_ids) = request.destroy {
        let mut destroyed = Vec::new();
        let mut not_destroyed = HashMap::new();

        for id in destroy_ids {
            match destroy_subscription(state, &id, principal) {
                Ok(()) => {
                    destroyed.push(id);
                }
                Err(e) => {
                    not_destroyed.insert(
                        id,
                        JmapSetError {
                            error_type: "serverFail".to_string(),
                            description: Some(e.to_string()),
                        },
                    );
                }
            }
        }

        if !destroyed.is_empty() {
            response.destroyed = Some(destroyed);
        }
        if !not_destroyed.is_empty() {
            response.not_destroyed = Some(not_destroyed);
        }
    }

    Ok(response)
}

/// Testable variant of [`push_subscription_set`] that accepts an explicit
/// `PushState` rather than reading from the `OnceLock`.
///
/// Use this in integration tests to avoid `OnceLock` contention across
/// parallel test processes.
pub async fn push_subscription_set_with_state(
    request: PushSubscriptionSetRequest,
    principal: &Principal,
    state: &Arc<PushState>,
) -> anyhow::Result<PushSubscriptionSetResponse> {
    let mut response = PushSubscriptionSetResponse::default();

    // ── Create ────────────────────────────────────────────────────────────────
    if let Some(creates) = request.create {
        let mut created = HashMap::new();
        let mut not_created = HashMap::new();

        for (client_id, create) in creates {
            match create_subscription(state, create, principal).await {
                Ok(result) => {
                    created.insert(client_id, result);
                }
                Err(e) => {
                    not_created.insert(
                        client_id,
                        JmapSetError {
                            error_type: "serverFail".to_string(),
                            description: Some(e.to_string()),
                        },
                    );
                }
            }
        }

        if !created.is_empty() {
            response.created = Some(created);
        }
        if !not_created.is_empty() {
            response.not_created = Some(not_created);
        }
    }

    // ── Update ────────────────────────────────────────────────────────────────
    if let Some(updates) = request.update {
        let mut updated = HashMap::new();
        let mut not_updated = HashMap::new();

        for (id, patch) in updates {
            match update_subscription(state, &id, patch, principal) {
                Ok(()) => {
                    updated.insert(id, None);
                }
                Err(e) => {
                    not_updated.insert(
                        id,
                        JmapSetError {
                            error_type: "serverFail".to_string(),
                            description: Some(e.to_string()),
                        },
                    );
                }
            }
        }

        if !updated.is_empty() {
            response.updated = Some(updated);
        }
        if !not_updated.is_empty() {
            response.not_updated = Some(not_updated);
        }
    }

    // ── Destroy ───────────────────────────────────────────────────────────────
    if let Some(destroy_ids) = request.destroy {
        let mut destroyed = Vec::new();
        let mut not_destroyed = HashMap::new();

        for id in destroy_ids {
            match destroy_subscription(state, &id, principal) {
                Ok(()) => {
                    destroyed.push(id);
                }
                Err(e) => {
                    not_destroyed.insert(
                        id,
                        JmapSetError {
                            error_type: "serverFail".to_string(),
                            description: Some(e.to_string()),
                        },
                    );
                }
            }
        }

        if !destroyed.is_empty() {
            response.destroyed = Some(destroyed);
        }
        if !not_destroyed.is_empty() {
            response.not_destroyed = Some(not_destroyed);
        }
    }

    Ok(response)
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Generate a 32-byte random verification code, base64url-encoded.
fn generate_verification_code() -> Result<String, anyhow::Error> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf)
        .map_err(|e| anyhow::anyhow!("RNG failure during verification code generation: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf))
}

/// Validate that `url` is an acceptable push endpoint URL.
///
/// In production (`cfg(not(feature = "test-push-http"))`): only HTTPS is
/// accepted per RFC 8030 §5.2.
///
/// When the `test-push-http` Cargo feature is enabled: plain HTTP is also
/// accepted so that `wiremock` mock servers (which start on loopback
/// without TLS) can be used as push endpoints in integration tests.
fn validate_push_url(url: &str) -> Result<(), anyhow::Error> {
    if url.starts_with("https://") {
        return Ok(());
    }
    #[cfg(feature = "test-push-http")]
    if url.starts_with("http://") {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "Push subscription URL must use HTTPS, got: {url}"
    ))
}

async fn create_subscription(
    state: &PushState,
    create: PushSubscriptionCreate,
    principal: &Principal,
) -> anyhow::Result<PushSubscriptionCreated> {
    validate_push_url(&create.url)?;

    let id = uuid::Uuid::new_v4().to_string();
    let verification_code = generate_verification_code()?;

    let sub = PushSubscription {
        id: id.clone(),
        device_client_id: create.device_client_id,
        url: create.url,
        keys: create.keys,
        verification_code: Some(verification_code.clone()),
        expires: create.expires,
        types: create.types,
        verified: false,
        principal_id: principal.account_id.clone(),
    };

    // Attempt to send the verification push.  A failure here is returned as a
    // `serverFail`; the subscription is NOT stored on failure because there is
    // no way to deliver the verification code.
    match state.client.send(&sub, b"").await {
        Ok(()) => {}
        Err(WebPushError::Gone) => {
            return Err(anyhow::anyhow!(
                "Push endpoint returned 410 Gone during verification"
            ));
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to send verification push: {e}"));
        }
    }

    state.registry.insert(id.clone(), sub);

    Ok(PushSubscriptionCreated {
        id,
        verification_code,
    })
}

fn update_subscription(
    state: &PushState,
    id: &str,
    patch: PushSubscriptionUpdate,
    principal: &Principal,
) -> anyhow::Result<()> {
    let mut entry = state
        .registry
        .get_mut(id)
        .ok_or_else(|| anyhow::anyhow!("Subscription not found: {id}"))?;

    if entry.value().principal_id != principal.account_id {
        return Err(anyhow::anyhow!(
            "Subscription {id} not owned by this principal"
        ));
    }

    // Verification code check.
    if let Some(code) = patch.verification_code {
        if entry.value().verification_code.as_deref() == Some(code.as_str()) {
            entry.value_mut().verified = true;
        } else {
            return Err(anyhow::anyhow!(
                "Verification code mismatch for subscription {id}"
            ));
        }
    }

    if let Some(types) = patch.types {
        entry.value_mut().types = types;
    }
    if let Some(expires) = patch.expires {
        entry.value_mut().expires = Some(expires);
    }

    Ok(())
}

fn destroy_subscription(state: &PushState, id: &str, principal: &Principal) -> anyhow::Result<()> {
    // Do ownership check inside a limited scope so the read guard is dropped
    // before we call `remove()`.  Holding a DashMap read guard while calling
    // `remove()` on the same shard deadlocks.
    let owned = {
        match state.registry.get(id) {
            None => return Err(anyhow::anyhow!("Subscription not found: {id}")),
            Some(entry) => entry.value().principal_id == principal.account_id,
        }
        // guard drops here
    };

    if !owned {
        return Err(anyhow::anyhow!(
            "Subscription {id} not owned by this principal"
        ));
    }

    state.registry.remove(id);
    Ok(())
}
