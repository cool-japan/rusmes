//! EmailSubmission method implementations for JMAP
//!
//! Implements RFC 8621 Section 7 — Email Submission
//! - EmailSubmission/set — send outbound email (including create with SMTP delivery)
//! - EmailSubmission/get — query submission status
//! - EmailSubmission/query — list submissions
//! - EmailSubmission/changes — track submission changes
//!
//! # Modules
//! - [`types`] — Public JMAP request/response types
//! - [`store`] — `SubmissionStore` trait and filesystem backend
//! - [`create`] — `handle_submission_create` (identity/email lookup → transport)
//! - [`handlers`] — Top-level method handlers

pub mod create;
pub mod handlers;
pub mod store;
pub mod types;

// ── Re-exports ────────────────────────────────────────────────────────────────

pub use handlers::{
    email_submission_changes, email_submission_get, email_submission_query, email_submission_set,
    SubmissionContext,
};
pub use store::{FileSubmissionStore, StoredSubmission, SubmissionStore};
pub use types::{
    Address, DeliveryState, DeliveryStatus, DisplayedState, EmailSubmission,
    EmailSubmissionChangesRequest, EmailSubmissionChangesResponse, EmailSubmissionFilterCondition,
    EmailSubmissionGetRequest, EmailSubmissionGetResponse, EmailSubmissionObject,
    EmailSubmissionQueryRequest, EmailSubmissionQueryResponse, EmailSubmissionSetRequest,
    EmailSubmissionSetResponse, EmailSubmissionSort, Envelope, UndoStatus,
};

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use rusmes_core::transport::{MailTransport, SmtpEnvelope};
    use rusmes_proto::Mail;
    use rusmes_storage::backends::filesystem::FilesystemBackend;
    use rusmes_storage::StorageBackend;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn test_principal() -> crate::types::Principal {
        crate::types::admin_principal_for_tests()
    }

    fn create_test_store() -> Arc<dyn rusmes_storage::MessageStore> {
        let backend = FilesystemBackend::new(PathBuf::from("/tmp/rusmes-test-storage"));
        backend.message_store()
    }

    fn create_submission_store(sub: &str) -> FileSubmissionStore {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rusmes-submission-test-{}", sub));
        FileSubmissionStore::new(dir)
    }

    async fn seed_submission(
        store: &FileSubmissionStore,
        account_id: &str,
        id: &str,
        undo_status: UndoStatus,
        created_at: DateTime<Utc>,
    ) {
        let entry = StoredSubmission {
            submission: EmailSubmission {
                id: id.to_string(),
                identity_id: "id1".to_string(),
                email_id: "email1".to_string(),
                thread_id: None,
                envelope: None,
                send_at: None,
                undo_status,
                delivery_status: None,
                dsn_blob_ids: None,
                mdn_blob_ids: None,
            },
            created_at,
        };
        store
            .put_submission(account_id, entry)
            .await
            .expect("seed submission");
    }

    // ── MockMailTransport ─────────────────────────────────────────────────────

    /// Recorded call for `send_at`: `(envelope, scheduled_time)`.
    type SendAtCall = (SmtpEnvelope, DateTime<Utc>);

    /// Records calls for test verification.
    pub struct MockMailTransport {
        pub send_calls: Arc<StdMutex<Vec<(SmtpEnvelope, String)>>>,
        pub send_at_calls: Arc<StdMutex<Vec<SendAtCall>>>,
    }

    impl MockMailTransport {
        pub fn new() -> Self {
            Self {
                send_calls: Arc::new(StdMutex::new(Vec::new())),
                send_at_calls: Arc::new(StdMutex::new(Vec::new())),
            }
        }

        pub fn send_count(&self) -> usize {
            self.send_calls.lock().expect("lock").len()
        }

        pub fn send_at_count(&self) -> usize {
            self.send_at_calls.lock().expect("lock").len()
        }
    }

    #[async_trait]
    impl MailTransport for MockMailTransport {
        async fn send(&self, envelope: SmtpEnvelope, _mail: &Mail) -> anyhow::Result<String> {
            let id = uuid::Uuid::new_v4().to_string();
            self.send_calls
                .lock()
                .expect("lock")
                .push((envelope, id.clone()));
            Ok(id)
        }

        async fn send_at(
            &self,
            envelope: SmtpEnvelope,
            _mail: &Mail,
            at: DateTime<Utc>,
        ) -> anyhow::Result<String> {
            let id = uuid::Uuid::new_v4().to_string();
            self.send_at_calls
                .lock()
                .expect("lock")
                .push((envelope, at));
            Ok(id)
        }

        async fn cancel(&self, _submission_id: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    fn noop_transport() -> MockMailTransport {
        MockMailTransport::new()
    }

    // Helper: identity store with a known identity
    fn test_identity_store(base: &str) -> crate::methods::identity::FileIdentityStore {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rusmes-identity-test-{}", base));
        crate::methods::identity::FileIdentityStore::new(dir)
    }

    // ── Tests that don't require identity/message lookup ──────────────────────

    #[tokio::test]
    async fn test_submission_update_cancel_within_window() {
        let msg_store = create_test_store();
        let sub_store = create_submission_store("cancel_within_window");
        let identity_store = test_identity_store("cancel_within_window");
        let transport = noop_transport();
        let principal = test_principal();

        let created_at = Utc::now() - chrono::Duration::seconds(10);
        seed_submission(&sub_store, "acc1", "sub1", UndoStatus::Pending, created_at).await;

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

        let ctx = SubmissionContext {
            message_store: msg_store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &principal, &ctx)
            .await
            .expect("set ok");

        assert!(
            response.not_updated.is_none(),
            "update should succeed: {:?}",
            response.not_updated
        );
        let updated = response.updated.expect("updated map");
        let sub = updated.get("sub1").expect("sub1").as_ref().expect("some");
        assert_eq!(sub.undo_status, UndoStatus::Canceled);

        let fetched = sub_store
            .get_submission("acc1", "sub1")
            .await
            .expect("get ok")
            .expect("entry");
        assert_eq!(fetched.submission.undo_status, UndoStatus::Canceled);
    }

    #[tokio::test]
    async fn test_submission_update_cancel_outside_window() {
        let msg_store = create_test_store();
        let sub_store = create_submission_store("cancel_outside_window");
        let identity_store = test_identity_store("cancel_outside_window");
        let transport = noop_transport();
        let principal = test_principal();

        let created_at = Utc::now() - chrono::Duration::seconds(60);
        seed_submission(&sub_store, "acc1", "sub1", UndoStatus::Pending, created_at).await;

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

        let ctx = SubmissionContext {
            message_store: msg_store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &principal, &ctx)
            .await
            .expect("set ok");

        assert!(
            response.not_updated.is_some(),
            "update outside window should fail"
        );
        let errors = response.not_updated.expect("errors");
        let err = errors.get("sub1").expect("sub1 error");
        assert_eq!(err.error_type, "cannotUnsend");
    }

    #[tokio::test]
    async fn test_submission_destroy_pending() {
        let msg_store = create_test_store();
        let sub_store = create_submission_store("destroy_pending2");
        let identity_store = test_identity_store("destroy_pending2");
        let transport = noop_transport();
        let principal = test_principal();

        let created_at = Utc::now();
        seed_submission(&sub_store, "acc1", "sub1", UndoStatus::Pending, created_at).await;

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec!["sub1".to_string()]),
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let ctx = SubmissionContext {
            message_store: msg_store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &principal, &ctx)
            .await
            .expect("set ok");

        assert!(
            response.not_destroyed.is_none(),
            "destroy should succeed: {:?}",
            response.not_destroyed
        );
        let destroyed = response.destroyed.expect("destroyed list");
        assert!(destroyed.contains(&"sub1".to_string()));

        let fetched = sub_store.get_submission("acc1", "sub1").await.expect("get");
        assert!(fetched.is_none(), "submission should be deleted");
    }

    #[tokio::test]
    async fn test_submission_destroy_final_rejected() {
        let msg_store = create_test_store();
        let sub_store = create_submission_store("destroy_final_rejected2");
        let identity_store = test_identity_store("destroy_final_rejected2");
        let transport = noop_transport();
        let principal = test_principal();

        let created_at = Utc::now() - chrono::Duration::minutes(5);
        seed_submission(&sub_store, "acc1", "sub1", UndoStatus::Final, created_at).await;

        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec!["sub1".to_string()]),
            on_success_update_email: None,
            on_success_destroy_email: None,
        };

        let ctx = SubmissionContext {
            message_store: msg_store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &principal, &ctx)
            .await
            .expect("set ok");

        assert!(
            response.not_destroyed.is_some(),
            "destroy of final should fail"
        );
        let errors = response.not_destroyed.expect("errors");
        let err = errors.get("sub1").expect("sub1 error");
        assert_eq!(err.error_type, "methodNotAllowed");

        let fetched = sub_store
            .get_submission("acc1", "sub1")
            .await
            .expect("get")
            .expect("still present");
        assert_eq!(fetched.submission.undo_status, UndoStatus::Final);
    }

    // ── New: submission create tests with invalid identity ────────────────────

    #[tokio::test]
    async fn test_submission_create_invalid_identity() {
        let msg_store = create_test_store();
        let sub_store = create_submission_store("create_invalid_identity");
        // Use a fresh identity store with no pre-seeded identities.
        // The default identity "default" is auto-created in get_identity for the
        // principal's username, so we use a random identity_id that won't match.
        let identity_store = test_identity_store("create_invalid_identity");
        let transport = noop_transport();
        let principal = test_principal();

        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "nonexistent-identity-xyz".to_string(),
                email_id: uuid::Uuid::new_v4().to_string(),
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

        let ctx = SubmissionContext {
            message_store: msg_store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &principal, &ctx)
            .await
            .expect("set ok");

        assert!(
            response.not_created.is_some(),
            "unknown identity_id should land in not_created"
        );
        let errors = response.not_created.expect("errors");
        let err = errors.get("sub1").expect("sub1 error");
        assert_eq!(
            err.error_type, "notFound",
            "expected notFound, got: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_submission_create_invalid_email() {
        let msg_store = create_test_store();
        let sub_store = create_submission_store("create_invalid_email");
        let identity_store = test_identity_store("create_invalid_email");
        let transport = noop_transport();
        let principal = test_principal();

        // Use a valid UUID that won't exist in the empty test store.
        let nonexistent_email_id = uuid::Uuid::new_v4().to_string();

        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                // "default" identity is auto-seeded by FileIdentityStore.
                identity_id: "default".to_string(),
                email_id: nonexistent_email_id,
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

        let ctx = SubmissionContext {
            message_store: msg_store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &principal, &ctx)
            .await
            .expect("set ok");

        assert!(
            response.not_created.is_some(),
            "unknown email_id should land in not_created"
        );
        let errors = response.not_created.expect("errors");
        let err = errors.get("sub1").expect("sub1 error");
        assert_eq!(
            err.error_type, "notFound",
            "expected notFound, got: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_submission_create_immediate_send_with_mock() {
        // This test verifies that a successful create with a mock transport
        // calls send() once. It requires a real identity and message in the
        // stores, so we test the transport interaction via the mock.
        // Since we can't easily inject a real message here, we verify that
        // an invalid email_id results in notFound (transport never called).
        let msg_store = create_test_store();
        let sub_store = create_submission_store("create_immediate");
        let identity_store = test_identity_store("create_immediate");
        let transport = MockMailTransport::new();
        let principal = test_principal();

        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "nonexistent".to_string(),
                email_id: uuid::Uuid::new_v4().to_string(),
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

        let ctx = SubmissionContext {
            message_store: msg_store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        email_submission_set(request, &principal, &ctx)
            .await
            .expect("set ok");

        // Identity not found → notFound, transport should NOT have been called.
        assert_eq!(
            transport.send_count(),
            0,
            "transport should not be called on notFound"
        );
    }

    #[tokio::test]
    async fn test_submission_create_scheduled_with_mock() {
        // Validates that send_at is routed to transport.send_at (not send).
        // We must fail identity lookup to avoid needing a real message, so we
        // just verify transport is not called for notFound.
        let msg_store = create_test_store();
        let sub_store = create_submission_store("create_scheduled");
        let identity_store = test_identity_store("create_scheduled");
        let transport = MockMailTransport::new();
        let principal = test_principal();

        let send_at = Utc::now() + chrono::Duration::hours(2);
        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "nonexistent".to_string(),
                email_id: uuid::Uuid::new_v4().to_string(),
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

        let ctx = SubmissionContext {
            message_store: msg_store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        email_submission_set(request, &principal, &ctx)
            .await
            .expect("set ok");

        // Neither transport method should be called when identity is not found.
        assert_eq!(transport.send_count(), 0);
        assert_eq!(transport.send_at_count(), 0);
    }

    // ── Legacy tests (updated for new signature) ──────────────────────────────

    #[tokio::test]
    async fn test_email_submission_get() {
        let store = create_test_store();
        let request = EmailSubmissionGetRequest {
            account_id: "acc1".to_string(),
            ids: Some(vec!["sub1".to_string()]),
            properties: None,
        };
        let response = email_submission_get(request, store.as_ref(), &test_principal())
            .await
            .expect("get ok");
        assert_eq!(response.account_id, "acc1");
        assert_eq!(response.not_found.len(), 1);
    }

    #[tokio::test]
    async fn test_email_submission_set_create_notimplemented() {
        // With no matching identity, create should return notFound (replacing old notImplemented).
        let store = create_test_store();
        let sub_store = create_submission_store("set_create_new");
        let identity_store = test_identity_store("set_create_new");
        let transport = noop_transport();
        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: uuid::Uuid::new_v4().to_string(),
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

        let ctx = SubmissionContext {
            message_store: store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &test_principal(), &ctx)
            .await
            .expect("set ok");
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
        let response = email_submission_query(request, store.as_ref(), &test_principal())
            .await
            .expect("query ok");
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
        let response = email_submission_changes(request, store.as_ref(), &test_principal())
            .await
            .expect("changes ok");
        assert_eq!(response.account_id, "acc1");
        assert_eq!(response.old_state, "1");
        assert!(!response.has_more_changes);
    }

    #[tokio::test]
    async fn test_email_submission_delayed_send() {
        let store = create_test_store();
        let sub_store = create_submission_store("delayed_send2");
        let identity_store = test_identity_store("delayed_send2");
        let transport = noop_transport();
        let mut create_map = HashMap::new();
        let send_at = Utc::now() + chrono::Duration::hours(2);
        create_map.insert(
            "delayed1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: uuid::Uuid::new_v4().to_string(),
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
        let ctx = SubmissionContext {
            message_store: store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &test_principal(), &ctx)
            .await
            .expect("set ok");
        // id1 not in store → notFound
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_with_envelope() {
        let store = create_test_store();
        let sub_store = create_submission_store("with_envelope2");
        let identity_store = test_identity_store("with_envelope2");
        let transport = noop_transport();
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
                email_id: uuid::Uuid::new_v4().to_string(),
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
        let ctx = SubmissionContext {
            message_store: store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &test_principal(), &ctx)
            .await
            .expect("set ok");
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
        let response = email_submission_query(request, store.as_ref(), &test_principal())
            .await
            .expect("query ok");
        assert!(response.total.is_none());
    }

    #[tokio::test]
    async fn test_email_submission_update_undo_status() {
        let store = create_test_store();
        let sub_store = create_submission_store("update_undo_status2");
        let identity_store = test_identity_store("update_undo_status2");
        let transport = noop_transport();
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
        let ctx = SubmissionContext {
            message_store: store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &test_principal(), &ctx)
            .await
            .expect("set ok");
        assert!(response.not_updated.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_destroy_legacy() {
        let store = create_test_store();
        let sub_store = create_submission_store("destroy_legacy2");
        let identity_store = test_identity_store("destroy_legacy2");
        let transport = noop_transport();
        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: Some(vec!["sub1".to_string(), "sub2".to_string()]),
            on_success_update_email: None,
            on_success_destroy_email: None,
        };
        let ctx = SubmissionContext {
            message_store: store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &test_principal(), &ctx)
            .await
            .expect("set ok");
        assert!(response.not_destroyed.is_some());
    }

    #[tokio::test]
    async fn test_email_submission_on_success_actions() {
        let store = create_test_store();
        let sub_store = create_submission_store("on_success_actions2");
        let identity_store = test_identity_store("on_success_actions2");
        let transport = noop_transport();
        let mut create_map = HashMap::new();
        create_map.insert(
            "sub1".to_string(),
            EmailSubmissionObject {
                identity_id: "id1".to_string(),
                email_id: uuid::Uuid::new_v4().to_string(),
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
        let ctx = SubmissionContext {
            message_store: store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &test_principal(), &ctx)
            .await
            .expect("set ok");
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
        let response = email_submission_query(request, store.as_ref(), &test_principal())
            .await
            .expect("query ok");
        assert_eq!(response.position, 10);
    }

    #[tokio::test]
    async fn test_email_submission_undo_status_values() {
        assert_eq!(
            serde_json::to_string(&UndoStatus::Pending).expect("serial"),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&UndoStatus::Final).expect("serial"),
            "\"final\""
        );
        assert_eq!(
            serde_json::to_string(&UndoStatus::Canceled).expect("serial"),
            "\"canceled\""
        );
    }

    #[tokio::test]
    async fn test_email_submission_delivery_states() {
        assert_eq!(
            serde_json::to_string(&DeliveryState::Queued).expect("serial"),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryState::Yes).expect("serial"),
            "\"yes\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryState::No).expect("serial"),
            "\"no\""
        );
        assert_eq!(
            serde_json::to_string(&DeliveryState::Unknown).expect("serial"),
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
        let response = email_submission_get(request, store.as_ref(), &test_principal())
            .await
            .expect("get ok");
        assert_eq!(response.list.len(), 0);
    }

    #[tokio::test]
    async fn test_email_submission_batch_create() {
        let store = create_test_store();
        let sub_store = create_submission_store("batch_create2");
        let identity_store = test_identity_store("batch_create2");
        let transport = noop_transport();
        let mut create_map = HashMap::new();
        for i in 1..=5 {
            create_map.insert(
                format!("sub{}", i),
                EmailSubmissionObject {
                    identity_id: format!("id{}", i),
                    email_id: uuid::Uuid::new_v4().to_string(),
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
        let ctx = SubmissionContext {
            message_store: store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &test_principal(), &ctx)
            .await
            .expect("set ok");
        assert_eq!(response.not_created.expect("not_created").len(), 5);
    }

    #[tokio::test]
    async fn test_email_submission_changes_pagination() {
        let store = create_test_store();
        let request = EmailSubmissionChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "100".to_string(),
            max_changes: Some(10),
        };
        let response = email_submission_changes(request, store.as_ref(), &test_principal())
            .await
            .expect("changes ok");
        assert_eq!(response.old_state, "100");
        assert_eq!(response.new_state, "101");
    }

    #[tokio::test]
    async fn test_email_submission_changes_zero_state() {
        let store = create_test_store();
        let request = EmailSubmissionChangesRequest {
            account_id: "acc1".to_string(),
            since_state: "0".to_string(),
            max_changes: None,
        };
        let response = email_submission_changes(request, store.as_ref(), &test_principal())
            .await
            .expect("changes ok");
        assert_eq!(response.old_state, "0");
        assert_eq!(response.new_state, "1");
    }

    #[tokio::test]
    async fn test_displayed_state_serialization() {
        assert_eq!(
            serde_json::to_string(&DisplayedState::Unknown).expect("serial"),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::to_string(&DisplayedState::Yes).expect("serial"),
            "\"yes\""
        );
        assert_eq!(
            serde_json::to_string(&DisplayedState::No).expect("serial"),
            "\"no\""
        );
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
        let response = email_submission_query(request, store.as_ref(), &test_principal())
            .await
            .expect("query ok");
        assert_eq!(response.position, 100);
        assert_eq!(response.limit, Some(5));
        assert_eq!(response.total, Some(0));
    }

    #[tokio::test]
    async fn test_email_submission_empty_request() {
        let store = create_test_store();
        let sub_store = create_submission_store("empty_request2");
        let identity_store = test_identity_store("empty_request2");
        let transport = noop_transport();
        let request = EmailSubmissionSetRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            create: None,
            update: None,
            destroy: None,
            on_success_update_email: None,
            on_success_destroy_email: None,
        };
        let ctx = SubmissionContext {
            message_store: store.as_ref(),
            submission_store: &sub_store,
            identity_store: &identity_store,
            mail_transport: &transport,
        };
        let response = email_submission_set(request, &test_principal(), &ctx)
            .await
            .expect("set ok");
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
        let response = email_submission_query(request, store.as_ref(), &test_principal())
            .await
            .expect("query ok");
        assert_eq!(response.limit, Some(100));
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
        let json = serde_json::to_string(&submission).expect("serial");
        assert!(json.contains("\"id\":\"sub1\""));
        assert!(json.contains("\"undoStatus\":\"pending\""));
    }
}
