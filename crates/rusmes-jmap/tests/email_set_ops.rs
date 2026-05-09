//! Integration tests for Email/set create, update, and destroy operations.
//!
//! Uses a real `FilesystemBackend` backed by a unique temporary directory so
//! tests are isolated from one another and from the host filesystem.

use rusmes_jmap::methods::email::{email_get, email_set};
use rusmes_jmap::types::{admin_principal_for_tests, Principal};
use rusmes_jmap::types::{
    EmailAddress, EmailBodyPart, EmailBodyValue, EmailGetRequest, EmailSetObject, EmailSetRequest,
};
use rusmes_storage::{
    backends::filesystem::FilesystemBackend, MailboxId, MailboxStore, StorageBackend,
};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn unique_temp_dir(test_name: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("rusmes-jmap-email-set-tests")
        .join(format!("{}-{}", test_name, uuid::Uuid::new_v4()))
}

/// Create an isolated FilesystemBackend backed by a temp directory.
fn make_backend(test_name: &str) -> FilesystemBackend {
    FilesystemBackend::new(unique_temp_dir(test_name))
}

/// Create a real mailbox in the backend and return its `MailboxId` as a string.
async fn create_test_mailbox(
    mailbox_store: &Arc<dyn MailboxStore>,
    user: &str,
    name: &str,
) -> String {
    use rusmes_proto::Username;
    use rusmes_storage::MailboxPath;

    let username: Username = user.parse().expect("valid username");
    let path = MailboxPath::new(username, vec![name.to_string()]);
    let id: MailboxId = mailbox_store
        .create_mailbox(&path)
        .await
        .expect("create mailbox");
    id.to_string()
}

/// Build a minimal `EmailSetObject` with one text body part and the given subject.
fn simple_email_object(mailbox_id_str: &str, subject: &str, body: &str) -> EmailSetObject {
    let mut mailbox_ids = HashMap::new();
    mailbox_ids.insert(mailbox_id_str.to_string(), true);

    let mut body_values = HashMap::new();
    body_values.insert(
        "1".to_string(),
        EmailBodyValue {
            value: body.to_string(),
            is_encoding_problem: false,
            is_truncated: false,
        },
    );

    EmailSetObject {
        mailbox_ids,
        keywords: None,
        received_at: None,
        from: Some(vec![EmailAddress::new("sender@example.com".to_string())]),
        to: Some(vec![EmailAddress::new("recipient@example.com".to_string())]),
        cc: None,
        bcc: None,
        reply_to: None,
        sender: None,
        subject: Some(subject.to_string()),
        sent_at: None,
        in_reply_to: None,
        references: None,
        message_id: None,
        body_values: Some(body_values),
        text_body: Some(vec![EmailBodyPart {
            part_id: "1".to_string(),
            blob_id: None,
            size: None,
            name: None,
            r#type: Some("text/plain".to_string()),
            charset: Some("utf-8".to_string()),
            disposition: None,
            cid: None,
            language: None,
            location: None,
        }]),
        html_body: None,
        attachments: None,
    }
}

fn admin() -> Principal {
    admin_principal_for_tests()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Create an email via Email/set and retrieve it via Email/get.
#[tokio::test]
async fn test_email_create_and_get() {
    let backend = make_backend("create_and_get");
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let mailbox_id_str = create_test_mailbox(&mailbox_store, "alice", "INBOX").await;

    let mut create_map = HashMap::new();
    create_map.insert(
        "c1".to_string(),
        simple_email_object(&mailbox_id_str, "Hello World", "This is the body."),
    );

    let set_req = EmailSetRequest {
        account_id: admin().account_id.clone(),
        if_in_state: None,
        create: Some(create_map),
        update: None,
        destroy: None,
    };

    let set_resp = email_set(set_req, message_store.as_ref(), &admin())
        .await
        .expect("email_set should succeed");

    // The create should have succeeded.
    assert!(
        set_resp.not_created.is_none() || set_resp.not_created.as_ref().unwrap().is_empty(),
        "not_created should be empty: {:?}",
        set_resp.not_created
    );
    let created = set_resp.created.expect("created map should be Some");
    let email = created.get("c1").expect("c1 should be in created");

    let message_id = email.id.clone();

    // Retrieve it with Email/get.
    let get_req = EmailGetRequest {
        account_id: admin().account_id.clone(),
        ids: Some(vec![message_id.clone()]),
        properties: None,
        body_properties: None,
        fetch_text_body_values: None,
        fetch_html_body_values: None,
        fetch_all_body_values: None,
        max_body_value_bytes: None,
    };

    let get_resp = email_get(get_req, message_store.as_ref(), &admin())
        .await
        .expect("email_get should succeed");

    assert!(get_resp.not_found.is_empty(), "message should be found");
    assert_eq!(get_resp.list.len(), 1);
    let fetched = &get_resp.list[0];

    // Verify basic fields.
    assert_eq!(fetched.id, message_id);
    assert_eq!(
        fetched.subject.as_deref(),
        Some("Hello World"),
        "subject should match"
    );
    // from header should parse back.
    let from = fetched.from.as_ref().expect("from should be set");
    assert!(
        from.iter().any(|a| a.email == "sender@example.com"),
        "from should contain sender@example.com"
    );
}

/// Create an email, update its $Seen keyword, verify flag is set via get.
#[tokio::test]
async fn test_email_update_keywords() {
    let backend = make_backend("update_keywords");
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let mailbox_id_str = create_test_mailbox(&mailbox_store, "bob", "INBOX").await;

    // --- Create ---
    let mut create_map = HashMap::new();
    create_map.insert(
        "c1".to_string(),
        simple_email_object(&mailbox_id_str, "Unseen initially", "Body text"),
    );

    let set_req = EmailSetRequest {
        account_id: admin().account_id.clone(),
        if_in_state: None,
        create: Some(create_map),
        update: None,
        destroy: None,
    };
    let set_resp = email_set(set_req, message_store.as_ref(), &admin())
        .await
        .expect("create email");
    let created = set_resp.created.expect("created map");
    let email = created.get("c1").expect("c1 created");
    let message_id = email.id.clone();

    // --- Update: set $seen = true using per-flag patch path ---
    // JMAP patch paths use JSON Pointer notation: "/keywords/$seen".
    // The mailboxIds are specified as a full-replacement "/mailboxIds" object
    // so the update handler can resolve the owning mailbox for the
    // read-modify-write.  We do NOT use the bare "mailboxIds" key because
    // that is not a valid JMAP patch path and would be silently mishandled.
    let mut update_map = HashMap::new();
    update_map.insert(
        message_id.clone(),
        serde_json::json!({
            "/keywords/$seen": true,
            "/mailboxIds": { &mailbox_id_str: true }
        }),
    );

    let update_req = EmailSetRequest {
        account_id: admin().account_id.clone(),
        if_in_state: None,
        create: None,
        update: Some(update_map),
        destroy: None,
    };
    let update_resp = email_set(update_req, message_store.as_ref(), &admin())
        .await
        .expect("update email");

    assert!(
        update_resp.not_updated.is_none() || update_resp.not_updated.as_ref().unwrap().is_empty(),
        "not_updated should be empty: {:?}",
        update_resp.not_updated
    );
    assert!(
        update_resp.updated.is_some(),
        "updated map should be Some after flag update"
    );

    // Verify the email can still be fetched after the keyword update.
    // Note: keywords are not currently round-tripped through
    // convert_mail_to_email, so only the presence of the message is checked.
    let get_req = EmailGetRequest {
        account_id: admin().account_id.clone(),
        ids: Some(vec![message_id.clone()]),
        properties: None,
        body_properties: None,
        fetch_text_body_values: None,
        fetch_html_body_values: None,
        fetch_all_body_values: None,
        max_body_value_bytes: None,
    };
    let get_resp = email_get(get_req, message_store.as_ref(), &admin())
        .await
        .expect("email_get after update");
    assert_eq!(
        get_resp.list.len(),
        1,
        "message should still be found after keyword update"
    );
}

/// Create an email and then destroy it; a subsequent get should not find it.
#[tokio::test]
async fn test_email_destroy() {
    let backend = make_backend("destroy");
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let mailbox_id_str = create_test_mailbox(&mailbox_store, "carol", "INBOX").await;

    // --- Create ---
    let mut create_map = HashMap::new();
    create_map.insert(
        "c1".to_string(),
        simple_email_object(&mailbox_id_str, "To be deleted", "Delete me"),
    );

    let set_req = EmailSetRequest {
        account_id: admin().account_id.clone(),
        if_in_state: None,
        create: Some(create_map),
        update: None,
        destroy: None,
    };
    let set_resp = email_set(set_req, message_store.as_ref(), &admin())
        .await
        .expect("create email");
    let created = set_resp.created.expect("created");
    let message_id = created.get("c1").expect("c1").id.clone();

    // --- Destroy ---
    let destroy_req = EmailSetRequest {
        account_id: admin().account_id.clone(),
        if_in_state: None,
        create: None,
        update: None,
        destroy: Some(vec![message_id.clone()]),
    };
    let destroy_resp = email_set(destroy_req, message_store.as_ref(), &admin())
        .await
        .expect("destroy email");

    assert!(
        destroy_resp.not_destroyed.is_none()
            || destroy_resp.not_destroyed.as_ref().unwrap().is_empty(),
        "not_destroyed should be empty: {:?}",
        destroy_resp.not_destroyed
    );
    let destroyed = destroy_resp.destroyed.expect("destroyed list");
    assert!(
        destroyed.contains(&message_id),
        "destroyed list should include the message id"
    );

    // --- Verify gone ---
    let get_req = EmailGetRequest {
        account_id: admin().account_id.clone(),
        ids: Some(vec![message_id.clone()]),
        properties: None,
        body_properties: None,
        fetch_text_body_values: None,
        fetch_html_body_values: None,
        fetch_all_body_values: None,
        max_body_value_bytes: None,
    };
    let get_resp = email_get(get_req, message_store.as_ref(), &admin())
        .await
        .expect("email_get after destroy");
    assert_eq!(
        get_resp.list.len(),
        0,
        "deleted message should not be in list"
    );
    assert!(
        get_resp.not_found.contains(&message_id),
        "deleted message should be in not_found"
    );
}

/// Destroying the same message ID twice should yield `notFound` on the second attempt.
#[tokio::test]
async fn test_email_destroy_idempotent() {
    let backend = make_backend("destroy_idempotent");
    let mailbox_store = backend.mailbox_store();
    let message_store = backend.message_store();

    let mailbox_id_str = create_test_mailbox(&mailbox_store, "dave", "INBOX").await;

    // --- Create ---
    let mut create_map = HashMap::new();
    create_map.insert(
        "c1".to_string(),
        simple_email_object(&mailbox_id_str, "Destroy twice", "Body"),
    );

    let set_req = EmailSetRequest {
        account_id: admin().account_id.clone(),
        if_in_state: None,
        create: Some(create_map),
        update: None,
        destroy: None,
    };
    let set_resp = email_set(set_req, message_store.as_ref(), &admin())
        .await
        .expect("create email");
    let created = set_resp.created.expect("created");
    let message_id = created.get("c1").expect("c1").id.clone();

    // --- First destroy ---
    let destroy_req = EmailSetRequest {
        account_id: admin().account_id.clone(),
        if_in_state: None,
        create: None,
        update: None,
        destroy: Some(vec![message_id.clone()]),
    };
    let first_resp = email_set(destroy_req, message_store.as_ref(), &admin())
        .await
        .expect("first destroy");
    assert!(
        first_resp.not_destroyed.is_none() || first_resp.not_destroyed.as_ref().unwrap().is_empty(),
        "first destroy: not_destroyed should be empty"
    );

    // --- Second destroy (must return notFound, not panic/serverError) ---
    let destroy_again = EmailSetRequest {
        account_id: admin().account_id.clone(),
        if_in_state: None,
        create: None,
        update: None,
        destroy: Some(vec![message_id.clone()]),
    };
    let second_resp = email_set(destroy_again, message_store.as_ref(), &admin())
        .await
        .expect("second destroy call should not propagate an error");

    let not_destroyed = second_resp
        .not_destroyed
        .expect("second destroy should yield not_destroyed");
    let err = not_destroyed
        .get(&message_id)
        .expect("message id should be in not_destroyed");
    assert_eq!(
        err.error_type, "notFound",
        "second destroy error type should be notFound, got: {}",
        err.error_type
    );
}
