//! EmailSubmission create handler — wires identity/message lookup to SMTP delivery

use crate::methods::submission::handlers::SubmissionContext;
use crate::methods::submission::store::StoredSubmission;
use crate::methods::submission::types::{EmailSubmission, EmailSubmissionObject, UndoStatus};
use crate::types::{JmapSetError, Principal};
use chrono::Utc;
use rusmes_core::transport::SmtpEnvelope;
use rusmes_proto::{Mail, MessageId};
use std::str::FromStr;
use uuid::Uuid;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse a JMAP email_id (UUID string) into a storage [`MessageId`].
fn parse_message_id(id: &str) -> Result<MessageId, JmapSetError> {
    let uuid = Uuid::from_str(id).map_err(|_| JmapSetError {
        error_type: "invalidProperties".to_string(),
        description: Some(format!(
            "'{}' is not a valid message ID (expected UUID)",
            id
        )),
    })?;
    Ok(MessageId::from_uuid(uuid))
}

/// Build a minimal [`Mail`] wrapper around the fetched message so the transport
/// can serialise it for the wire.
fn mail_from_fetched(fetched: &Mail) -> Mail {
    // Clone the underlying message data without changing it.
    fetched.clone()
}

// ── Main handler ──────────────────────────────────────────────────────────────

/// Handle a single EmailSubmission create operation.
///
/// Returns `Ok(EmailSubmission)` on success or `Err(JmapSetError)` on
/// validation/delivery failure.
pub(super) async fn handle_submission_create(
    account_id: &str,
    creation_id: &str,
    obj: EmailSubmissionObject,
    principal: &Principal,
    ctx: &SubmissionContext<'_>,
) -> Result<EmailSubmission, JmapSetError> {
    // ── 1. Validate identity_id ───────────────────────────────────────────────
    let identity = ctx
        .identity_store
        .get_identity(account_id, &principal.username, &obj.identity_id)
        .await
        .map_err(|e| JmapSetError {
            error_type: "serverFail".to_string(),
            description: Some(format!("identity lookup error: {}", e)),
        })?;

    let identity = identity.ok_or_else(|| JmapSetError {
        error_type: "notFound".to_string(),
        description: Some(format!(
            "Identity '{}' not found for account '{}'",
            obj.identity_id, account_id
        )),
    })?;

    // ── 2. Validate email_id and fetch the message ────────────────────────────
    let message_id = parse_message_id(&obj.email_id)?;

    let fetched_mail = ctx
        .message_store
        .get_message(&message_id)
        .await
        .map_err(|e| JmapSetError {
            error_type: "serverFail".to_string(),
            description: Some(format!("message lookup error: {}", e)),
        })?
        .ok_or_else(|| JmapSetError {
            error_type: "notFound".to_string(),
            description: Some(format!(
                "Email '{}' not found for account '{}'",
                obj.email_id, account_id
            )),
        })?;

    // ── 3. Build SMTP envelope ────────────────────────────────────────────────
    let envelope = build_envelope(&obj, &identity.email, &fetched_mail)?;

    // ── 4. Determine thread_id ────────────────────────────────────────────────
    let thread_id = ctx
        .message_store
        .get_message_thread_id(&message_id)
        .await
        .unwrap_or(None);

    // ── 5. Build StoredSubmission with pending status ─────────────────────────
    let submission_id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let stored = StoredSubmission {
        submission: EmailSubmission {
            id: submission_id.clone(),
            identity_id: obj.identity_id.clone(),
            email_id: obj.email_id.clone(),
            thread_id,
            envelope: obj.envelope.clone(),
            send_at: obj.send_at,
            undo_status: UndoStatus::Pending,
            delivery_status: None,
            dsn_blob_ids: None,
            mdn_blob_ids: None,
        },
        created_at: now,
    };

    // ── 6. Persist to submission store ────────────────────────────────────────
    ctx.submission_store
        .put_submission(account_id, stored.clone())
        .await
        .map_err(|e| JmapSetError {
            error_type: "serverFail".to_string(),
            description: Some(format!("failed to persist submission: {}", e)),
        })?;

    // ── 7. Deliver via transport ──────────────────────────────────────────────
    let mail = mail_from_fetched(&fetched_mail);

    let transport_result = if let Some(send_at) = obj.send_at {
        ctx.mail_transport.send_at(envelope, &mail, send_at).await
    } else {
        ctx.mail_transport.send(envelope, &mail).await
    };

    if let Err(e) = transport_result {
        tracing::error!(
            submission_id = %submission_id,
            creation_id = %creation_id,
            "Mail transport failed: {}",
            e
        );
        // Transport failure is not fatal at the JMAP level — the submission is
        // persisted in pending state.  A retry mechanism can pick it up later.
    }

    // ── 8. Return created submission object ───────────────────────────────────
    Ok(stored.submission)
}

// ── Envelope building ─────────────────────────────────────────────────────────

/// Build the `SmtpEnvelope` from the JMAP request or fall back to
/// identity/message-derived addresses.
fn build_envelope(
    obj: &EmailSubmissionObject,
    identity_email: &str,
    mail: &Mail,
) -> Result<SmtpEnvelope, JmapSetError> {
    if let Some(env) = &obj.envelope {
        // Explicit envelope provided — use it directly.
        if env.rcpt_to.is_empty() {
            return Err(JmapSetError {
                error_type: "invalidProperties".to_string(),
                description: Some("envelope.rcptTo must contain at least one address".to_string()),
            });
        }
        return Ok(SmtpEnvelope {
            mail_from: env.mail_from.email.clone(),
            rcpt_to: env.rcpt_to.iter().map(|a| a.email.clone()).collect(),
        });
    }

    // Derive envelope from identity email + message To/Cc headers.
    let mail_from = identity_email.to_string();
    let rcpt_to = derive_recipients_from_mail(mail);

    if rcpt_to.is_empty() {
        return Err(JmapSetError {
            error_type: "invalidProperties".to_string(),
            description: Some(
                "No envelope provided and no To/Cc/Bcc headers found in message".to_string(),
            ),
        });
    }

    Ok(SmtpEnvelope { mail_from, rcpt_to })
}

/// Extract RFC 5321 recipient addresses from a [`Mail`]'s To/Cc/Bcc headers.
fn derive_recipients_from_mail(mail: &Mail) -> Vec<String> {
    let mut recipients = Vec::new();
    let headers = mail.message().headers();

    for (name, values) in headers.iter() {
        let lc = name.to_lowercase();
        if lc == "to" || lc == "cc" || lc == "bcc" {
            for value in values {
                extract_addresses_from_header(value, &mut recipients);
            }
        }
    }

    recipients
}

/// Simple address extraction: handles `Name <email>` and bare `email` forms,
/// comma-separated.
fn extract_addresses_from_header(header_value: &str, out: &mut Vec<String>) {
    for token in header_value.split(',') {
        let token = token.trim();
        // Look for angle-bracket form first.
        if let (Some(open), Some(close)) = (token.find('<'), token.rfind('>')) {
            if open < close {
                let addr = token[open + 1..close].trim().to_string();
                if !addr.is_empty() {
                    out.push(addr);
                    continue;
                }
            }
        }
        // Fall back to bare token if it looks like an email.
        if token.contains('@') {
            out.push(token.to_string());
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

    #[test]
    fn test_extract_angle_bracket_addresses() {
        let mut out = Vec::new();
        extract_addresses_from_header("Alice <alice@example.com>, Bob <bob@example.com>", &mut out);
        assert_eq!(out, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn test_extract_bare_addresses() {
        let mut out = Vec::new();
        extract_addresses_from_header("carol@example.com, dave@example.com", &mut out);
        assert_eq!(out, vec!["carol@example.com", "dave@example.com"]);
    }

    #[test]
    fn test_parse_message_id_valid_uuid() {
        let id = Uuid::new_v4().to_string();
        let result = parse_message_id(&id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_message_id_invalid() {
        let result = parse_message_id("not-a-uuid");
        assert!(result.is_err());
        assert_eq!(result.expect_err("err").error_type, "invalidProperties");
    }

    #[test]
    fn test_build_envelope_empty_rcpt_explicit() {
        use crate::methods::submission::types::{Address, Envelope};
        let obj = EmailSubmissionObject {
            identity_id: "id1".to_string(),
            email_id: Uuid::new_v4().to_string(),
            envelope: Some(Envelope {
                mail_from: Address {
                    email: "from@example.com".to_string(),
                    parameters: None,
                },
                rcpt_to: vec![],
            }),
            send_at: None,
        };

        let mut headers = HeaderMap::new();
        headers.insert("From", "from@example.com");
        let msg = MimeMessage::new(headers, MessageBody::Small(bytes::Bytes::new()));
        let mail = Mail::new(None, vec![], msg, None, None);

        let err = build_envelope(&obj, "identity@example.com", &mail).expect_err("should err");
        assert_eq!(err.error_type, "invalidProperties");
    }
}
