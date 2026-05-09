//! Email/import handler implementation.

use super::types::{EmailImportObject, EmailImportRequest, EmailImportResponse};
use crate::blob::{compute_blob_id, BlobStorage};
use crate::methods::email::{parse_date_header, parse_email_addresses};
use crate::methods::ensure_account_ownership;
use crate::types::{Email, JmapSetError, Principal};
use chrono::Utc;
use rusmes_proto::{Mail, MimeMessage};
use rusmes_storage::{MailboxId, MessageFlags, MessageStore};
use std::collections::HashMap;

/// Handle Email/import method
///
/// Imports a raw RFC 5322 message from blob storage into the message store.
/// For each email in the request:
///   - Fetches the blob by ID; records `blobNotFound` if missing
///   - Validates minimum RFC 5322 structure (header/body separator); records `invalidEmail` if invalid
///   - Parses the MIME message and stores it in the primary mailbox
///   - Copies to additional mailboxes (if `mailbox_ids` has more than one entry)
///   - Applies keyword flags via the storage layer
pub async fn email_import(
    request: EmailImportRequest,
    message_store: &dyn MessageStore,
    blob_storage: &BlobStorage,
    principal: &Principal,
) -> anyhow::Result<EmailImportResponse> {
    ensure_account_ownership(&request.account_id, principal)?;

    let old_state = super::get_current_modseq(message_store).await?.to_string();

    let mut created: HashMap<String, Email> = HashMap::new();
    let mut not_created: HashMap<String, JmapSetError> = HashMap::new();

    for (creation_id, import_obj) in &request.emails {
        // Step 1: fetch blob
        let blob_data = match blob_storage.get(&import_obj.blob_id) {
            Some(b) => b,
            None => {
                not_created.insert(
                    creation_id.clone(),
                    JmapSetError {
                        error_type: "blobNotFound".to_string(),
                        description: Some(format!("Blob '{}' not found", import_obj.blob_id)),
                    },
                );
                continue;
            }
        };

        let raw = blob_data.data().to_vec();

        // Step 2: validate RFC 5322 minimum structure (header/body separator)
        if !is_valid_rfc5322(&raw) {
            not_created.insert(
                creation_id.clone(),
                JmapSetError {
                    error_type: "invalidEmail".to_string(),
                    description: Some(
                        "Data does not contain a valid RFC 5322 header/body separator".to_string(),
                    ),
                },
            );
            continue;
        }

        // Step 3: parse MIME message
        let mime = match MimeMessage::parse_from_bytes(&raw) {
            Ok(m) => m,
            Err(e) => {
                not_created.insert(
                    creation_id.clone(),
                    JmapSetError {
                        error_type: "invalidEmail".to_string(),
                        description: Some(format!("Failed to parse MIME message: {}", e)),
                    },
                );
                continue;
            }
        };
        let mail = Mail::new(None, vec![], mime, None, None);

        // Step 4: determine primary mailbox and additional mailboxes
        let active_mailbox_ids: Vec<String> = import_obj
            .mailbox_ids
            .iter()
            .filter_map(|(id, &active)| if active { Some(id.clone()) } else { None })
            .collect();

        if active_mailbox_ids.is_empty() {
            not_created.insert(
                creation_id.clone(),
                JmapSetError {
                    error_type: "invalidArguments".to_string(),
                    description: Some("At least one mailboxId must be specified".to_string()),
                },
            );
            continue;
        }

        let primary_mailbox_id = parse_or_create_mailbox_id(&active_mailbox_ids[0]);

        // Step 5: store in primary mailbox
        let meta = match message_store
            .append_message(&primary_mailbox_id, mail)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                not_created.insert(
                    creation_id.clone(),
                    JmapSetError {
                        error_type: "serverFail".to_string(),
                        description: Some(format!("Failed to store message: {}", e)),
                    },
                );
                continue;
            }
        };

        let message_id = *meta.message_id();
        let message_id_str = message_id.to_string();

        // Step 6: copy to additional mailboxes
        for extra_mailbox_str in active_mailbox_ids.iter().skip(1) {
            let extra_mailbox = parse_or_create_mailbox_id(extra_mailbox_str);
            if let Err(e) = message_store
                .copy_messages(std::slice::from_ref(&message_id), &extra_mailbox)
                .await
            {
                tracing::warn!(
                    "Email/import: failed to copy message {} to mailbox {}: {}",
                    message_id_str,
                    extra_mailbox_str,
                    e
                );
            }
        }

        // Step 7: apply keywords as flags
        if let Some(ref keywords) = import_obj.keywords {
            let flags = keywords_to_flags(keywords);
            if let Err(e) = message_store
                .set_flags(std::slice::from_ref(&message_id), flags)
                .await
            {
                tracing::warn!(
                    "Email/import: failed to set flags on message {}: {}",
                    message_id_str,
                    e
                );
            }
        }

        // Step 8: build the JMAP Email object with header-derived fields.
        let email = make_placeholder_email(&message_id_str, import_obj, raw.len(), &raw);
        created.insert(creation_id.clone(), email);
    }

    let new_state = super::get_current_modseq(message_store).await?.to_string();

    Ok(EmailImportResponse {
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

/// Validate that `data` contains the RFC 5322 mandatory header/body separator.
///
/// RFC 5322 requires headers and body to be separated by an empty line
/// (`\r\n\r\n` or bare `\n\n`). We also accept a trailing `\r\n` / `\n` as
/// a degenerate (header-only, no body) message.
pub(super) fn is_valid_rfc5322(data: &[u8]) -> bool {
    if data.windows(4).any(|w| w == b"\r\n\r\n") {
        return true;
    }
    if data.windows(2).any(|w| w == b"\n\n") {
        return true;
    }
    // Accept header-only messages that end with a single CRLF or LF
    if data.ends_with(b"\r\n") || data.ends_with(b"\n") {
        return true;
    }
    false
}

/// Convert a JMAP keyword map to storage [`MessageFlags`].
fn keywords_to_flags(keywords: &HashMap<String, bool>) -> MessageFlags {
    let mut flags = MessageFlags::new();
    for (keyword, &value) in keywords {
        match keyword.as_str() {
            "$seen" => flags.set_seen(value),
            "$answered" => flags.set_answered(value),
            "$flagged" => flags.set_flagged(value),
            "$draft" => flags.set_draft(value),
            _ => {}
        }
    }
    flags
}

/// Parse a mailbox ID string into a [`MailboxId`].
///
/// Attempts UUID parsing first; falls back to a deterministic UUID derived
/// from the string bytes via SHA-256 so that opaque IDs (e.g. "inbox") are
/// accepted without panicking and produce a stable result across calls.
fn parse_or_create_mailbox_id(s: &str) -> MailboxId {
    if let Ok(id) = uuid::Uuid::parse_str(s) {
        return MailboxId::from_uuid(id);
    }
    // Derive a deterministic UUID from the name bytes using SHA-256.
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let hash = hasher.finalize();
    // Take first 16 bytes and set UUID version/variant bits (version 4 style).
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant RFC 4122
    MailboxId::from_uuid(uuid::Uuid::from_bytes(bytes))
}

/// Build a JMAP [`Email`] from import metadata and header-derived fields.
///
/// The `id` is the storage-assigned message ID string.  Header fields
/// (from/to/cc/bcc/sender/reply-to/subject/message-id/in-reply-to/references/sentAt)
/// are extracted from the raw RFC 5322 bytes so the response reflects the
/// actual message content rather than returning `null` for everything.
fn make_placeholder_email(
    id: &str,
    import_obj: &EmailImportObject,
    size: usize,
    raw: &[u8],
) -> Email {
    // Re-parse headers from raw bytes to extract addressable metadata.
    // A parse failure here degrades gracefully to None fields.
    struct ParsedHeaders {
        message_id: Option<Vec<String>>,
        in_reply_to: Option<Vec<String>>,
        references: Option<Vec<String>>,
        subject: Option<String>,
        sent_at: Option<chrono::DateTime<Utc>>,
        from: Option<Vec<crate::types::EmailAddress>>,
        to: Option<Vec<crate::types::EmailAddress>>,
        cc: Option<Vec<crate::types::EmailAddress>>,
        bcc: Option<Vec<crate::types::EmailAddress>>,
        reply_to: Option<Vec<crate::types::EmailAddress>>,
        sender: Option<Vec<crate::types::EmailAddress>>,
    }

    let ph = match rusmes_proto::MimeMessage::parse_from_bytes(raw) {
        Ok(mime) => {
            let h = mime.headers().clone();
            ParsedHeaders {
                message_id: h.get_first("message-id").map(|s| vec![s.to_string()]),
                in_reply_to: h.get_first("in-reply-to").map(|s| vec![s.to_string()]),
                references: h
                    .get("references")
                    .map(|rs| rs.iter().map(|s| s.to_string()).collect()),
                subject: h.get_first("subject").map(|s| s.to_string()),
                sent_at: parse_date_header(h.get_first("date")),
                from: parse_email_addresses(&h, "from"),
                to: parse_email_addresses(&h, "to"),
                cc: parse_email_addresses(&h, "cc"),
                bcc: parse_email_addresses(&h, "bcc"),
                reply_to: parse_email_addresses(&h, "reply-to"),
                sender: parse_email_addresses(&h, "sender"),
            }
        }
        Err(_) => ParsedHeaders {
            message_id: None,
            in_reply_to: None,
            references: None,
            subject: None,
            sent_at: None,
            from: None,
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            sender: None,
        },
    };

    // received_at: prefer the client-supplied value; fall back to now.
    let received_at = import_obj.received_at.unwrap_or_else(Utc::now);

    // Use content-addressed blob ID derived from raw bytes.
    let blob_id = compute_blob_id(raw);

    Email {
        id: id.to_string(),
        blob_id,
        thread_id: None,
        mailbox_ids: import_obj.mailbox_ids.clone(),
        keywords: import_obj.keywords.clone().unwrap_or_default(),
        size: size as u64,
        received_at,
        message_id: ph.message_id,
        in_reply_to: ph.in_reply_to,
        references: ph.references,
        sender: ph.sender,
        from: ph.from,
        to: ph.to,
        cc: ph.cc,
        bcc: ph.bcc,
        reply_to: ph.reply_to,
        subject: ph.subject,
        sent_at: ph.sent_at,
        has_attachment: false,
        preview: None,
        body_values: None,
        text_body: None,
        html_body: None,
        attachments: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::BlobStorage;
    use crate::methods::email_advanced::test_helpers::{
        create_test_backend, create_test_store, empty_blobs, test_principal,
    };
    use rusmes_proto::Username;
    use rusmes_storage::{MailboxPath, StorageBackend};

    #[tokio::test]
    async fn test_email_import() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let mut emails = HashMap::new();
        emails.insert(
            "import1".to_string(),
            EmailImportObject {
                blob_id: "blob123".to_string(),
                mailbox_ids: [("inbox".to_string(), true)].iter().cloned().collect(),
                keywords: None,
                received_at: None,
            },
        );

        let request = EmailImportRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            emails,
        };

        let response = email_import(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_import failed");
        assert_eq!(response.account_id, "acc1");
        // blob123 not in blob storage => not_created with blobNotFound
        assert!(response.not_created.is_some());
        let nc = response.not_created.expect("not_created");
        assert_eq!(nc["import1"].error_type, "blobNotFound");
    }

    #[tokio::test]
    async fn test_email_import_with_keywords() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let mut emails = HashMap::new();
        let mut keywords = HashMap::new();
        keywords.insert("$flagged".to_string(), true);
        keywords.insert("$seen".to_string(), true);

        emails.insert(
            "import1".to_string(),
            EmailImportObject {
                blob_id: "blob456".to_string(),
                mailbox_ids: [("sent".to_string(), true)].iter().cloned().collect(),
                keywords: Some(keywords),
                received_at: Some(chrono::Utc::now()),
            },
        );

        let request = EmailImportRequest {
            account_id: "acc1".to_string(),
            if_in_state: Some("state5".to_string()),
            emails,
        };

        let response = email_import(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_import with_keywords failed");
        assert_eq!(response.account_id, "acc1");
    }

    #[tokio::test]
    async fn test_email_import_multiple_emails() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let mut emails = HashMap::new();

        for i in 1..=3 {
            emails.insert(
                format!("import{}", i),
                EmailImportObject {
                    blob_id: format!("blob{}", i),
                    mailbox_ids: [("inbox".to_string(), true)].iter().cloned().collect(),
                    keywords: None,
                    received_at: None,
                },
            );
        }

        let request = EmailImportRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            emails,
        };

        let response = email_import(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_import multiple_emails failed");
        assert_eq!(response.not_created.expect("not_created").len(), 3);
    }

    #[tokio::test]
    async fn test_email_import_empty_emails() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let request = EmailImportRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            emails: HashMap::new(),
        };

        let response = email_import(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_import empty_emails failed");
        assert!(response.created.is_none());
        assert!(response.not_created.is_none());
    }

    #[tokio::test]
    async fn test_email_import_invalid_blob() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let mut emails = HashMap::new();
        emails.insert(
            "import1".to_string(),
            EmailImportObject {
                blob_id: "invalid_blob_id".to_string(),
                mailbox_ids: [("inbox".to_string(), true)].iter().cloned().collect(),
                keywords: None,
                received_at: None,
            },
        );

        let request = EmailImportRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            emails,
        };

        let response = email_import(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_import invalid_blob failed");
        assert!(response.not_created.is_some());
    }

    #[tokio::test]
    async fn test_email_import_with_multiple_mailboxes() {
        let store = create_test_store();
        let blobs = empty_blobs();
        let mut emails = HashMap::new();
        let mut mailbox_ids = HashMap::new();
        mailbox_ids.insert("inbox".to_string(), true);
        mailbox_ids.insert("important".to_string(), true);
        mailbox_ids.insert("work".to_string(), true);

        emails.insert(
            "import1".to_string(),
            EmailImportObject {
                blob_id: "blob999".to_string(),
                mailbox_ids,
                keywords: None,
                received_at: None,
            },
        );

        let request = EmailImportRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            emails,
        };

        let response = email_import(request, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_import with_multiple_mailboxes failed");
        assert!(response.not_created.is_some());
    }

    /// Import a valid RFC 5322 message stored in blob storage.
    ///
    /// The test pre-creates a mailbox in the filesystem backend so that
    /// `append_message` can succeed.
    #[tokio::test]
    async fn test_email_import_valid_rfc5322() {
        let backend = create_test_backend();
        let mailbox_store = backend.mailbox_store();
        let message_store = backend.message_store();

        // Pre-create the mailbox so append_message finds it
        let user = Username::new("testuser").expect("username");
        let path = MailboxPath::new(user, vec!["INBOX".to_string()]);
        let mailbox_id = mailbox_store
            .create_mailbox(&path)
            .await
            .expect("create mailbox");

        let raw_rfc5322 = b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Test\r\nDate: Mon, 01 Jan 2024 00:00:00 +0000\r\n\r\nHello, world!\r\n";

        let blobs = BlobStorage::new();
        blobs.store(
            "test-blob-001".to_string(),
            raw_rfc5322.to_vec(),
            "message/rfc822".to_string(),
        );

        let mut mailbox_ids = HashMap::new();
        mailbox_ids.insert(mailbox_id.to_string(), true);

        let mut emails = HashMap::new();
        emails.insert(
            "import-rfc5322".to_string(),
            EmailImportObject {
                blob_id: "test-blob-001".to_string(),
                mailbox_ids,
                keywords: None,
                received_at: None,
            },
        );

        let req = EmailImportRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            emails,
        };

        let resp = email_import(req, message_store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_import valid_rfc5322 failed");

        assert_eq!(resp.account_id, "acc1");
        assert!(
            resp.created.is_some(),
            "expected created, got not_created: {:?}",
            resp.not_created
        );
        let created = resp.created.expect("created");
        assert!(created.contains_key("import-rfc5322"));
        let email = &created["import-rfc5322"];
        // blob_id is now a content-addressed SHA-256 hex of the raw bytes,
        // not the upload blob_id.
        let expected_blob_id = compute_blob_id(raw_rfc5322);
        assert_eq!(email.blob_id, expected_blob_id);
        assert_eq!(email.size, raw_rfc5322.len() as u64);

        // make_placeholder_email must extract RFC 5322 headers from raw bytes.
        // Regression guard: these fields must not be None / empty after the fix.
        let from_addrs = email
            .from
            .as_deref()
            .expect("from must be populated from raw bytes");
        assert!(
            from_addrs.iter().any(|a| a.email == "sender@example.com"),
            "expected sender@example.com in from, got: {:?}",
            from_addrs
        );

        let to_addrs = email
            .to
            .as_deref()
            .expect("to must be populated from raw bytes");
        assert!(
            to_addrs.iter().any(|a| a.email == "recipient@example.com"),
            "expected recipient@example.com in to, got: {:?}",
            to_addrs
        );

        // The subject value retains the leading SP from the header folding
        // ("Subject: Test" → " Test"); trim before comparison.
        assert_eq!(
            email.subject.as_deref().map(str::trim),
            Some("Test"),
            "subject must be extracted from raw bytes"
        );

        assert!(
            email.sent_at.is_some(),
            "sent_at must be parsed from Date header in raw bytes"
        );
    }

    /// Import a blob whose content lacks the RFC 5322 header/body separator —
    /// must be rejected with `invalidEmail`.
    #[tokio::test]
    async fn test_email_import_invalid_blob_content() {
        let store = create_test_store();
        let blobs = BlobStorage::new();
        // No \r\n\r\n, \n\n, or trailing line ending — unambiguously invalid
        blobs.store(
            "bad-blob".to_string(),
            b"no-separator-here-at-all".to_vec(),
            "message/rfc822".to_string(),
        );

        let mut mailbox_ids = HashMap::new();
        mailbox_ids.insert("inbox".to_string(), true);

        let mut emails = HashMap::new();
        emails.insert(
            "import-bad".to_string(),
            EmailImportObject {
                blob_id: "bad-blob".to_string(),
                mailbox_ids,
                keywords: None,
                received_at: None,
            },
        );

        let req = EmailImportRequest {
            account_id: "acc1".to_string(),
            if_in_state: None,
            emails,
        };

        let resp = email_import(req, store.as_ref(), &blobs, &test_principal())
            .await
            .expect("email_import invalid_blob_content failed");

        assert!(resp.not_created.is_some());
        let nc = resp.not_created.expect("not_created");
        assert_eq!(
            nc["import-bad"].error_type, "invalidEmail",
            "expected invalidEmail, got: {}",
            nc["import-bad"].error_type
        );
    }
}
