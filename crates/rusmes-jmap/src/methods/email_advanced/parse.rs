//! Email/parse handler implementation.

use super::import::is_valid_rfc5322;
use super::types::{EmailParseRequest, EmailParseResponse};
use crate::blob::{compute_blob_id, BlobStorage};
use crate::methods::email::{convert_mail_to_email, EmailConversionContext};
use crate::methods::ensure_account_ownership;
use crate::types::{Email, Principal};
use rusmes_proto::{Mail, MimeMessage};
use rusmes_storage::MessageStore;
use std::collections::HashMap;

/// Handle Email/parse method
///
/// Parses raw RFC 5322 messages from blob storage without storing them.
/// For each blob ID:
///   - Fetches the blob; records in `not_found` if missing
///   - Validates RFC 5322 structure; records in `not_parsable` if invalid
///   - Converts to JMAP Email with optional property filtering
pub async fn email_parse(
    request: EmailParseRequest,
    _message_store: &dyn MessageStore,
    blob_storage: &BlobStorage,
    principal: &Principal,
) -> anyhow::Result<EmailParseResponse> {
    ensure_account_ownership(&request.account_id, principal)?;

    let mut parsed: HashMap<String, Email> = HashMap::new();
    let mut not_parsable: Vec<String> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    for blob_id in &request.blob_ids {
        // Step 1: fetch blob
        let blob_data = match blob_storage.get(blob_id) {
            Some(b) => b,
            None => {
                not_found.push(blob_id.clone());
                continue;
            }
        };

        let raw = blob_data.data().to_vec();

        // Step 2: validate RFC 5322 minimum structure
        if !is_valid_rfc5322(&raw) {
            not_parsable.push(blob_id.clone());
            continue;
        }

        // Step 3: parse MIME message
        let mime = match MimeMessage::parse_from_bytes(&raw) {
            Ok(m) => m,
            Err(_) => {
                not_parsable.push(blob_id.clone());
                continue;
            }
        };
        let mail = Mail::new(None, vec![], mime, None, None);

        // Step 4: convert to JMAP Email (no storage write).
        // Use a placeholder context: the message is not stored, so there are
        // no persisted flags, mailbox memberships, or thread assignments yet.
        // The blob_id is content-addressed so parse results are stable.
        let content_blob_id = compute_blob_id(&raw);
        let ctx = EmailConversionContext::placeholder(content_blob_id);
        let email = match convert_mail_to_email(blob_id, &mail, ctx).await {
            Ok(e) => e,
            Err(_) => {
                not_parsable.push(blob_id.clone());
                continue;
            }
        };

        // Step 5: apply property filtering if requested
        let email = if let Some(ref properties) = request.properties {
            if !properties.is_empty() {
                filter_email_properties(email, properties)
            } else {
                email
            }
        } else {
            email
        };

        parsed.insert(blob_id.clone(), email);
    }

    Ok(EmailParseResponse {
        account_id: request.account_id,
        parsed,
        not_parsable,
        not_found,
    })
}

/// Return a copy of `email` that only contains the properties listed in
/// `properties`. Unknown property names are silently ignored.
fn filter_email_properties(mut email: Email, properties: &[String]) -> Email {
    let keep: std::collections::HashSet<&str> = properties.iter().map(|s| s.as_str()).collect();

    if !keep.contains("threadId") {
        email.thread_id = None;
    }
    if !keep.contains("mailboxIds") {
        email.mailbox_ids = HashMap::new();
    }
    if !keep.contains("keywords") {
        email.keywords = HashMap::new();
    }
    if !keep.contains("messageId") {
        email.message_id = None;
    }
    if !keep.contains("inReplyTo") {
        email.in_reply_to = None;
    }
    if !keep.contains("references") {
        email.references = None;
    }
    if !keep.contains("sender") {
        email.sender = None;
    }
    if !keep.contains("from") {
        email.from = None;
    }
    if !keep.contains("to") {
        email.to = None;
    }
    if !keep.contains("cc") {
        email.cc = None;
    }
    if !keep.contains("bcc") {
        email.bcc = None;
    }
    if !keep.contains("replyTo") {
        email.reply_to = None;
    }
    if !keep.contains("subject") {
        email.subject = None;
    }
    if !keep.contains("sentAt") {
        email.sent_at = None;
    }
    if !keep.contains("preview") {
        email.preview = None;
    }
    if !keep.contains("bodyValues") {
        email.body_values = None;
    }
    if !keep.contains("textBody") {
        email.text_body = None;
    }
    if !keep.contains("htmlBody") {
        email.html_body = None;
    }
    if !keep.contains("attachments") {
        email.attachments = None;
    }
    email
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::BlobStorage;
    use crate::methods::email_advanced::test_helpers::{
        create_test_store, empty_blobs, test_principal,
    };

    #[tokio::test]
    async fn test_email_parse() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let request = EmailParseRequest {
            account_id: "acc1".to_string(),
            blob_ids: vec!["blob123".to_string()],
            properties: None,
            body_properties: None,
            fetch_text_body_values: None,
            fetch_html_body_values: None,
            fetch_all_body_values: None,
            max_body_value_bytes: None,
        };

        let response = email_parse(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_parse failed");
        assert_eq!(response.account_id, "acc1");
        assert_eq!(response.not_found.len(), 1);
    }

    #[tokio::test]
    async fn test_email_parse_multiple_blobs() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let request = EmailParseRequest {
            account_id: "acc1".to_string(),
            blob_ids: vec![
                "blob1".to_string(),
                "blob2".to_string(),
                "blob3".to_string(),
            ],
            properties: Some(vec!["from".to_string(), "subject".to_string()]),
            body_properties: None,
            fetch_text_body_values: Some(true),
            fetch_html_body_values: Some(false),
            fetch_all_body_values: None,
            max_body_value_bytes: Some(4096),
        };

        let response = email_parse(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_parse multiple_blobs failed");
        assert_eq!(response.not_found.len(), 3);
    }

    #[tokio::test]
    async fn test_email_parse_with_body_values() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let request = EmailParseRequest {
            account_id: "acc1".to_string(),
            blob_ids: vec!["blob789".to_string()],
            properties: None,
            body_properties: Some(vec!["partId".to_string(), "type".to_string()]),
            fetch_text_body_values: Some(true),
            fetch_html_body_values: Some(true),
            fetch_all_body_values: Some(false),
            max_body_value_bytes: Some(8192),
        };

        let response = email_parse(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_parse with_body_values failed");
        assert_eq!(response.parsed.len(), 0);
    }

    #[tokio::test]
    async fn test_email_parse_all_properties() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let properties = vec![
            "id".to_string(),
            "blobId".to_string(),
            "threadId".to_string(),
            "mailboxIds".to_string(),
            "keywords".to_string(),
            "size".to_string(),
            "receivedAt".to_string(),
            "messageId".to_string(),
            "inReplyTo".to_string(),
            "references".to_string(),
            "sender".to_string(),
            "from".to_string(),
            "to".to_string(),
            "cc".to_string(),
            "bcc".to_string(),
            "replyTo".to_string(),
            "subject".to_string(),
            "sentAt".to_string(),
            "hasAttachment".to_string(),
            "preview".to_string(),
        ];

        let request = EmailParseRequest {
            account_id: "acc1".to_string(),
            blob_ids: vec!["blob_all".to_string()],
            properties: Some(properties),
            body_properties: None,
            fetch_text_body_values: Some(true),
            fetch_html_body_values: Some(true),
            fetch_all_body_values: Some(true),
            max_body_value_bytes: Some(1048576),
        };

        let response = email_parse(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_parse all_properties failed");
        assert_eq!(response.account_id, "acc1");
    }

    #[tokio::test]
    async fn test_email_parse_empty_blob_ids() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let request = EmailParseRequest {
            account_id: "acc1".to_string(),
            blob_ids: vec![],
            properties: None,
            body_properties: None,
            fetch_text_body_values: None,
            fetch_html_body_values: None,
            fetch_all_body_values: None,
            max_body_value_bytes: None,
        };

        let response = email_parse(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_parse empty_blob_ids failed");
        assert_eq!(response.parsed.len(), 0);
        assert_eq!(response.not_parsable.len(), 0);
        assert_eq!(response.not_found.len(), 0);
    }

    #[tokio::test]
    async fn test_email_parse_empty_properties() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let request = EmailParseRequest {
            account_id: "acc1".to_string(),
            blob_ids: vec!["blob1".to_string()],
            properties: Some(vec![]),
            body_properties: Some(vec![]),
            fetch_text_body_values: None,
            fetch_html_body_values: None,
            fetch_all_body_values: None,
            max_body_value_bytes: None,
        };

        let response = email_parse(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_parse empty_properties failed");
        assert_eq!(response.not_found.len(), 1);
    }

    /// Parse a valid RFC 5322 blob without storing it — must appear in `parsed`.
    #[tokio::test]
    async fn test_email_parse_valid() {
        let store = create_test_store();
        let blobs = BlobStorage::new();

        let raw_rfc5322 = b"From: alice@example.com\r\nTo: bob@example.com\r\nSubject: Hello\r\nDate: Mon, 01 Jan 2024 12:00:00 +0000\r\n\r\nHi Bob!\r\n";
        blobs.store(
            "parse-blob-001".to_string(),
            raw_rfc5322.to_vec(),
            "message/rfc822".to_string(),
        );

        let req = EmailParseRequest {
            account_id: "acc1".to_string(),
            blob_ids: vec!["parse-blob-001".to_string()],
            properties: None,
            body_properties: None,
            fetch_text_body_values: None,
            fetch_html_body_values: None,
            fetch_all_body_values: None,
            max_body_value_bytes: None,
        };

        let resp = email_parse(req, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_parse valid failed");

        assert_eq!(resp.account_id, "acc1");
        assert!(
            resp.parsed.contains_key("parse-blob-001"),
            "expected blob in parsed; not_parsable={:?} not_found={:?}",
            resp.not_parsable,
            resp.not_found
        );
        assert!(resp.not_found.is_empty());
        assert!(resp.not_parsable.is_empty());
    }

    /// Parse a blob that does not exist in blob storage — must appear in `not_found`.
    #[tokio::test]
    async fn test_email_parse_not_found() {
        let store = create_test_store();
        let blobs = BlobStorage::new(); // empty — no blobs stored

        let req = EmailParseRequest {
            account_id: "acc1".to_string(),
            blob_ids: vec!["nonexistent-blob".to_string()],
            properties: None,
            body_properties: None,
            fetch_text_body_values: None,
            fetch_html_body_values: None,
            fetch_all_body_values: None,
            max_body_value_bytes: None,
        };

        let resp = email_parse(req, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_parse not_found failed");

        assert_eq!(resp.account_id, "acc1");
        assert!(resp.not_found.contains(&"nonexistent-blob".to_string()));
        assert!(resp.parsed.is_empty());
        assert!(resp.not_parsable.is_empty());
    }
}
