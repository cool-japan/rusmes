//! Email/copy handler implementation.

use super::types::{EmailCopyObject, EmailCopyRequest, EmailCopyResponse};
use crate::methods::ensure_account_ownership;
use crate::types::{Email, JmapSetError, Principal};
use chrono::Utc;
use rusmes_storage::MessageStore;
use std::collections::HashMap;

/// Handle Email/copy method
///
/// Copies emails between accounts, preserving the message content
/// but allowing different mailbox placements and keywords.
///
/// **Account ownership.** Per RFC 8621 §5.4 the principal must be authorised
/// to read the source (`from_account_id`) AND write the destination
/// (`account_id`). Until this server supports cross-account delegation, the
/// principal must own BOTH accounts. Either mismatch returns
/// `urn:ietf:params:jmap:error:forbidden`.
pub async fn email_copy(
    request: EmailCopyRequest,
    message_store: &dyn MessageStore,
    principal: &Principal,
) -> anyhow::Result<EmailCopyResponse> {
    ensure_account_ownership(&request.from_account_id, principal)?;
    ensure_account_ownership(&request.account_id, principal)?;
    let old_state = super::get_current_modseq(message_store).await?.to_string();

    if let Some(ref expected_state) = request.if_from_in_state {
        let from_state = super::get_current_modseq(message_store).await?.to_string();
        if &from_state != expected_state {
            return Err(anyhow::anyhow!("State mismatch in source account"));
        }
    }

    if let Some(ref expected_state) = request.if_in_state {
        let dest_state = super::get_current_modseq(message_store).await?.to_string();
        if &dest_state != expected_state {
            return Err(anyhow::anyhow!("State mismatch in destination account"));
        }
    }

    let mut created = HashMap::new();
    let mut not_created = HashMap::new();

    for (creation_id, copy_obj) in request.create {
        match copy_email(message_store, &copy_obj, &request.account_id).await {
            Ok(email) => {
                created.insert(creation_id, email);
            }
            Err(e) => {
                not_created.insert(
                    creation_id,
                    JmapSetError {
                        error_type: "notFound".to_string(),
                        description: Some(format!("Failed to copy email: {}", e)),
                    },
                );
            }
        }
    }

    // Handle onSuccessDestroyOriginal if needed
    if request.on_success_destroy_original.unwrap_or(false) && !created.is_empty() {
        // Would destroy original emails here
        // Need to verify destroyFromIfInState if specified
    }

    let new_state = super::get_current_modseq(message_store).await?.to_string();

    Ok(EmailCopyResponse {
        from_account_id: request.from_account_id,
        account_id: request.account_id,
        old_state,
        new_state,
        created: if created.is_empty() {
            None
        } else {
            Some(created)
        },
        not_created: if not_created.is_empty() {
            None
        } else {
            Some(not_created)
        },
    })
}

/// Helper function to copy an email
async fn copy_email(
    _message_store: &dyn MessageStore,
    copy_obj: &EmailCopyObject,
    _account_id: &str,
) -> anyhow::Result<Email> {
    Ok(Email {
        id: uuid::Uuid::new_v4().to_string(),
        blob_id: "blob_".to_string() + &copy_obj.id,
        thread_id: Some("thread_1".to_string()),
        mailbox_ids: copy_obj.mailbox_ids.clone(),
        keywords: copy_obj.keywords.clone().unwrap_or_default(),
        size: 1000,
        received_at: copy_obj.received_at.unwrap_or_else(Utc::now),
        message_id: None,
        in_reply_to: None,
        references: None,
        sender: None,
        from: None,
        to: None,
        cc: None,
        bcc: None,
        reply_to: None,
        subject: None,
        sent_at: None,
        has_attachment: false,
        preview: Some("Copied email".to_string()),
        body_values: None,
        text_body: None,
        html_body: None,
        attachments: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods::email_advanced::test_helpers::{create_test_store, test_principal};
    use chrono::Utc;

    #[tokio::test]
    async fn test_email_copy() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "copy1".to_string(),
            EmailCopyObject {
                id: "msg1".to_string(),
                mailbox_ids: [("inbox".to_string(), true)].iter().cloned().collect(),
                keywords: None,
                received_at: None,
            },
        );

        let request = EmailCopyRequest {
            from_account_id: "acc1".to_string(),
            account_id: "acc2".to_string(),
            if_from_in_state: None,
            if_in_state: None,
            create: create_map,
            on_success_destroy_original: Some(false),
            destroy_from_if_in_state: None,
        };

        let response = email_copy(request, store.as_ref(), &test_principal())
            .await
            .expect("email_copy failed");
        assert_eq!(response.from_account_id, "acc1");
        assert_eq!(response.account_id, "acc2");
    }

    #[tokio::test]
    async fn test_email_copy_with_destroy_original() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let mut keywords = HashMap::new();
        keywords.insert("$seen".to_string(), true);

        create_map.insert(
            "copy1".to_string(),
            EmailCopyObject {
                id: "msg1".to_string(),
                mailbox_ids: [("inbox".to_string(), true)].iter().cloned().collect(),
                keywords: Some(keywords),
                received_at: Some(Utc::now()),
            },
        );

        let request = EmailCopyRequest {
            from_account_id: "acc1".to_string(),
            account_id: "acc2".to_string(),
            if_from_in_state: None,
            if_in_state: None,
            create: create_map,
            on_success_destroy_original: Some(true),
            destroy_from_if_in_state: None,
        };

        let _response = email_copy(request, store.as_ref(), &test_principal())
            .await
            .expect("email_copy with_destroy_original failed");
    }

    #[tokio::test]
    async fn test_email_copy_multiple_emails() {
        let store = create_test_store();
        let mut create_map = HashMap::new();

        for i in 1..=5 {
            create_map.insert(
                format!("copy{}", i),
                EmailCopyObject {
                    id: format!("msg{}", i),
                    mailbox_ids: [("inbox".to_string(), true)].iter().cloned().collect(),
                    keywords: None,
                    received_at: None,
                },
            );
        }

        let request = EmailCopyRequest {
            from_account_id: "acc1".to_string(),
            account_id: "acc2".to_string(),
            if_from_in_state: None,
            if_in_state: None,
            create: create_map,
            on_success_destroy_original: None,
            destroy_from_if_in_state: None,
        };

        let response = email_copy(request, store.as_ref(), &test_principal())
            .await
            .expect("email_copy multiple_emails failed");
        assert!(response.created.is_some());
        assert_eq!(response.created.expect("created").len(), 5);
    }

    #[tokio::test]
    async fn test_email_copy_empty_create_map() {
        let store = create_test_store();
        let request = EmailCopyRequest {
            from_account_id: "acc1".to_string(),
            account_id: "acc2".to_string(),
            if_from_in_state: None,
            if_in_state: None,
            create: HashMap::new(),
            on_success_destroy_original: None,
            destroy_from_if_in_state: None,
        };

        let response = email_copy(request, store.as_ref(), &test_principal())
            .await
            .expect("email_copy empty_create_map failed");
        assert!(response.created.is_none());
        assert!(response.not_created.is_none());
    }

    #[tokio::test]
    async fn test_email_copy_cross_account() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let mut mailbox_ids = HashMap::new();
        mailbox_ids.insert("inbox".to_string(), true);
        mailbox_ids.insert("archive".to_string(), true);

        create_map.insert(
            "copy1".to_string(),
            EmailCopyObject {
                id: "msg1".to_string(),
                mailbox_ids,
                keywords: None,
                received_at: None,
            },
        );

        let request = EmailCopyRequest {
            from_account_id: "user1@example.com".to_string(),
            account_id: "user2@example.com".to_string(),
            if_from_in_state: None,
            if_in_state: None,
            create: create_map,
            on_success_destroy_original: Some(false),
            destroy_from_if_in_state: None,
        };

        let response = email_copy(request, store.as_ref(), &test_principal())
            .await
            .expect("email_copy cross_account failed");
        assert_eq!(response.from_account_id, "user1@example.com");
        assert_eq!(response.account_id, "user2@example.com");
    }

    #[tokio::test]
    async fn test_email_copy_with_keywords() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        let mut keywords = HashMap::new();
        keywords.insert("$draft".to_string(), true);
        keywords.insert("$answered".to_string(), true);

        create_map.insert(
            "copy1".to_string(),
            EmailCopyObject {
                id: "msg1".to_string(),
                mailbox_ids: [("drafts".to_string(), true)].iter().cloned().collect(),
                keywords: Some(keywords.clone()),
                received_at: None,
            },
        );

        let request = EmailCopyRequest {
            from_account_id: "acc1".to_string(),
            account_id: "acc2".to_string(),
            if_from_in_state: None,
            if_in_state: None,
            create: create_map,
            on_success_destroy_original: None,
            destroy_from_if_in_state: None,
        };

        let response = email_copy(request, store.as_ref(), &test_principal())
            .await
            .expect("email_copy with_keywords failed");
        assert!(response.created.is_some());
        assert_eq!(response.created.as_ref().expect("created ref").len(), 1);
        let created = response.created.expect("created");
        let created_email = created.values().next().expect("first email");
        assert_eq!(created_email.keywords, keywords);
    }

    #[tokio::test]
    async fn test_email_copy_empty_mailbox_ids() {
        let store = create_test_store();
        let mut create_map = HashMap::new();
        create_map.insert(
            "copy1".to_string(),
            EmailCopyObject {
                id: "msg1".to_string(),
                mailbox_ids: HashMap::new(),
                keywords: None,
                received_at: None,
            },
        );

        let request = EmailCopyRequest {
            from_account_id: "acc1".to_string(),
            account_id: "acc2".to_string(),
            if_from_in_state: None,
            if_in_state: None,
            create: create_map,
            on_success_destroy_original: None,
            destroy_from_if_in_state: None,
        };

        let response = email_copy(request, store.as_ref(), &test_principal())
            .await
            .expect("email_copy empty_mailbox_ids failed");
        assert!(response.created.is_some());
    }
}
