//! EmailSubmission method implementations for JMAP
//!
//! Implements RFC 8621 Section 7 - Email Submission
//! - EmailSubmission/set - send outbound email
//! - EmailSubmission/get - query submission status
//! - EmailSubmission/query - list submissions
//! - EmailSubmission/changes - track submission changes

use crate::types::JmapSetError;
use chrono::{DateTime, Utc};
use rusmes_storage::MessageStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Email submission object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmission {
    /// Unique identifier
    pub id: String,
    /// Identity ID for sender
    pub identity_id: String,
    /// Email ID being submitted
    pub email_id: String,
    /// Thread ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Envelope information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Envelope>,
    /// Send at time (for delayed send)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_at: Option<DateTime<Utc>>,
    /// Undo status
    pub undo_status: UndoStatus,
    /// Delivery status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_status: Option<HashMap<String, DeliveryStatus>>,
    /// DSN blob IDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsn_blob_ids: Option<Vec<String>>,
    /// MDN blob IDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mdn_blob_ids: Option<Vec<String>>,
}

/// Envelope for submission
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub mail_from: Address,
    pub rcpt_to: Vec<Address>,
}

/// Email address for envelope
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HashMap<String, Option<String>>>,
}

/// Undo status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UndoStatus {
    Pending,
    Final,
    Canceled,
}

/// Delivery status for a recipient
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryStatus {
    pub smtp_reply: String,
    pub delivered: DeliveryState,
    pub displayed: DisplayedState,
}

/// Delivery state enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryState {
    Queued,
    Yes,
    No,
    Unknown,
}

/// Displayed state enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisplayedState {
    Unknown,
    Yes,
    No,
}

/// EmailSubmission/get request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionGetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<String>>,
}

/// EmailSubmission/get response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionGetResponse {
    pub account_id: String,
    pub state: String,
    pub list: Vec<EmailSubmission>,
    pub not_found: Vec<String>,
}

/// EmailSubmission/set request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionSetRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_in_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create: Option<HashMap<String, EmailSubmissionObject>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroy: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_success_update_email: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_success_destroy_email: Option<Vec<String>>,
}

/// EmailSubmission object for creation
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionObject {
    pub identity_id: String,
    pub email_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Envelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_at: Option<DateTime<Utc>>,
}

/// EmailSubmission/set response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionSetResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<HashMap<String, EmailSubmission>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<HashMap<String, Option<EmailSubmission>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_created: Option<HashMap<String, JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_updated: Option<HashMap<String, JmapSetError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_destroyed: Option<HashMap<String, JmapSetError>>,
}

/// EmailSubmission/query request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionQueryRequest {
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<EmailSubmissionFilterCondition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<EmailSubmissionSort>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculate_total: Option<bool>,
}

/// EmailSubmission filter condition
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionFilterCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_status: Option<UndoStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<DateTime<Utc>>,
}

/// EmailSubmission sort comparator
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionSort {
    pub property: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_ascending: Option<bool>,
}

/// EmailSubmission/query response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionQueryResponse {
    pub account_id: String,
    pub query_state: String,
    pub can_calculate_changes: bool,
    pub position: i64,
    pub ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// EmailSubmission/changes request
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionChangesRequest {
    pub account_id: String,
    pub since_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_changes: Option<u64>,
}

/// EmailSubmission/changes response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSubmissionChangesResponse {
    pub account_id: String,
    pub old_state: String,
    pub new_state: String,
    pub has_more_changes: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}

/// Handle EmailSubmission/get method
pub async fn email_submission_get(
    request: EmailSubmissionGetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<EmailSubmissionGetResponse> {
    let list = Vec::new();
    let mut not_found = Vec::new();

    // If no IDs specified, return empty list
    let ids = request.ids.unwrap_or_default();

    for id in ids {
        // In production, would query submission database
        not_found.push(id);
    }

    Ok(EmailSubmissionGetResponse {
        account_id: request.account_id,
        state: "1".to_string(),
        list,
        not_found,
    })
}

/// Handle EmailSubmission/set method
#[allow(clippy::too_many_arguments)]
pub async fn email_submission_set(
    request: EmailSubmissionSetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<EmailSubmissionSetResponse> {
    let created = HashMap::new();
    let updated = HashMap::new();
    let destroyed = Vec::new();
    let mut not_created = HashMap::new();
    let mut not_updated = HashMap::new();
    let mut not_destroyed = HashMap::new();

    // Handle creates (send new emails)
    if let Some(create_map) = request.create {
        for (creation_id, _submission_obj) in create_map {
            // In production, would:
            // 1. Validate identity_id and email_id
            // 2. Check if sendAt is in the future (delayed send)
            // 3. Queue message for SMTP delivery
            // 4. Create EmailSubmission object with pending status
            // 5. Process onSuccessUpdateEmail if provided

            not_created.insert(
                creation_id,
                JmapSetError {
                    error_type: "notImplemented".to_string(),
                    description: Some("Email submission not yet implemented".to_string()),
                },
            );
        }
    }

    // Handle updates (e.g., cancel submission)
    if let Some(update_map) = request.update {
        for (id, _patch) in update_map {
            // In production, would allow updating undoStatus to cancel
            not_updated.insert(
                id,
                JmapSetError {
                    error_type: "notImplemented".to_string(),
                    description: Some("Submission update not yet implemented".to_string()),
                },
            );
        }
    }

    // Handle destroys (delete submission records)
    if let Some(destroy_ids) = request.destroy {
        for id in destroy_ids {
            not_destroyed.insert(
                id,
                JmapSetError {
                    error_type: "notImplemented".to_string(),
                    description: Some("Submission deletion not yet implemented".to_string()),
                },
            );
        }
    }

    Ok(EmailSubmissionSetResponse {
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

/// Handle EmailSubmission/query method
pub async fn email_submission_query(
    request: EmailSubmissionQueryRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<EmailSubmissionQueryResponse> {
    // In production, would query submission database based on filter
    let ids = Vec::new();

    let position = request.position.unwrap_or(0);
    let limit = request.limit.unwrap_or(100);

    Ok(EmailSubmissionQueryResponse {
        account_id: request.account_id,
        query_state: "1".to_string(),
        can_calculate_changes: false,
        position,
        ids,
        total: if request.calculate_total.unwrap_or(false) {
            Some(0)
        } else {
            None
        },
        limit: Some(limit),
    })
}

/// Handle EmailSubmission/changes method
pub async fn email_submission_changes(
    request: EmailSubmissionChangesRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<EmailSubmissionChangesResponse> {
    let since_state: u64 = request.since_state.parse().unwrap_or(0);
    let new_state = (since_state + 1).to_string();

    // In production, would query change log
    let created = Vec::new();
    let updated = Vec::new();
    let destroyed = Vec::new();

    Ok(EmailSubmissionChangesResponse {
        account_id: request.account_id,
        old_state: request.since_state,
        new_state,
        has_more_changes: false,
        created,
        updated,
        destroyed,
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
    async fn test_email_submission_get() {
        let store = create_test_store();
        let request = EmailSubmissionGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["sub1".to_string()]),
            properties: None,
        };

        let response = email_submission_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.account_id, "acc1");
        assert_eq!(response.not_found.len(), 1);
    }

    #[tokio::test]
    async fn test_email_submission_set_create() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: None,
                send_at: None,
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert_eq!(response.account_id, "acc1");
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_query() {
        let store = create_test_store();
        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: None,
            position: None,
            limit: Some(50),
            calculate_total: Some(true),
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.account_id, "acc1");
        assert_eq!(response.total, Some(0));
    }

    #[tokio::test]
    async fn test_email_submission_changes() {
        let store = create_test_store();
        let request = EmailSubmissionChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "1".to_string(),
            max_changes: Some(50),
        };

        let response = email_submission_changes(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.account_id, "acc1");
        assert_eq!(response.old_state, "1");
        assert!(!response.has_more_changes);
    }

    #[tokio::test]
    async fn test_email_submission_delayed_send() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let send_at = Utc::now() + chrono::Duration::hours(2);

        create_map.insert(
            "delayed1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: None,
                send_at: Some(send_at),
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_with_envelope() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let envelope = Envelope {
            mail_from: Address {
                email: "sender@example.com".to_string(),
                parameters: None,
            },
            rcpt_to: vec![Address {
                email: "recipient@example.com".to_string(),
                parameters: None,
            }],
        };

        create_map.insert(
            "env1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: Some(envelope),
                send_at: None,
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_query_with_filter() {
        let store = create_test_store();
        let filter = EmailSubmissionFilterCondition {
            identity_ids: Some(vec!["id1".to_string()]),
            email_ids: None,
            thread_ids: None,
            undo_status: Some(UndoStatus::Pending),
            before: None,
            after: None,
        };

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: Some(false),
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert!(response.total.is_none());
    }

    #[tokio::test]
    async fn test_email_submission_update_undo_status() {
        let store = create_test_store();
        let mut update_map = HashMap::new();
        update_map.insert(
            "sub1".to_string(),
            serde_json::json!({"undoStatus": "canceled"}),
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: Some(update_map),
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_updated.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_destroy() {
        let store = create_test_store();
        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec!["sub1".to_string(), "sub2".to_string()]),
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_destroyed.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_on_success_actions() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: None,
                send_at: None,
            },
        );

        let mut on_success_update = HashMap::new();
        on_success_update.insert(
            "email1".to_string(),
            serde_json::json!({"keywords/$sent": true}),
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: Some(on_success_update),
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_query_sort() {
        let store = create_test_store();
        let sort = vec![EmailSubmissionSort {
            property: "sendAt".to_string(),
            is_ascending: Some(false),
        }];

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: Some(sort),
            position: Some(10),
            limit: Some(25),
            calculate_total: None,
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.position, 10);
    }

    #[tokio::test]
    async fn test_email_submission_multiple_recipients() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let envelope = Envelope {
            mail_from: Address {
                email: "sender@example.com".to_string(),
                parameters: None,
            },
            rcpt_to: vec![
                Address {
                    email: "recipient1@example.com".to_string(),
                    parameters: None,
                },
                Address {
                    email: "recipient2@example.com".to_string(),
                    parameters: None,
                },
                Address {
                    email: "recipient3@example.com".to_string(),
                    parameters: None,
                },
            ],
        };

        create_map.insert(
            "multi1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: Some(envelope),
                send_at: None,
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_changes_pagination() {
        let store = create_test_store();
        let request = EmailSubmissionChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "100".to_string(),
            max_changes: Some(10),
        };

        let response = email_submission_changes(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.old_state, "100");
        assert_eq!(response.new_state, "101");
    }

    #[tokio::test]
    async fn test_email_submission_get_with_properties() {
        let store = create_test_store();
        let properties = vec![
            "id".to_string(),
            "identityId".to_string(),
            "emailId".to_string(),
            "undoStatus".to_string(),
            "deliveryStatus".to_string(),
        ];

        let request = EmailSubmissionGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["sub1".to_string()]),
            properties: Some(properties),
        };

        let response = email_submission_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 0);
    }

    #[tokio::test]
    async fn test_email_submission_query_date_range() {
        let store = create_test_store();
        let now = Utc::now();
        let filter = EmailSubmissionFilterCondition {
            identity_ids: None,
            email_ids: None,
            thread_ids: None,
            undo_status: None,
            before: Some(now),
            after: Some(now - chrono::Duration::days(7)),
        };

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: Some(true),
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.total, Some(0));
    }

    #[tokio::test]
    async fn test_email_submission_envelope_parameters() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let mut params = HashMap::new();
        params.insert("SIZE".to_string(), Some("1024".to_string()));
        params.insert("BODY".to_string(), Some("8BITMIME".to_string()));

        let envelope = Envelope {
            mail_from: Address {
                email: "sender@example.com".to_string(),
                parameters: Some(params),
            },
            rcpt_to: vec![Address {
                email: "recipient@example.com".to_string(),
                parameters: None,
            }],
        };

        create_map.insert(
            "params1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: Some(envelope),
                send_at: None,
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_if_in_state() {
        let store = create_test_store();
        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: Some("state123".to_string()),
            create: None,
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert_eq!(response.old_state, "1");
    }

    #[tokio::test]
    async fn test_email_submission_query_thread_filter() {
        let store = create_test_store();
        let filter = EmailSubmissionFilterCondition {
            identity_ids: None,
            email_ids: None,
            thread_ids: Some(vec!["thread1".to_string(), "thread2".to_string()]),
            undo_status: None,
            before: None,
            after: None,
        };

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.ids.len(), 0);
    }

    #[tokio::test]
    async fn test_email_submission_undo_status_values() {
        assert_eq!(
            serde_json::to_string(&UndoStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&UndoStatus::Final).unwrap(),
            "\"final\""
        );
        assert_eq!(
            serde_json::to_string(&UndoStatus::Canceled).unwrap(),
            "\"canceled\""
        );
    }

    #[tokio::test]
    async fn test_email_submission_delivery_states() {
        assert_eq!(
            serde_json::to_string(&DeliveryState::Queued).unwrap(),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryState::Yes).unwrap(),
            "\"yes\""
        );
        assert_eq!(serde_json::to_string(&DeliveryState::No).unwrap(), "\"no\"");
        assert_eq!(
            serde_json::to_string(&DeliveryState::Unknown).unwrap(),
            "\"unknown\""
        );
    }

    #[tokio::test]
    async fn test_email_submission_get_all() {
        let store = create_test_store();
        let request = EmailSubmissionGetRequest {
            account_id: "acc1".to_string(),
            ids: None,
            properties: None,
        };

        let response = email_submission_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.list.len(), 0);
    }

    #[tokio::test]
    async fn test_email_submission_batch_create() {
        let store = create_test_store();
        let mut create_map = HashMap::new();

        for i in 1..=10 {
            create_map.insert(
                format!("sub{}", i),
                EmailSubmissionObject {
                    identity_id: format!("id{}", i),
                    email_id: format!("email{}", i),
                    envelope: None,
                    send_at: None,
                },
            );
        }

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert_eq!(response.not_created.unwrap().len(), 10);
    }

    #[tokio::test]
    async fn test_email_submission_on_success_destroy() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "draft1".to_string(),
                envelope: None,
                send_at: None,
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: Some(vec!["draft1".to_string()]),
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_delayed_send_past() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let send_at = Utc::now() - chrono::Duration::hours(1);

        create_map.insert(
            "past1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: None,
                send_at: Some(send_at),
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_empty_envelope_recipients() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let envelope = Envelope {
            mail_from: Address {
                email: "sender@example.com".to_string(),
                parameters: None,
            },
            rcpt_to: vec![],
        };

        create_map.insert(
            "empty1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: Some(envelope),
                send_at: None,
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_query_email_ids_filter() {
        let store = create_test_store();
        let filter = EmailSubmissionFilterCondition {
            identity_ids: None,
            email_ids: Some(vec!["email1".to_string(), "email2".to_string()]),
            thread_ids: None,
            undo_status: None,
            before: None,
            after: None,
        };

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: Some(true),
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.total, Some(0));
    }

    #[tokio::test]
    async fn test_email_submission_query_final_status() {
        let store = create_test_store();
        let filter = EmailSubmissionFilterCondition {
            identity_ids: None,
            email_ids: None,
            thread_ids: None,
            undo_status: Some(UndoStatus::Final),
            before: None,
            after: None,
        };

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.ids.len(), 0);
    }

    #[tokio::test]
    async fn test_email_submission_query_canceled_status() {
        let store = create_test_store();
        let filter = EmailSubmissionFilterCondition {
            identity_ids: None,
            email_ids: None,
            thread_ids: None,
            undo_status: Some(UndoStatus::Canceled),
            before: None,
            after: None,
        };

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.ids.len(), 0);
    }

    #[tokio::test]
    async fn test_email_submission_mixed_operations() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "new1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: None,
                send_at: None,
            },
        );

        let mut update_map = HashMap::new();
        update_map.insert(
            "sub1".to_string(),
            serde_json::json!({"undoStatus": "canceled"}),
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: Some(update_map),
            destroy: Some(vec!["sub2".to_string()]),
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
        assert!(response.not_updated.is_some());
        assert!(response.not_destroyed.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_changes_zero_state() {
        let store = create_test_store();
        let request = EmailSubmissionChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "0".to_string(),
            max_changes: None,
        };

        let response = email_submission_changes(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.old_state, "0");
        assert_eq!(response.new_state, "1");
    }

    #[tokio::test]
    async fn test_displayed_state_serialization() {
        assert_eq!(
            serde_json::to_string(&DisplayedState::Unknown).unwrap(),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::to_string(&DisplayedState::Yes).unwrap(),
            "\"yes\""
        );
        assert_eq!(
            serde_json::to_string(&DisplayedState::No).unwrap(),
            "\"no\""
        );
    }

    #[tokio::test]
    async fn test_email_submission_get_multiple_ids() {
        let store = create_test_store();
        let request = EmailSubmissionGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec![
                "sub1".to_string(),
                "sub2".to_string(),
                "sub3".to_string(),
            ]),
            properties: None,
        };

        let response = email_submission_get(request, store.as_ref()).await.unwrap();
        assert_eq!(response.not_found.len(), 3);
    }

    #[tokio::test]
    async fn test_email_submission_query_multiple_sorts() {
        let store = create_test_store();
        let sort = vec![
            EmailSubmissionSort {
                property: "sendAt".to_string(),
                is_ascending: Some(false),
            },
            EmailSubmissionSort {
                property: "emailId".to_string(),
                is_ascending: Some(true),
            },
        ];

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: Some(sort),
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.ids.len(), 0);
    }

    #[tokio::test]
    async fn test_email_submission_query_complex_filter() {
        let store = create_test_store();
        let now = Utc::now();
        let filter = EmailSubmissionFilterCondition {
            identity_ids: Some(vec!["id1".to_string(), "id2".to_string()]),
            email_ids: Some(vec!["email1".to_string()]),
            thread_ids: Some(vec!["thread1".to_string()]),
            undo_status: Some(UndoStatus::Pending),
            before: Some(now),
            after: Some(now - chrono::Duration::days(30)),
        };

        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: Some(filter),
            sort: None,
            position: Some(5),
            limit: Some(10),
            calculate_total: Some(true),
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.position, 5);
        assert_eq!(response.limit, Some(10));
    }

    #[tokio::test]
    async fn test_email_submission_envelope_with_dsn_params() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let mut params = HashMap::new();
        params.insert("RET".to_string(), Some("FULL".to_string()));
        params.insert("ENVID".to_string(), Some("QQ314159".to_string()));

        let envelope = Envelope {
            mail_from: Address {
                email: "sender@example.com".to_string(),
                parameters: Some(params),
            },
            rcpt_to: vec![Address {
                email: "recipient@example.com".to_string(),
                parameters: None,
            }],
        };

        create_map.insert(
            "dsn1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: Some(envelope),
                send_at: None,
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_with_recipient_params() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let mut rcpt_params = HashMap::new();
        rcpt_params.insert("NOTIFY".to_string(), Some("SUCCESS,FAILURE".to_string()));

        let envelope = Envelope {
            mail_from: Address {
                email: "sender@example.com".to_string(),
                parameters: None,
            },
            rcpt_to: vec![Address {
                email: "recipient@example.com".to_string(),
                parameters: Some(rcpt_params),
            }],
        };

        create_map.insert(
            "notify1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: Some(envelope),
                send_at: None,
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_destroy_multiple() {
        let store = create_test_store();
        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec![
                "sub1".to_string(),
                "sub2".to_string(),
                "sub3".to_string(),
                "sub4".to_string(),
                "sub5".to_string(),
            ]),
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert_eq!(response.not_destroyed.unwrap().len(), 5);
    }

    #[tokio::test]
    async fn test_email_submission_query_position_and_limit() {
        let store = create_test_store();
        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: None,
            position: Some(100),
            limit: Some(5),
            calculate_total: Some(true),
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.position, 100);
        assert_eq!(response.limit, Some(5));
        assert_eq!(response.total, Some(0));
    }

    #[tokio::test]
    async fn test_email_submission_empty_request() {
        let store = create_test_store();
        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.created.is_none());
        assert!(response.updated.is_none());
        assert!(response.destroyed.is_none());
    }

    #[tokio::test]
    async fn test_email_submission_query_default_limit() {
        let store = create_test_store();
        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: None,
            position: None,
            limit: None,
            calculate_total: None,
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.limit, Some(100));
    }

    #[tokio::test]
    async fn test_email_submission_changes_with_max_changes() {
        let store = create_test_store();
        let request = EmailSubmissionChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "5".to_string(),
            max_changes: Some(100),
        };

        let response = email_submission_changes(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.old_state, "5");
        assert_eq!(response.new_state, "6");
        assert!(!response.has_more_changes);
    }

    #[tokio::test]
    async fn test_email_submission_on_success_combined() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "draft1".to_string(),
                envelope: None,
                send_at: None,
            },
        );

        let mut on_success_update = HashMap::new();
        on_success_update.insert(
            "draft1".to_string(),
            serde_json::json!({"keywords/$sent": true}),
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: Some(on_success_update),
            on_success_destroy_email: Some(vec!["draft2".to_string()]),
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_serialization() {
        let submission = EmailSubmission {
            id: "sub1".to_string(),
            identity_id: "id1".to_string(),
            email_id: "email1".to_string(),
            thread_id: Some("thread1".to_string()),
            envelope: None,
            send_at: None,
            undo_status: UndoStatus::Pending,
            delivery_status: None,
            dsn_blob_ids: None,
            mdn_blob_ids: None,
        };

        let json = serde_json::to_string(&submission).unwrap();
        assert!(json.contains("\"id\":\"sub1\""));
        assert!(json.contains("\"undoStatus\":\"pending\""));
    }

    #[tokio::test]
    async fn test_email_submission_with_all_fields() {
        let mut delivery = HashMap::new();
        delivery.insert(
            "recipient@example.com".to_string(),
            DeliveryStatus {
                smtp_reply: "250 OK".to_string(),
                delivered: DeliveryState::Yes,
                displayed: DisplayedState::Unknown,
            },
        );

        let submission = EmailSubmission {
            id: "sub1".to_string(),
            identity_id: "id1".to_string(),
            email_id: "email1".to_string(),
            thread_id: Some("thread1".to_string()),
            envelope: Some(Envelope {
                mail_from: Address {
                    email: "sender@example.com".to_string(),
                    parameters: None,
                },
                rcpt_to: vec![Address {
                    email: "recipient@example.com".to_string(),
                    parameters: None,
                }],
            }),
            send_at: Some(Utc::now()),
            undo_status: UndoStatus::Final,
            delivery_status: Some(delivery),
            dsn_blob_ids: Some(vec!["blob1".to_string()]),
            mdn_blob_ids: Some(vec!["blob2".to_string()]),
        };

        let json = serde_json::to_string(&submission).unwrap();
        assert!(json.contains("\"id\":\"sub1\""));
        assert!(json.contains("\"deliveryStatus\""));
    }

    #[tokio::test]
    async fn test_email_submission_query_zero_limit() {
        let store = create_test_store();
        let request = EmailSubmissionQueryRequest {
            account_id: "acc1".to_string(),
            filter: None,
            sort: None,
            position: None,
            limit: Some(0),
            calculate_total: Some(true),
        };

        let response = email_submission_query(request, store.as_ref())
            .await
            .unwrap();
        assert_eq!(response.limit, Some(0));
    }

    #[tokio::test]
    async fn test_email_submission_delayed_send_far_future() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let send_at = Utc::now() + chrono::Duration::days(365);

        create_map.insert(
            "future1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                envelope: None,
                send_at: Some(send_at),
            },
        );

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: Some(create_map),
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let response = email_submission_set(request, store.as_ref()).await.unwrap();
        assert!(response.not_created.is_some());
    }
}
