//! Per-method account ownership tests for JMAP.
//!
//! Every method handler that accepts an `accountId` argument MUST reject
//! requests where the requested account does not belong to the authenticated
//! [`Principal`] (RFC 8621 §1.6 + RFC 8620 §3.6 conventions). These tests
//! exercise representative handlers across the dispatch surface.

use rusmes_core::transport::NullMailTransport;
use rusmes_jmap::methods::identity::FileIdentityStore;
use rusmes_jmap::methods::submission::FileSubmissionStore;
use rusmes_jmap::methods::vacation::FileVacationStore;
use rusmes_jmap::methods::{
    email::email_get, email::email_query, email::email_set, email_advanced::email_changes,
    email_advanced::email_copy, email_advanced::email_import, email_advanced::email_parse,
    email_advanced::email_query_changes, identity::identity_changes, identity::identity_get,
    identity::identity_set, mailbox::mailbox_changes, mailbox::mailbox_get, mailbox::mailbox_query,
    mailbox::mailbox_query_changes, mailbox::mailbox_set, search_snippet::search_snippet_get,
    submission::email_submission_changes, submission::email_submission_get,
    submission::email_submission_query, submission::email_submission_set,
    submission::SubmissionContext, thread::thread_changes, thread::thread_get,
    vacation::vacation_response_get, vacation::vacation_response_set,
};
use rusmes_jmap::methods::{
    email_advanced::{
        EmailChangesRequest, EmailCopyObject, EmailCopyRequest, EmailImportObject,
        EmailImportRequest, EmailParseRequest, EmailQueryChangesRequest,
    },
    identity::{IdentityChangesRequest, IdentityGetRequest, IdentitySetRequest},
    mailbox::{
        MailboxChangesRequest, MailboxGetRequest, MailboxQueryChangesRequest, MailboxQueryRequest,
        MailboxSetRequest,
    },
    search_snippet::SearchSnippetGetRequest,
    submission::{
        EmailSubmissionChangesRequest, EmailSubmissionGetRequest, EmailSubmissionQueryRequest,
        EmailSubmissionSetRequest,
    },
    thread::{ThreadChangesRequest, ThreadGetRequest},
    vacation::{VacationResponseGetRequest, VacationResponseSetRequest},
};
use rusmes_jmap::types::{
    EmailGetRequest, EmailQueryRequest, EmailSetRequest, JmapErrorType, Principal,
};
use rusmes_jmap::BlobStorage;
use rusmes_storage::{backends::filesystem::FilesystemBackend, MessageStore, StorageBackend};
use std::collections::HashMap;
use std::sync::Arc;

fn store() -> Arc<dyn MessageStore> {
    let backend = FilesystemBackend::new(std::env::temp_dir().join("rusmes-jmap-binding-test"));
    backend.message_store()
}

fn empty_blobs() -> BlobStorage {
    BlobStorage::new()
}

fn identity_store() -> FileIdentityStore {
    FileIdentityStore::new(std::env::temp_dir().join("rusmes-jmap-binding-identity-test"))
}

fn vacation_store() -> FileVacationStore {
    FileVacationStore::new(std::env::temp_dir().join("rusmes-jmap-binding-vacation-test"))
}

fn alice() -> Principal {
    // Owns ONLY "account-alice" — no admin scope.
    Principal {
        username: "alice".to_string(),
        account_id: "account-alice".to_string(),
        scopes: vec![],
    }
}

/// Assert that a returned `anyhow::Error` carries the JMAP `forbidden` error
/// type. `forbidden_error` from `methods::mod` boxes a `ForbiddenError` whose
/// Display includes the canonical URN; checking the URN is the contract.
fn assert_forbidden(err: anyhow::Error) {
    let msg = err.to_string();
    assert!(
        msg.contains(JmapErrorType::Forbidden.as_str()),
        "expected forbidden error, got: {}",
        msg
    );
}

#[tokio::test]
async fn email_get_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailGetRequest {
        account_id: "account-bob".to_string(),
        ids: Some(vec![]),
        properties: None,
        body_properties: None,
        fetch_text_body_values: None,
        fetch_html_body_values: None,
        fetch_all_body_values: None,
        max_body_value_bytes: None,
    };
    let err = email_get(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_set_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailSetRequest {
        account_id: "account-bob".to_string(),
        if_in_state: None,
        create: None,
        update: None,
        destroy: None,
    };
    let err = email_set(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_query_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailQueryRequest {
        account_id: "account-bob".to_string(),
        filter: None,
        sort: None,
        position: None,
        anchor: None,
        anchor_offset: None,
        limit: None,
        calculate_total: None,
    };
    let err = email_query(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_changes_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailChangesRequest {
        account_id: "account-bob".to_string(),
        since_state: "0".to_string(),
        max_changes: None,
    };
    let err = email_changes(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_query_changes_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailQueryChangesRequest {
        account_id: "account-bob".to_string(),
        since_query_state: "0".to_string(),
        filter: None,
        sort: None,
        max_changes: None,
        up_to_id: None,
        calculate_total: None,
    };
    let err = email_query_changes(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_copy_rejects_foreign_destination() {
    let principal = alice();
    let store = store();
    let req = EmailCopyRequest {
        from_account_id: "account-alice".to_string(),
        account_id: "account-bob".to_string(),
        if_from_in_state: None,
        if_in_state: None,
        create: HashMap::new(),
        on_success_destroy_original: None,
        destroy_from_if_in_state: None,
    };
    let err = email_copy(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_copy_rejects_foreign_source() {
    let principal = alice();
    let store = store();
    let mut create = HashMap::new();
    create.insert(
        "c1".to_string(),
        EmailCopyObject {
            id: "msg1".to_string(),
            mailbox_ids: HashMap::new(),
            keywords: None,
            received_at: None,
        },
    );
    let req = EmailCopyRequest {
        from_account_id: "account-bob".to_string(),
        account_id: "account-alice".to_string(),
        if_from_in_state: None,
        if_in_state: None,
        create,
        on_success_destroy_original: None,
        destroy_from_if_in_state: None,
    };
    let err = email_copy(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_import_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let mut emails = HashMap::new();
    emails.insert(
        "i1".to_string(),
        EmailImportObject {
            blob_id: "blob".to_string(),
            mailbox_ids: HashMap::new(),
            keywords: None,
            received_at: None,
        },
    );
    let req = EmailImportRequest {
        account_id: "account-bob".to_string(),
        if_in_state: None,
        emails,
    };
    let err = email_import(req, store.as_ref(), &empty_blobs(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_parse_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailParseRequest {
        account_id: "account-bob".to_string(),
        blob_ids: vec![],
        properties: None,
        body_properties: None,
        fetch_text_body_values: None,
        fetch_html_body_values: None,
        fetch_all_body_values: None,
        max_body_value_bytes: None,
    };
    let err = email_parse(req, store.as_ref(), &empty_blobs(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn mailbox_get_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = MailboxGetRequest {
        account_id: "account-bob".to_string(),
        ids: None,
        properties: None,
    };
    let err = mailbox_get(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn mailbox_set_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = MailboxSetRequest {
        account_id: "account-bob".to_string(),
        if_in_state: None,
        create: None,
        update: None,
        destroy: None,
        on_destroy_remove_emails: None,
    };
    let err = mailbox_set(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn mailbox_query_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = MailboxQueryRequest {
        account_id: "account-bob".to_string(),
        filter: None,
        sort: None,
        position: None,
        limit: None,
        calculate_total: None,
    };
    let err = mailbox_query(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn mailbox_changes_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = MailboxChangesRequest {
        account_id: "account-bob".to_string(),
        since_state: "0".to_string(),
        max_changes: None,
    };
    let err = mailbox_changes(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn mailbox_query_changes_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = MailboxQueryChangesRequest {
        account_id: "account-bob".to_string(),
        since_query_state: "0".to_string(),
        filter: None,
        sort: None,
        max_changes: None,
        up_to_id: None,
        calculate_total: None,
    };
    let err = mailbox_query_changes(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn thread_get_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = ThreadGetRequest {
        account_id: "account-bob".to_string(),
        ids: None,
        properties: None,
    };
    let err = thread_get(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn thread_changes_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = ThreadChangesRequest {
        account_id: "account-bob".to_string(),
        since_state: "0".to_string(),
        max_changes: None,
    };
    let err = thread_changes(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn search_snippet_get_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = SearchSnippetGetRequest {
        account_id: "account-bob".to_string(),
        email_ids: vec![],
        filter: None,
    };
    let err = search_snippet_get(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn identity_get_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let id_store = identity_store();
    let req = IdentityGetRequest {
        account_id: "account-bob".to_string(),
        ids: None,
        properties: None,
    };
    let err = identity_get(req, store.as_ref(), &id_store, &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn identity_set_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let id_store = identity_store();
    let req = IdentitySetRequest {
        account_id: "account-bob".to_string(),
        if_in_state: None,
        create: None,
        update: None,
        destroy: None,
    };
    let err = identity_set(req, store.as_ref(), &id_store, &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn identity_changes_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let id_store = identity_store();
    let req = IdentityChangesRequest {
        account_id: "account-bob".to_string(),
        since_state: "0".to_string(),
        max_changes: None,
    };
    let err = identity_changes(req, store.as_ref(), &id_store, &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn vacation_response_get_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let vstore = vacation_store();
    let req = VacationResponseGetRequest {
        account_id: "account-bob".to_string(),
        ids: None,
        properties: None,
    };
    let err = vacation_response_get(req, store.as_ref(), &principal, &vstore)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn vacation_response_set_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let vstore = vacation_store();
    let req = VacationResponseSetRequest {
        account_id: "account-bob".to_string(),
        if_in_state: None,
        update: None,
    };
    let err = vacation_response_set(req, store.as_ref(), &principal, &vstore)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_submission_get_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailSubmissionGetRequest {
        account_id: "account-bob".to_string(),
        ids: None,
        properties: None,
    };
    let err = email_submission_get(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_submission_set_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailSubmissionSetRequest {
        account_id: "account-bob".to_string(),
        if_in_state: None,
        create: None,
        update: None,
        destroy: None,
        on_success_update_email: None,
        on_success_destroy_email: None,
    };
    let sstore = FileSubmissionStore::new(std::env::temp_dir().join("rusmes-jmap-binding-test"));
    let istore = FileIdentityStore::new(std::env::temp_dir().join("rusmes-jmap-binding-test"));
    let transport = NullMailTransport;
    let ctx = SubmissionContext {
        message_store: store.as_ref(),
        submission_store: &sstore,
        identity_store: &istore,
        mail_transport: &transport,
    };
    let err = email_submission_set(req, &principal, &ctx)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_submission_query_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailSubmissionQueryRequest {
        account_id: "account-bob".to_string(),
        filter: None,
        sort: None,
        position: None,
        limit: None,
        calculate_total: None,
    };
    let err = email_submission_query(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

#[tokio::test]
async fn email_submission_changes_rejects_foreign_account() {
    let principal = alice();
    let store = store();
    let req = EmailSubmissionChangesRequest {
        account_id: "account-bob".to_string(),
        since_state: "0".to_string(),
        max_changes: None,
    };
    let err = email_submission_changes(req, store.as_ref(), &principal)
        .await
        .expect_err("should reject");
    assert_forbidden(err);
}

// ---------------------------------------------------------------------------
// Positive tests — owning principal succeeds.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn owning_principal_can_call_mailbox_get() {
    let principal = alice();
    let store = store();
    let req = MailboxGetRequest {
        account_id: "account-alice".to_string(),
        ids: None,
        properties: None,
    };
    let resp = mailbox_get(req, store.as_ref(), &principal)
        .await
        .expect("owning principal succeeds");
    assert_eq!(resp.account_id, "account-alice");
}
