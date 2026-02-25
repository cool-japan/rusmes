//! Identity method implementations for JMAP
//!
//! Implements:
//! - Identity/get, Identity/set - sender identities
//! - Identity/changes - identity tracking

use crate::types::JmapSetError;
use rusmes_storage::MessageStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Handle Identity/get method
pub async fn identity_get(
    request: IdentityGetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<IdentityGetResponse> {
    let mut list = Vec::new();
    let mut not_found = Vec::new();

    // If no IDs specified, return default identity
    let ids = request.ids.unwrap_or_else(|| vec!["default".to_string()]);

    for id in ids {
        if id == "default" {
            // Return a default identity
            list.push(Identity {
                id: "default".to_string(),
                name: "Default User".to_string(),
                email: "user@example.com".to_string(),
                reply_to: None,
                bcc: None,
                text_signature: None,
                html_signature: None,
                may_delete: false,
            });
        } else {
            not_found.push(id);
        }
    }

    Ok(IdentityGetResponse {
        account_id: request.account_id,
        state: "1".to_string(),
        list,
        not_found,
    })
}

/// Handle Identity/set method
#[allow(clippy::too_many_arguments)]
pub async fn identity_set(
    request: IdentitySetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<IdentitySetResponse> {
    let created = HashMap::new();
    let updated = HashMap::new();
    let destroyed = Vec::new();
    let mut not_created = HashMap::new();
    let mut not_updated = HashMap::new();
    let mut not_destroyed = HashMap::new();

    // Handle creates
    if let Some(create_map) = request.create {
        for (creation_id, _identity_obj) in create_map {
            not_created.insert(
                creation_id,
                JmapSetError {
                    error_type: "notImplemented".to_string(),
                    description: Some("Identity creation not yet implemented".to_string()),
                },
            );
        }
    }

    // Handle updates
    if let Some(update_map) = request.update {
        for (id, _patch) in update_map {
            not_updated.insert(
                id,
                JmapSetError {
                    error_type: "notImplemented".to_string(),
                    description: Some("Identity update not yet implemented".to_string()),
                },
            );
        }
    }

    // Handle destroys
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
            } else {
                not_destroyed.insert(
                    id,
                    JmapSetError {
                        error_type: "notImplemented".to_string(),
                        description: Some("Identity deletion not yet implemented".to_string()),
                    },
                );
            }
        }
    }

    Ok(IdentitySetResponse {
        account_id: request.account_id,
        old_state: "1".to_string(),
        new_state: "2".to_string(),
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
) -> anyhow::Result<IdentityChangesResponse> {
    let since_state: u64 = request.since_state.parse().unwrap_or(0);
    let new_state = (since_state + 1).to_string();

    Ok(IdentityChangesResponse {
        account_id: request.account_id,
        old_state: request.since_state,
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

    fn create_test_store() -> std::sync::Arc<dyn MessageStore> {
        let backend = FilesystemBackend::new(PathBuf::from("/tmp/rusmes-test-storage"));
        backend.message_store()
    }

    #[tokio::test]
    async fn test_identity_get() {
        let store = create_test_store();
        let request = IdentityGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["default".to_string()]),
            properties: None,
        };

        let response = identity_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 1);
        assert_eq!(response.list[0].id, "default");
    }

    #[tokio::test]
    async fn test_identity_set_create() {
        let store = create_test_store();
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

        let response = identity_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_identity_changes() {
        let store = create_test_store();
        let request = IdentityChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "1".to_string(),
            max_changes: Some(50),
        };

        let response = identity_changes(request, store.as_ref()).await.unwrap();
        assert_eq!(response.old_state, "1");
        assert_eq!(response.new_state, "2");
    }

    #[tokio::test]
    async fn test_identity_set_destroy_default() {
        let store = create_test_store();
        let request = IdentitySetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec!["default".to_string()]),
        };

        let response = identity_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_destroyed.is_some());
        let errors = response.not_destroyed.unwrap();
        assert_eq!(errors.get("default").unwrap().error_type, "forbidden");
    }

    #[tokio::test]
    async fn test_identity_with_signature() {
        let store = create_test_store();
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

        let response = identity_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_identity_with_bcc() {
        let store = create_test_store();
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

        let response = identity_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_identity_get_not_found() {
        let store = create_test_store();
        let request = IdentityGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["nonexistent".to_string()]),
            properties: None,
        };

        let response = identity_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.not_found.len(), 1);
    }

    #[tokio::test]
    async fn test_identity_get_all() {
        let store = create_test_store();
        let request = IdentityGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = identity_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 1);
    }

    #[tokio::test]
    async fn test_identity_set_update() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert(
            "default".to_string(),
            serde_json::json!({"name": "New Name"}),
        );

        let request = IdentitySetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
        };

        let response = identity_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_updated.is_some());
    }

    #[tokio::test]
    async fn test_identity_changes_state_progression() {
        let store = create_test_store();

        let request1 = IdentityChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "5".to_string(),
            max_changes: None,
        };
        let response1 = identity_changes(request1, store.as_ref()).await.unwrap();

        let request2 = IdentityChangesRequest {
            account_id: "acc1".to_string(),
            since_state: response1.new_state.clone(),
            max_changes: None,
        };
        let response2 = identity_changes(request2, store.as_ref()).await.unwrap();

        assert!(response1.new_state < response2.new_state);
    }

    #[tokio::test]
    async fn test_identity_default_may_not_delete() {
        let store = create_test_store();
        let request = IdentityGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["default".to_string()]),
            properties: None,
        };

        let response = identity_get(request, store.as_ref()).await.unwrap();
        assert!(!response.list[0].may_delete);
    }

    #[tokio::test]
    async fn test_identity_with_reply_to() {
        let store = create_test_store();
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

        let response = identity_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }
}
