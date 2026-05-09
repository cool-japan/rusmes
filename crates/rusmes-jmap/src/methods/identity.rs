//! Identity method implementations for JMAP
//!
//! Implements:
//! - Identity/get, Identity/set - sender identities
//! - Identity/changes - identity tracking

use crate::methods::ensure_account_ownership;
use crate::types::{JmapSetError, Principal};
use async_trait::async_trait;
use rusmes_storage::MessageStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Persisted state for one account's identities
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IdentityAccountState {
    /// Map of identity id → Identity
    identities: HashMap<String, Identity>,
    /// Monotonic version counter; incremented on every mutation
    state_version: u64,
}

impl IdentityAccountState {
    fn new_with_default(account_id: &str, username: &str) -> Self {
        let default_email = if username.contains('@') {
            username.to_string()
        } else {
            format!("{}@localhost", account_id)
        };

        let mut identities = HashMap::new();
        identities.insert(
            "default".to_string(),
            Identity {
                id: "default".to_string(),
                name: "Default User".to_string(),
                email: default_email,
                reply_to: None,
                bcc: None,
                text_signature: None,
                html_signature: None,
                may_delete: false,
            },
        );

        Self {
            identities,
            state_version: 1,
        }
    }
}

/// Trait for identity persistence
#[async_trait]
pub trait IdentityStore: Send + Sync {
    /// Return all identities for an account (pre-populates default if absent)
    async fn list_identities(
        &self,
        account_id: &str,
        username: &str,
    ) -> anyhow::Result<Vec<Identity>>;

    /// Return one identity by id, or None
    async fn get_identity(
        &self,
        account_id: &str,
        username: &str,
        id: &str,
    ) -> anyhow::Result<Option<Identity>>;

    /// Create a new identity, returning the stored object
    async fn create_identity(
        &self,
        account_id: &str,
        username: &str,
        identity: Identity,
    ) -> anyhow::Result<Identity>;

    /// Update an identity via a flat JSON patch (top-level keys only), returning the updated object
    async fn update_identity(
        &self,
        account_id: &str,
        username: &str,
        id: &str,
        patch: &serde_json::Value,
    ) -> anyhow::Result<Identity>;

    /// Delete an identity by id; caller must reject "default" before calling
    async fn delete_identity(
        &self,
        account_id: &str,
        username: &str,
        id: &str,
    ) -> anyhow::Result<()>;

    /// Return the current state token for an account
    async fn state_token(&self, account_id: &str, username: &str) -> anyhow::Result<String>;
}

/// Filesystem-backed identity store.
///
/// Each account's identities are persisted to
/// `{base_dir}/identities/{account_id}.json`.
pub struct FileIdentityStore {
    base_dir: PathBuf,
}

impl FileIdentityStore {
    /// Create a new store rooted at `base_dir`.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn account_path(&self, account_id: &str) -> PathBuf {
        self.base_dir
            .join("identities")
            .join(format!("{}.json", account_id))
    }

    async fn load(&self, account_id: &str, username: &str) -> anyhow::Result<IdentityAccountState> {
        let path = self.account_path(account_id);
        if !path.exists() {
            return Ok(IdentityAccountState::new_with_default(account_id, username));
        }
        let bytes = tokio::fs::read(&path).await?;
        let state: IdentityAccountState = serde_json::from_slice(&bytes)?;
        Ok(state)
    }

    async fn save(&self, account_id: &str, state: &IdentityAccountState) -> anyhow::Result<()> {
        let path = self.account_path(account_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec_pretty(state)?;
        tokio::fs::write(&path, bytes).await?;
        Ok(())
    }
}

#[async_trait]
impl IdentityStore for FileIdentityStore {
    async fn list_identities(
        &self,
        account_id: &str,
        username: &str,
    ) -> anyhow::Result<Vec<Identity>> {
        let state = self.load(account_id, username).await?;
        Ok(state.identities.into_values().collect())
    }

    async fn get_identity(
        &self,
        account_id: &str,
        username: &str,
        id: &str,
    ) -> anyhow::Result<Option<Identity>> {
        let state = self.load(account_id, username).await?;
        Ok(state.identities.get(id).cloned())
    }

    async fn create_identity(
        &self,
        account_id: &str,
        username: &str,
        identity: Identity,
    ) -> anyhow::Result<Identity> {
        let mut state = self.load(account_id, username).await?;
        state
            .identities
            .insert(identity.id.clone(), identity.clone());
        state.state_version += 1;
        self.save(account_id, &state).await?;
        Ok(identity)
    }

    async fn update_identity(
        &self,
        account_id: &str,
        username: &str,
        id: &str,
        patch: &serde_json::Value,
    ) -> anyhow::Result<Identity> {
        let mut state = self.load(account_id, username).await?;
        let existing = state
            .identities
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Identity '{}' not found", id))?;

        // Serialize current identity to a mutable JSON value, apply the JMAP
        // patch (top-level "/fieldName" paths), then deserialize back.
        let mut current_json = serde_json::to_value(&existing)?;
        if let (Some(obj), Some(patch_obj)) = (current_json.as_object_mut(), patch.as_object()) {
            for (path_key, value) in patch_obj {
                // JMAP patch keys are "/fieldName"; strip leading '/'
                let field = path_key.trim_start_matches('/');
                obj.insert(field.to_string(), value.clone());
            }
        }
        let mut updated: Identity = serde_json::from_value(current_json)?;

        // Preserve immutable fields
        updated.id = existing.id.clone();
        if id == "default" {
            updated.may_delete = false;
        }

        state.identities.insert(id.to_string(), updated.clone());
        state.state_version += 1;
        self.save(account_id, &state).await?;
        Ok(updated)
    }

    async fn delete_identity(
        &self,
        account_id: &str,
        username: &str,
        id: &str,
    ) -> anyhow::Result<()> {
        let mut state = self.load(account_id, username).await?;
        if state.identities.remove(id).is_none() {
            return Err(anyhow::anyhow!("Identity '{}' not found", id));
        }
        state.state_version += 1;
        self.save(account_id, &state).await?;
        Ok(())
    }

    async fn state_token(&self, account_id: &str, username: &str) -> anyhow::Result<String> {
        let state = self.load(account_id, username).await?;
        Ok(state.state_version.to_string())
    }
}

// ─── JMAP types ──────────────────────────────────────────────────────────────

/// Identity object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    /// Unique identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Email address
    pub email: String,
    /// Reply-to address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Vec<crate::types::EmailAddress>>,
    /// Bcc address (auto-bcc on sends)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc: Option<Vec<crate::types::EmailAddress>>,
    /// Text signature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_signature: Option<String>,
    /// HTML signature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_signature: Option<String>,
    /// May delete
    pub may_delete: bool,
}

/// Identity/get request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityGetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
}

/// Identity/get response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityGetResponse {
    pub account_id: String,
    pub state: String,
    pub list: Vec<Identity>,
    pub not_found: Vec<String>,
}

/// Identity/set request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_in_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create: Option<HashMap<String, IdentityObject>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroy: Option<Vec<String>>,
}

/// Identity object for creation
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityObject {
    pub name: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<Vec<crate::types::EmailAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcc: Option<Vec<crate::types::EmailAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_signature: Option<String>,
}

/// Identity/set response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySetResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<HashMap<String, Identity>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<HashMap<String, Option<Identity>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_created: Option<HashMap<String, JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_updated: Option<HashMap<String, JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_destroyed: Option<HashMap<String, JmapSetError>>,
}

/// Identity/changes request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityChangesRequest {
    pub account_id: String,
    pub since_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_changes: Option<u64>,
}

/// Identity/changes response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityChangesResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    pub has_more_changes: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Minimal email format validation: must contain exactly one '@' with non-empty
/// local-part and domain.
fn is_valid_email(email: &str) -> bool {
    let at_count = email.chars().filter(|&c| c == '@').count();
    if at_count != 1 {
        return false;
    }
    let mut parts = email.splitn(2, '@');
    let local = parts.next().unwrap_or("");
    let domain = parts.next().unwrap_or("");
    !local.is_empty() && !domain.is_empty()
}

// ─── Method handlers ─────────────────────────────────────────────────────────

/// Handle Identity/get method
pub async fn identity_get(
    request: IdentityGetRequest,
    _message_store: &dyn MessageStore,
    identity_store: &dyn IdentityStore,
    principal: &Principal,
) -> anyhow::Result<IdentityGetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;

    let state = identity_store
        .state_token(&request.account_id, &principal.username)
        .await?;

    let mut list = Vec::new();
    let mut not_found = Vec::new();

    match request.ids {
        None => {
            let all = identity_store
                .list_identities(&request.account_id, &principal.username)
                .await?;
            list.extend(all);
        }
        Some(ids) => {
            for id in ids {
                match identity_store
                    .get_identity(&request.account_id, &principal.username, &id)
                    .await?
                {
                    Some(identity) => list.push(identity),
                    None => not_found.push(id),
                }
            }
        }
    }

    Ok(IdentityGetResponse {
        account_id: request.account_id,
        state,
        list,
        not_found,
    })
}

/// Handle Identity/set method
pub async fn identity_set(
    request: IdentitySetRequest,
    _message_store: &dyn MessageStore,
    identity_store: &dyn IdentityStore,
    principal: &Principal,
) -> anyhow::Result<IdentitySetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;

    let old_state = identity_store
        .state_token(&request.account_id, &principal.username)
        .await?;

    // Per RFC 8620 §5.3, if_in_state mismatch aborts the entire call
    if let Some(ref expected) = request.if_in_state {
        if *expected != old_state {
            return Err(anyhow::anyhow!(
                "stateMismatch: expected state '{}', current state is '{}'",
                expected,
                old_state
            ));
        }
    }

    let mut created: HashMap<String, Identity> = HashMap::new();
    let mut updated: HashMap<String, Option<Identity>> = HashMap::new();
    let mut destroyed: Vec<String> = Vec::new();
    let mut not_created: HashMap<String, JmapSetError> = HashMap::new();
    let mut not_updated: HashMap<String, JmapSetError> = HashMap::new();
    let mut not_destroyed: HashMap<String, JmapSetError> = HashMap::new();

    // ── Creates ──────────────────────────────────────────────────────────────
    if let Some(create_map) = request.create {
        for (creation_id, identity_obj) in create_map {
            if !is_valid_email(&identity_obj.email) {
                not_created.insert(
                    creation_id,
                    JmapSetError {
                        error_type: "invalidProperties".to_string(),
                        description: Some(format!(
                            "Invalid email address: '{}'",
                            identity_obj.email
                        )),
                    },
                );
                continue;
            }
            let new_id = uuid::Uuid::new_v4().to_string();
            let new_identity = Identity {
                id: new_id,
                name: identity_obj.name,
                email: identity_obj.email,
                reply_to: identity_obj.reply_to,
                bcc: identity_obj.bcc,
                text_signature: identity_obj.text_signature,
                html_signature: identity_obj.html_signature,
                may_delete: true,
            };
            match identity_store
                .create_identity(&request.account_id, &principal.username, new_identity)
                .await
            {
                Ok(stored) => {
                    created.insert(creation_id, stored);
                }
                Err(e) => {
                    not_created.insert(
                        creation_id,
                        JmapSetError {
                            error_type: "serverFail".to_string(),
                            description: Some(format!("Failed to create identity: {}", e)),
                        },
                    );
                }
            }
        }
    }

    // ── Updates ──────────────────────────────────────────────────────────────
    if let Some(update_map) = request.update {
        for (id, patch) in update_map {
            match identity_store
                .update_identity(&request.account_id, &principal.username, &id, &patch)
                .await
            {
                Ok(stored) => {
                    updated.insert(id, Some(stored));
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    let error_type = if err_msg.contains("not found") {
                        "notFound"
                    } else {
                        "serverFail"
                    };
                    not_updated.insert(
                        id,
                        JmapSetError {
                            error_type: error_type.to_string(),
                            description: Some(err_msg),
                        },
                    );
                }
            }
        }
    }

    // ── Destroys ─────────────────────────────────────────────────────────────
    if let Some(destroy_ids) = request.destroy {
        for id in destroy_ids {
            if id == "default" {
                not_destroyed.insert(
                    id,
                    JmapSetError {
                        error_type: "forbidden".to_string(),
                        description: Some("Cannot delete default identity".to_string()),
                    },
                );
                continue;
            }
            match identity_store
                .delete_identity(&request.account_id, &principal.username, &id)
                .await
            {
                Ok(()) => destroyed.push(id),
                Err(e) => {
                    let err_msg = e.to_string();
                    let error_type = if err_msg.contains("not found") {
                        "notFound"
                    } else {
                        "serverFail"
                    };
                    not_destroyed.insert(
                        id,
                        JmapSetError {
                            error_type: error_type.to_string(),
                            description: Some(err_msg),
                        },
                    );
                }
            }
        }
    }

    let new_state = identity_store
        .state_token(&request.account_id, &principal.username)
        .await?;

    Ok(IdentitySetResponse {
        account_id: request.account_id,
        old_state,
        new_state,
        created: if created.is_empty() {
            None
        } else {
            Some(created)
        },
        updated: if updated.is_empty() {
            None
        } else {
            Some(updated)
        },
        destroyed: if destroyed.is_empty() {
            None
        } else {
            Some(destroyed)
        },
        not_created: if not_created.is_empty() {
            None
        } else {
            Some(not_created)
        },
        not_updated: if not_updated.is_empty() {
            None
        } else {
            Some(not_updated)
        },
        not_destroyed: if not_destroyed.is_empty() {
            None
        } else {
            Some(not_destroyed)
        },
    })
}

/// Handle Identity/changes method
pub async fn identity_changes(
    request: IdentityChangesRequest,
    _message_store: &dyn MessageStore,
    identity_store: &dyn IdentityStore,
    principal: &Principal,
) -> anyhow::Result<IdentityChangesResponse> {
    ensure_account_ownership(&request.account_id, principal)?;

    let new_state = identity_store
        .state_token(&request.account_id, &principal.username)
        .await?;
    let old_state = request.since_state;

    // Slice A: we report the current state token but do not track per-object
    // change history. Callers should re-fetch all identities when the state
    // token differs from what they last saw.
    Ok(IdentityChangesResponse {
        account_id: request.account_id,
        old_state,
        new_state,
        has_more_changes: false,
        created: Vec::new(),
        updated: Vec::new(),
        destroyed: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusmes_storage::backends::filesystem::FilesystemBackend;
    use rusmes_storage::StorageBackend;
    use std::path::PathBuf;

    fn test_principal() -> crate::types::Principal {
        crate::types::Principal {
            username: "alice@example.com".to_string(),
            account_id: "acc1".to_string(),
            scopes: vec![crate::types::SCOPE_ADMIN.to_string()],
        }
    }

    fn create_test_store() -> std::sync::Arc<dyn MessageStore> {
        let backend = FilesystemBackend::new(PathBuf::from("/tmp/rusmes-test-storage"));
        backend.message_store()
    }

    fn create_identity_store(sub: &str) -> FileIdentityStore {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rusmes-identity-test-{}", sub));
        FileIdentityStore::new(dir)
    }

    // ── Required new tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_identity_create_and_get() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("create_and_get");
        let principal = test_principal();

        let mut create_map = HashMap::new();
        create_map.insert(
            "c1".to_string(),
            IdentityObject {
                name: "Alice".to_string(),
                email: "alice@example.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: None,
                html_signature: None,
            },
        );
        let set_resp = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: Some(create_map),
                update: None,
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();

        assert!(set_resp.not_created.is_none(), "create should succeed");
        let created = set_resp.created.unwrap();
        assert_eq!(created.len(), 1);
        let stored = created.get("c1").unwrap();
        assert_eq!(stored.name, "Alice");
        assert_eq!(stored.email, "alice@example.com");
        assert!(stored.may_delete);

        // Fetch it back by id
        let get_resp = identity_get(
            IdentityGetRequest {
                account_id: "acc1".to_string(),
                ids: Some(vec![stored.id.clone()]),
                properties: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        assert_eq!(get_resp.list.len(), 1);
        assert_eq!(get_resp.list[0].email, "alice@example.com");
    }

    #[tokio::test]
    async fn test_identity_update_name() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("update_name");
        let principal = test_principal();

        // Create first
        let mut create_map = HashMap::new();
        create_map.insert(
            "c1".to_string(),
            IdentityObject {
                name: "Original".to_string(),
                email: "orig@example.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: None,
                html_signature: None,
            },
        );
        let set_resp = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: Some(create_map),
                update: None,
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        let new_id = set_resp.created.unwrap().get("c1").unwrap().id.clone();

        // Update the name
        let mut update_map = HashMap::new();
        update_map.insert(new_id.clone(), serde_json::json!({"/name": "Updated Name"}));
        let upd_resp = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: None,
                update: Some(update_map),
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();

        assert!(upd_resp.not_updated.is_none(), "update should succeed");
        let upd = upd_resp.updated.unwrap();
        let id_obj = upd.get(&new_id).unwrap().as_ref().unwrap();
        assert_eq!(id_obj.name, "Updated Name");
    }

    #[tokio::test]
    async fn test_identity_destroy_custom() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("destroy_custom");
        let principal = test_principal();

        // Create an identity to destroy
        let mut create_map = HashMap::new();
        create_map.insert(
            "c1".to_string(),
            IdentityObject {
                name: "To Be Deleted".to_string(),
                email: "delete@example.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: None,
                html_signature: None,
            },
        );
        let set_resp = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: Some(create_map),
                update: None,
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        let new_id = set_resp.created.unwrap().get("c1").unwrap().id.clone();

        // Destroy it
        let del_resp = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: None,
                update: None,
                destroy: Some(vec![new_id.clone()]),
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();

        assert!(del_resp.not_destroyed.is_none(), "destroy should succeed");
        let destroyed = del_resp.destroyed.unwrap();
        assert!(destroyed.contains(&new_id));

        // Verify gone
        let get_resp = identity_get(
            IdentityGetRequest {
                account_id: "acc1".to_string(),
                ids: Some(vec![new_id.clone()]),
                properties: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        assert_eq!(get_resp.not_found, vec![new_id]);
    }

    #[tokio::test]
    async fn test_identity_destroy_default_rejected() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("destroy_default_rejected");
        let principal = test_principal();

        let resp = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: None,
                update: None,
                destroy: Some(vec!["default".to_string()]),
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();

        assert!(resp.not_destroyed.is_some());
        let errors = resp.not_destroyed.unwrap();
        assert_eq!(errors.get("default").unwrap().error_type, "forbidden");
    }

    #[tokio::test]
    async fn test_identity_state_mismatch() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("state_mismatch");
        let principal = test_principal();

        // Trigger a create to advance state past "1"
        let mut create_map = HashMap::new();
        create_map.insert(
            "c1".to_string(),
            IdentityObject {
                name: "Trigger".to_string(),
                email: "trigger@example.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: None,
                html_signature: None,
            },
        );
        identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: Some(create_map),
                update: None,
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();

        // Submit with wrong state
        let result = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: Some("999".to_string()),
                create: None,
                update: None,
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await;

        assert!(result.is_err(), "wrong state should return Err");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("stateMismatch"),
            "error should mention stateMismatch: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_identity_full_roundtrip() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("full_roundtrip");
        let principal = test_principal();

        // 1. Initial get — only default
        let get1 = identity_get(
            IdentityGetRequest {
                account_id: "acc1".to_string(),
                ids: None,
                properties: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        assert_eq!(get1.list.len(), 1);
        assert_eq!(get1.list[0].id, "default");
        assert!(!get1.list[0].may_delete);
        let state_after_default = get1.state.clone();

        // 2. Create a new identity
        let mut create_map = HashMap::new();
        create_map.insert(
            "newone".to_string(),
            IdentityObject {
                name: "Work".to_string(),
                email: "work@company.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: Some("Regards,\nAlice".to_string()),
                html_signature: None,
            },
        );
        let set1 = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: Some(state_after_default.clone()),
                create: Some(create_map),
                update: None,
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        assert!(set1.not_created.is_none());
        let work_id = set1.created.unwrap().get("newone").unwrap().id.clone();
        let state_after_create = set1.new_state.clone();
        assert_ne!(state_after_default, state_after_create);

        // 3. Update it
        let mut upd_map = HashMap::new();
        upd_map.insert(
            work_id.clone(),
            serde_json::json!({"/name": "Work (updated)"}),
        );
        let set2 = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: Some(state_after_create.clone()),
                create: None,
                update: Some(upd_map),
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        assert!(set2.not_updated.is_none());
        let upd_identity = set2
            .updated
            .unwrap()
            .get(&work_id)
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        assert_eq!(upd_identity.name, "Work (updated)");

        // 4. Destroy it
        let set3 = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: None,
                update: None,
                destroy: Some(vec![work_id.clone()]),
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        assert!(set3.not_destroyed.is_none());
        assert_eq!(set3.destroyed.unwrap(), vec![work_id.clone()]);

        // 5. Verify only default remains
        let get_final = identity_get(
            IdentityGetRequest {
                account_id: "acc1".to_string(),
                ids: None,
                properties: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        assert_eq!(get_final.list.len(), 1);
        assert_eq!(get_final.list[0].id, "default");
    }

    // ── Retained legacy tests (updated for new signatures) ───────────────────

    #[tokio::test]
    async fn test_identity_get() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("get_default");
        let principal = test_principal();
        let request = IdentityGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["default".to_string()]),
            properties: None,
        };

        let response = identity_get(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert_eq!(response.list.len(), 1);
        assert_eq!(response.list[0].id, "default");
    }

    #[tokio::test]
    async fn test_identity_set_create() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("set_create");
        let principal = test_principal();

        let mut create_map = HashMap::new();
        create_map.insert(
            "new1".to_string(),
            IdentityObject {
                name: "John Doe".to_string(),
                email: "john@example.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: Some("Best regards,\nJohn".to_string()),
                html_signature: None,
            },
        );

        let request = IdentitySetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
        };

        let response = identity_set(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert!(response.created.is_some());
        assert_eq!(response.created.as_ref().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_identity_changes() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("changes");
        let principal = test_principal();
        let request = IdentityChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "1".to_string(),
            max_changes: Some(50),
        };

        let response = identity_changes(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert_eq!(response.old_state, "1");
        assert!(!response.new_state.is_empty());
    }

    #[tokio::test]
    async fn test_identity_set_destroy_default() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("destroy_default");
        let principal = test_principal();
        let request = IdentitySetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec!["default".to_string()]),
        };

        let response = identity_set(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert!(response.not_destroyed.is_some());
        let errors = response.not_destroyed.unwrap();
        assert_eq!(errors.get("default").unwrap().error_type, "forbidden");
    }

    #[tokio::test]
    async fn test_identity_with_signature() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("with_signature");
        let principal = test_principal();

        let mut create_map = HashMap::new();
        create_map.insert(
            "sig1".to_string(),
            IdentityObject {
                name: "Test User".to_string(),
                email: "test@example.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: Some("--\nBest regards".to_string()),
                html_signature: Some("<p>Best regards</p>".to_string()),
            },
        );

        let request = IdentitySetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
        };

        let response = identity_set(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert!(response.created.is_some());
        let stored = response.created.as_ref().unwrap().get("sig1").unwrap();
        assert_eq!(stored.text_signature.as_deref(), Some("--\nBest regards"));
    }

    #[tokio::test]
    async fn test_identity_with_bcc() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("with_bcc");
        let principal = test_principal();

        let mut create_map = HashMap::new();
        let bcc = vec![crate::types::EmailAddress::new(
            "archive@example.com".to_string(),
        )];

        create_map.insert(
            "bcc1".to_string(),
            IdentityObject {
                name: "Test User".to_string(),
                email: "test@example.com".to_string(),
                reply_to: None,
                bcc: Some(bcc),
                text_signature: None,
                html_signature: None,
            },
        );

        let request = IdentitySetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
        };

        let response = identity_set(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert!(response.created.is_some());
        let stored = response.created.as_ref().unwrap().get("bcc1").unwrap();
        assert!(stored.bcc.is_some());
    }

    #[tokio::test]
    async fn test_identity_get_not_found() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("get_not_found");
        let principal = test_principal();
        let request = IdentityGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["nonexistent".to_string()]),
            properties: None,
        };

        let response = identity_get(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert_eq!(response.not_found.len(), 1);
    }

    #[tokio::test]
    async fn test_identity_get_all() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("get_all");
        let principal = test_principal();
        let request = IdentityGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = identity_get(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert!(!response.list.is_empty());
    }

    #[tokio::test]
    async fn test_identity_set_update() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("set_update_default");
        let principal = test_principal();

        let mut update_map = HashMap::new();
        update_map.insert(
            "default".to_string(),
            serde_json::json!({"/name": "New Name"}),
        );

        let request = IdentitySetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
        };

        let response = identity_set(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        // Default can be updated (only destroy is forbidden)
        assert!(response.not_updated.is_none());
        let upd = response.updated.unwrap();
        let id_obj = upd.get("default").unwrap().as_ref().unwrap();
        assert_eq!(id_obj.name, "New Name");
    }

    #[tokio::test]
    async fn test_identity_changes_state_progression() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("changes_state_progression");
        let principal = test_principal();

        // Create one identity to advance the version
        let mut create_map = HashMap::new();
        create_map.insert(
            "c1".to_string(),
            IdentityObject {
                name: "Test".to_string(),
                email: "test@example.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: None,
                html_signature: None,
            },
        );
        identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: Some(create_map),
                update: None,
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();

        let request1 = IdentityChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "1".to_string(),
            max_changes: None,
        };
        let response1 = identity_changes(request1, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();

        let new_state_num: u64 = response1.new_state.parse().unwrap();
        assert!(new_state_num > 1, "state should have advanced beyond 1");

        // Calling changes again with the returned new_state yields same state
        let request2 = IdentityChangesRequest {
            account_id: "acc1".to_string(),
            since_state: response1.new_state.clone(),
            max_changes: None,
        };
        let response2 = identity_changes(request2, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert_eq!(response1.new_state, response2.new_state);
    }

    #[tokio::test]
    async fn test_identity_default_may_not_delete() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("default_may_not_delete");
        let principal = test_principal();
        let request = IdentityGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["default".to_string()]),
            properties: None,
        };

        let response = identity_get(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert!(!response.list[0].may_delete);
    }

    #[tokio::test]
    async fn test_identity_with_reply_to() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("with_reply_to");
        let principal = test_principal();

        let mut create_map = HashMap::new();
        let reply_to = vec![crate::types::EmailAddress::new(
            "support@example.com".to_string(),
        )];

        create_map.insert(
            "replyto1".to_string(),
            IdentityObject {
                name: "Support".to_string(),
                email: "noreply@example.com".to_string(),
                reply_to: Some(reply_to),
                bcc: None,
                text_signature: None,
                html_signature: None,
            },
        );

        let request = IdentitySetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
        };

        let response = identity_set(request, msg_store.as_ref(), &id_store, &principal)
            .await
            .unwrap();
        assert!(response.created.is_some());
        let stored = response.created.as_ref().unwrap().get("replyto1").unwrap();
        assert!(stored.reply_to.is_some());
    }

    #[tokio::test]
    async fn test_identity_invalid_email_rejected() {
        let msg_store = create_test_store();
        let id_store = create_identity_store("invalid_email");
        let principal = test_principal();

        let mut create_map = HashMap::new();
        create_map.insert(
            "bad1".to_string(),
            IdentityObject {
                name: "Bad".to_string(),
                email: "not-an-email".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: None,
                html_signature: None,
            },
        );

        let response = identity_set(
            IdentitySetRequest {
                account_id: "acc1".to_string(),
                if_in_state: None,
                create: Some(create_map),
                update: None,
                destroy: None,
            },
            msg_store.as_ref(),
            &id_store,
            &principal,
        )
        .await
        .unwrap();
        assert!(response.not_created.is_some());
        let err = response.not_created.unwrap();
        assert_eq!(err.get("bad1").unwrap().error_type, "invalidProperties");
    }
}
