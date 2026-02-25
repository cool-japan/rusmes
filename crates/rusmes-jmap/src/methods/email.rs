//! Email method implementations

use crate::types::{
    Email, EmailAddress, EmailGetRequest, EmailGetResponse, EmailQueryRequest, EmailQueryResponse,
    EmailSetRequest, EmailSetResponse, JmapSetError,
};
use chrono::Utc;
use rusmes_proto::{HeaderMap, Mail, MessageId, MimeMessage};
use rusmes_storage::{MailboxId, MessageStore};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

/// Handle Email/get method
pub async fn email_get(
    request: EmailGetRequest,
    message_store: &dyn MessageStore,
) -> anyhow::Result<EmailGetResponse> {
    let mut list = Vec::new();
    let mut not_found = Vec::new();

    // If no IDs specified, return empty list
    let ids = request.ids.unwrap_or_default();

    for id in ids {
        // Parse the ID as MessageId
        match parse_message_id(&id) {
            Ok(message_id) => {
                // Fetch the message from storage
                match message_store.get_message(&message_id).await? {
                    Some(mail) => {
                        // Convert Mail to Email
                        let email = convert_mail_to_email(&id, &mail)?;
                        list.push(email);
                    }
                    None => {
                        not_found.push(id);
                    }
                }
            }
            Err(_) => {
                not_found.push(id);
            }
        }
    }

    // Generate state token based on current timestamp
    // In production, this would be a monotonic counter from the storage backend
    let state = format!("{}", Utc::now().timestamp());

    Ok(EmailGetResponse {
        account_id: request.account_id,
        state,
        list,
        not_found,
    })
}

/// Handle Email/set method
#[allow(clippy::too_many_arguments)]
pub async fn email_set(
    request: EmailSetRequest,
    _message_store: &dyn MessageStore,
) -> anyhow::Result<EmailSetResponse> {
    let created: HashMap<String, crate::types::Email> = HashMap::new();
    let updated: HashMap<String, Option<crate::types::Email>> = HashMap::new();
    let destroyed: Vec<String> = Vec::new();
    let mut not_created = HashMap::new();
    let mut not_updated = HashMap::new();
    let mut not_destroyed = HashMap::new();

    // Handle creates
    if let Some(create_map) = request.create {
        for (creation_id, _email_obj) in create_map {
            // For now, return error as we need more infrastructure
            not_created.insert(
                creation_id,
                JmapSetError {
                    error_type: "notImplemented".to_string(),
                    description: Some("Email creation not yet implemented".to_string()),
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
                    description: Some("Email update not yet implemented".to_string()),
                },
            );
        }
    }

    // Handle destroys
    if let Some(destroy_ids) = request.destroy {
        for id in destroy_ids {
            not_destroyed.insert(
                id,
                JmapSetError {
                    error_type: "notImplemented".to_string(),
                    description: Some("Email deletion not yet implemented".to_string()),
                },
            );
        }
    }

    Ok(EmailSetResponse {
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

/// Handle Email/query method
pub async fn email_query(
    request: EmailQueryRequest,
    message_store: &dyn MessageStore,
) -> anyhow::Result<EmailQueryResponse> {
    tracing::debug!("EMAIL/QUERY CALLED - accountId: {}", request.account_id);
    let mut ids = Vec::new();

    // If filter specifies inMailbox, use that
    if let Some(filter) = &request.filter {
        if let Some(mailbox_id_str) = &filter.in_mailbox {
            // Parse mailbox ID and get all messages from that mailbox
            if let Ok(mailbox_id) = parse_mailbox_id(mailbox_id_str) {
                let messages = message_store.get_mailbox_messages(&mailbox_id).await?;

                // Convert MessageMetadata to string IDs
                for metadata in messages {
                    ids.push(metadata.message_id().to_string());
                }
            }
        } else {
            // No specific mailbox - search all known mailboxes
            // This is a simplified approach - in production, would track all mailboxes
            ids = get_all_messages_from_all_mailboxes(message_store).await?;
        }
    } else {
        // No filter at all - get all messages from all mailboxes
        ids = get_all_messages_from_all_mailboxes(message_store).await?;
    }

    // Apply position and limit
    let position = request.position.unwrap_or(0) as usize;
    let limit = request.limit.unwrap_or(100) as usize;

    let total = ids.len() as u64;

    // Slice the results
    let start = position.min(ids.len());
    let end = (start + limit).min(ids.len());
    let result_ids = ids[start..end].to_vec();

    Ok(EmailQueryResponse {
        account_id: request.account_id,
        query_state: "1".to_string(),
        can_calculate_changes: false,
        position: position as i64,
        ids: result_ids,
        total: if request.calculate_total.unwrap_or(false) {
            Some(total)
        } else {
            None
        },
        limit: Some(limit as u64),
    })
}

/// Detect if a mail has attachments by checking MIME headers
fn detect_attachment(mail: &Mail) -> bool {
    let headers = mail.message().headers();

    // Check Content-Type for multipart
    if let Some(content_type) = headers.get_first("Content-Type") {
        let content_type_lower = content_type.to_lowercase();

        // Multipart messages (except multipart/alternative which is just text+html)
        if content_type_lower.contains("multipart/mixed")
            || content_type_lower.contains("multipart/related")
        {
            return true;
        }
    }

    // Check Content-Disposition for attachment
    if let Some(disposition) = headers.get_first("Content-Disposition") {
        if disposition.to_lowercase().contains("attachment") {
            return true;
        }
    }

    // Check for common attachment indicators in headers
    // Files like image/*, application/*, etc. (but not text/plain or text/html)
    if let Some(content_type) = headers.get_first("Content-Type") {
        let content_type_lower = content_type.to_lowercase();
        if (content_type_lower.starts_with("image/")
            || content_type_lower.starts_with("application/")
            || content_type_lower.starts_with("audio/")
            || content_type_lower.starts_with("video/"))
            && !content_type_lower.contains("multipart/alternative")
        {
            // Additional check: must have filename parameter or be disposition: attachment
            if content_type_lower.contains("name=")
                || headers
                    .get_first("Content-Disposition")
                    .map(|d| d.to_lowercase().contains("attachment"))
                    .unwrap_or(false)
            {
                return true;
            }
        }
    }

    false
}

/// Convert a Mail object to an Email JMAP object
fn convert_mail_to_email(id: &str, mail: &Mail) -> anyhow::Result<Email> {
    let message = mail.message();
    let headers = message.headers();

    // Extract basic metadata
    let size = mail.size() as u64;
    let blob_id = format!("blob-{}", id); // In production, would be actual blob ID
    let received_at = Utc::now(); // In production, would come from metadata

    // Parse headers for email fields
    let subject = headers.get_first("subject").map(|s| s.to_string());
    let from = parse_email_addresses(headers, "from");
    let to = parse_email_addresses(headers, "to");
    let cc = parse_email_addresses(headers, "cc");
    let bcc = parse_email_addresses(headers, "bcc");
    let reply_to = parse_email_addresses(headers, "reply-to");
    let sender = parse_email_addresses(headers, "sender");

    let message_id_header = headers.get_first("message-id").map(|s| vec![s.to_string()]);
    let in_reply_to = headers
        .get_first("in-reply-to")
        .map(|s| vec![s.to_string()]);
    let references = headers
        .get("references")
        .map(|refs| refs.iter().map(|s| s.to_string()).collect());

    // Parse Date header for sentAt
    let sent_at = parse_date_header(headers.get_first("date"));

    // Extract preview from message body (simplified)
    let preview = extract_preview(message).ok();

    // Default mailbox IDs (would come from storage metadata in production)
    let mut mailbox_ids = HashMap::new();
    mailbox_ids.insert("inbox".to_string(), true);

    // Default keywords (would come from message flags in production)
    let keywords = HashMap::new();

    Ok(Email {
        id: id.to_string(),
        blob_id,
        thread_id: None, // Threading not implemented yet
        mailbox_ids,
        keywords,
        size,
        received_at,
        message_id: message_id_header,
        in_reply_to,
        references,
        sender,
        from,
        to,
        cc,
        bcc,
        reply_to,
        subject,
        sent_at,
        has_attachment: detect_attachment(mail),
        preview,
        body_values: None,
        text_body: None,
        html_body: None,
        attachments: None,
    })
}

/// Parse email addresses from a header
fn parse_email_addresses(headers: &HeaderMap, header_name: &str) -> Option<Vec<EmailAddress>> {
    headers.get_first(header_name).map(|value| {
        // Simple parsing - in production would use proper RFC 2822 parser
        value
            .split(',')
            .filter_map(|addr| {
                let trimmed = addr.trim();
                if trimmed.is_empty() {
                    return None;
                }

                // Try to parse "Name <email@example.com>" format
                if let Some(start) = trimmed.find('<') {
                    if let Some(end) = trimmed.find('>') {
                        let name = trimmed[..start].trim().trim_matches('"');
                        let email = trimmed[start + 1..end].trim();
                        return Some(EmailAddress {
                            name: if name.is_empty() {
                                None
                            } else {
                                Some(name.to_string())
                            },
                            email: email.to_string(),
                        });
                    }
                }

                // Just an email address
                Some(EmailAddress::new(trimmed.to_string()))
            })
            .collect()
    })
}

/// Parse date header to DateTime
fn parse_date_header(date_str: Option<&str>) -> Option<chrono::DateTime<Utc>> {
    date_str.and_then(|s| {
        // Try RFC 2822 format
        chrono::DateTime::parse_from_rfc2822(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    })
}

/// Extract preview text from message
fn extract_preview(message: &MimeMessage) -> anyhow::Result<String> {
    let text = message.extract_text()?;
    let preview_len = 256.min(text.len());
    Ok(text[..preview_len].to_string())
}

/// Parse a string ID to MessageId
fn parse_message_id(id: &str) -> anyhow::Result<MessageId> {
    let uuid =
        uuid::Uuid::from_str(id).map_err(|e| anyhow::anyhow!("Invalid message ID: {}", e))?;
    Ok(MessageId::from_uuid(uuid))
}

/// Parse a string ID to MailboxId
fn parse_mailbox_id(id: &str) -> anyhow::Result<MailboxId> {
    let uuid =
        uuid::Uuid::from_str(id).map_err(|e| anyhow::anyhow!("Invalid mailbox ID: {}", e))?;
    Ok(MailboxId::from_uuid(uuid))
}

/// Build search criteria from email filter
#[allow(dead_code)]
fn build_search_criteria(
    filter: &crate::types::EmailFilterCondition,
) -> rusmes_storage::SearchCriteria {
    use rusmes_storage::SearchCriteria;

    // Build search criteria based on filter
    let mut criteria = Vec::new();

    if let Some(text) = &filter.text {
        criteria.push(SearchCriteria::Subject(text.clone()));
        criteria.push(SearchCriteria::Body(text.clone()));
    }

    if let Some(from) = &filter.from {
        criteria.push(SearchCriteria::From(from.clone()));
    }

    if let Some(to) = &filter.to {
        criteria.push(SearchCriteria::To(to.clone()));
    }

    if let Some(subject) = &filter.subject {
        criteria.push(SearchCriteria::Subject(subject.clone()));
    }

    if let Some(body) = &filter.body {
        criteria.push(SearchCriteria::Body(body.clone()));
    }

    if let Some(has_keyword) = &filter.has_keyword {
        match has_keyword.as_str() {
            "$seen" => criteria.push(SearchCriteria::Seen),
            "$flagged" => criteria.push(SearchCriteria::Flagged),
            "$draft" => {
                // Draft is not directly supported, would need custom handling
            }
            _ => {}
        }
    }

    if let Some(not_keyword) = &filter.not_keyword {
        match not_keyword.as_str() {
            "$seen" => criteria.push(SearchCriteria::Unseen),
            "$flagged" => criteria.push(SearchCriteria::Unflagged),
            _ => {}
        }
    }

    if criteria.is_empty() {
        SearchCriteria::All
    } else if criteria.len() == 1 {
        // Safe: we just confirmed len == 1
        criteria.into_iter().next().unwrap_or(SearchCriteria::All)
    } else {
        SearchCriteria::And(criteria)
    }
}

/// Get all messages from all mailboxes in the storage
/// This is a workaround since we don't have direct access to mailbox listing
async fn get_all_messages_from_all_mailboxes(
    message_store: &dyn MessageStore,
) -> anyhow::Result<Vec<String>> {
    use std::path::Path;
    use tokio::fs;

    let all_ids = Vec::new();

    // Since we're using FilesystemBackend, we need to scan the mailboxes directory directly
    // The configured storage path is /tmp/rusmes/mail
    let mailboxes_dir = Path::new("/tmp/rusmes/mail/mailboxes");

    tracing::debug!("Checking primary mailboxes path: {:?}", mailboxes_dir);
    if !fs::try_exists(mailboxes_dir).await.unwrap_or(false) {
        // Try alternative path (legacy)
        let alt_path = Path::new("/tmp/rusmes-storage/mailboxes");
        tracing::debug!(
            "Primary path doesn't exist, trying alternative: {:?}",
            alt_path
        );
        if fs::try_exists(alt_path).await.unwrap_or(false) {
            tracing::debug!("Using alternative path");
            return scan_mailboxes_directory(alt_path, message_store).await;
        }
        tracing::warn!("No mailboxes directory found!");
        return Ok(all_ids);
    }

    tracing::debug!("Using primary path");
    scan_mailboxes_directory(mailboxes_dir, message_store).await
}

/// Scan a mailboxes directory and collect all message IDs
async fn scan_mailboxes_directory(
    mailboxes_dir: &Path,
    message_store: &dyn MessageStore,
) -> anyhow::Result<Vec<String>> {
    use tokio::fs;

    let mut all_ids = Vec::new();

    tracing::debug!("Scanning mailboxes directory: {:?}", mailboxes_dir);

    let mut entries = match fs::read_dir(mailboxes_dir).await {
        Ok(e) => e,
        Err(err) => {
            tracing::error!("Failed to read mailboxes directory: {}", err);
            return Err(err.into());
        }
    };
    while let Some(entry) = match entries.next_entry().await {
        Ok(e) => e,
        Err(err) => {
            tracing::error!("Error reading directory entry: {}", err);
            return Err(err.into());
        }
    } {
        if let Ok(file_type) = entry.file_type().await {
            if file_type.is_dir() {
                // Try to parse the directory name as a MailboxId UUID
                if let Some(dir_name) = entry.file_name().to_str() {
                    tracing::debug!("Found directory: {}", dir_name);
                    if let Ok(_uuid) = uuid::Uuid::from_str(dir_name) {
                        tracing::debug!("Valid UUID mailbox: {}", dir_name);
                        // This is a valid mailbox ID - get its messages
                        if let Ok(mailbox_id) = parse_mailbox_id(dir_name) {
                            match message_store.get_mailbox_messages(&mailbox_id).await {
                                Ok(messages) => {
                                    tracing::debug!(
                                        "Found {} messages in mailbox {}",
                                        messages.len(),
                                        dir_name
                                    );
                                    for metadata in messages {
                                        let msg_id = metadata.message_id().to_string();
                                        tracing::debug!("Adding message ID: {}", msg_id);
                                        all_ids.push(msg_id);
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to get messages from mailbox {}: {}",
                                        dir_name,
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    tracing::debug!("Total message IDs found: {}", all_ids.len());
    Ok(all_ids)
}
