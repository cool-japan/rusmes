//! Email method implementations

use crate::blob::compute_blob_id;
use crate::methods::ensure_account_ownership;
use crate::types::{
    Email, EmailAddress, EmailGetRequest, EmailGetResponse, EmailQueryRequest, EmailQueryResponse,
    EmailSetRequest, EmailSetResponse, JmapSetError, Principal,
};
use bytes::Bytes;
use chrono::Utc;
use rusmes_proto::{HeaderMap, Mail, MessageId, MimeMessage};
use rusmes_storage::{MailboxId, MessageStore};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

/// Context required to populate all fields of a JMAP [`Email`] object.
///
/// Use [`EmailConversionContext::placeholder`] for parse-only callers where the
/// message has not been stored yet — this records the intent explicitly rather
/// than silently defaulting to wrong values.
pub(crate) struct EmailConversionContext<'a> {
    /// Content-addressed blob ID (SHA-256 hex). Use [`compute_blob_id`].
    pub blob_id: std::borrow::Cow<'a, str>,
    /// Delivery timestamp. Parsed from `Received:` header, falling back to
    /// `Date:` header, falling back to `Utc::now()`.
    pub received_at: chrono::DateTime<chrono::Utc>,
    /// All mailboxes the message lives in, as JMAP IDs.
    pub mailbox_ids: HashMap<String, bool>,
    /// JMAP keywords from `MessageFlags`.
    pub keywords: HashMap<String, bool>,
    /// RFC 5256 thread ID from `MessageStore::get_message_thread_id`, or `None`.
    pub thread_id: Option<String>,
}

impl<'a> EmailConversionContext<'a> {
    /// Intentional placeholder for parse-only contexts (message not stored).
    /// Uses `blob_id` as provided; all other fields use safe defaults.
    pub fn placeholder(blob_id: impl Into<std::borrow::Cow<'a, str>>) -> Self {
        let mut mailbox_ids = HashMap::new();
        mailbox_ids.insert("inbox".to_string(), true);
        Self {
            blob_id: blob_id.into(),
            received_at: chrono::Utc::now(),
            mailbox_ids,
            keywords: HashMap::new(),
            thread_id: None,
        }
    }
}

/// Convert storage [`MessageFlags`] to a JMAP keywords map.
///
/// Also re-exported as [`jmap_keywords_from_flags`] for external callers.
pub(crate) fn flags_to_keywords(flags: &rusmes_storage::MessageFlags) -> HashMap<String, bool> {
    let mut kw = HashMap::new();
    if flags.is_seen() {
        kw.insert("$seen".to_string(), true);
    }
    if flags.is_answered() {
        kw.insert("$answered".to_string(), true);
    }
    if flags.is_flagged() {
        kw.insert("$flagged".to_string(), true);
    }
    if flags.is_deleted() {
        kw.insert("$deleted".to_string(), true);
    }
    if flags.is_draft() {
        kw.insert("$draft".to_string(), true);
    }
    kw
}

/// Convert storage [`MessageFlags`] to JMAP keyword map per RFC 8621 §4.1.1.
///
/// Public alias of [`flags_to_keywords`] used by tests and other modules.
pub(crate) fn jmap_keywords_from_flags(
    flags: &rusmes_storage::MessageFlags,
) -> HashMap<String, bool> {
    flags_to_keywords(flags)
}

/// Handle Email/get method
pub async fn email_get(
    request: EmailGetRequest,
    message_store: &dyn MessageStore,
    principal: &Principal,
) -> anyhow::Result<EmailGetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;
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
                        // Build real context: keywords from persisted flags,
                        // thread_id from the thread index, blob_id as SHA-256
                        // of the message-ID string (stable, content-addressed).
                        let keywords = message_store
                            .get_message_flags(&message_id)
                            .await
                            .ok()
                            .flatten()
                            .map(|f| jmap_keywords_from_flags(&f))
                            .unwrap_or_default();

                        let thread_id = message_store
                            .get_message_thread_id(&message_id)
                            .await
                            .ok()
                            .flatten();

                        // Derive a stable blob ID from the storage message ID.
                        let blob_id_str = compute_blob_id(id.as_bytes());

                        // Parse received_at from Received: or Date: headers; fall back to now.
                        let headers = mail.message().headers();
                        let received_at = parse_received_header(headers)
                            .or_else(|| parse_date_header(headers.get_first("date")))
                            .unwrap_or_else(Utc::now);

                        let mut mailbox_ids = HashMap::new();
                        mailbox_ids.insert("inbox".to_string(), true);

                        let ctx = EmailConversionContext {
                            blob_id: std::borrow::Cow::Owned(blob_id_str),
                            received_at,
                            mailbox_ids,
                            keywords,
                            thread_id,
                        };

                        let email = convert_mail_to_email(&id, &mail, ctx).await?;
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

/// Build the plain-text body string from an `EmailSetObject`.
///
/// Walks `text_body` parts in order, looks each `part_id` up in `body_values`,
/// and concatenates the resulting text. Falls back to an empty string when no
/// text body is present.
fn build_body_text(email_obj: &crate::types::EmailSetObject) -> String {
    let body_values = match &email_obj.body_values {
        Some(bv) => bv,
        None => return String::new(),
    };

    let text_parts = match &email_obj.text_body {
        Some(parts) => parts,
        None => return String::new(),
    };

    let mut body = String::new();
    for part in text_parts {
        if let Some(bv) = body_values.get(&part.part_id) {
            body.push_str(&bv.value);
        }
    }
    body
}

/// Format an `EmailAddress` slice for an RFC 5322 header value.
fn format_addresses(addrs: &[crate::types::EmailAddress]) -> String {
    addrs
        .iter()
        .map(|a| {
            if let Some(name) = &a.name {
                format!("\"{}\" <{}>", name, a.email)
            } else {
                a.email.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build a [`Mail`] from an [`EmailSetObject`] supplied in Email/set create.
fn build_mail_from_set_object(email_obj: &crate::types::EmailSetObject) -> anyhow::Result<Mail> {
    use rusmes_proto::MessageBody;

    let mut headers = rusmes_proto::HeaderMap::new();

    if let Some(from) = &email_obj.from {
        if !from.is_empty() {
            headers.insert("from", format_addresses(from));
        }
    }
    if let Some(to) = &email_obj.to {
        if !to.is_empty() {
            headers.insert("to", format_addresses(to));
        }
    }
    if let Some(cc) = &email_obj.cc {
        if !cc.is_empty() {
            headers.insert("cc", format_addresses(cc));
        }
    }
    if let Some(bcc) = &email_obj.bcc {
        if !bcc.is_empty() {
            headers.insert("bcc", format_addresses(bcc));
        }
    }
    if let Some(reply_to) = &email_obj.reply_to {
        if !reply_to.is_empty() {
            headers.insert("reply-to", format_addresses(reply_to));
        }
    }
    if let Some(sender) = &email_obj.sender {
        if !sender.is_empty() {
            headers.insert("sender", format_addresses(sender));
        }
    }
    if let Some(subject) = &email_obj.subject {
        headers.insert("subject", subject.clone());
    }
    if let Some(sent_at) = &email_obj.sent_at {
        headers.insert("date", sent_at.to_rfc2822());
    } else {
        headers.insert("date", Utc::now().to_rfc2822());
    }
    if let Some(msg_ids) = &email_obj.message_id {
        if let Some(first) = msg_ids.first() {
            headers.insert("message-id", first.clone());
        }
    }
    if let Some(in_reply_to) = &email_obj.in_reply_to {
        if let Some(first) = in_reply_to.first() {
            headers.insert("in-reply-to", first.clone());
        }
    }
    if let Some(references) = &email_obj.references {
        if !references.is_empty() {
            headers.insert("references", references.join(" "));
        }
    }
    headers.insert("content-type", "text/plain; charset=utf-8");

    let body_text = build_body_text(email_obj);
    let body = MessageBody::Small(Bytes::from(body_text));
    let mime = rusmes_proto::MimeMessage::new(headers, body);

    Ok(Mail::new(None, vec![], mime, None, None))
}

/// Convert `HashMap<String, bool>` JMAP keywords to [`rusmes_storage::MessageFlags`].
fn keywords_to_flags(keywords: &HashMap<String, bool>) -> rusmes_storage::MessageFlags {
    let mut flags = rusmes_storage::MessageFlags::new();
    for (kw, &active) in keywords {
        if !active {
            continue;
        }
        match kw.to_lowercase().as_str() {
            "$seen" => flags.set_seen(true),
            "$answered" => flags.set_answered(true),
            "$flagged" => flags.set_flagged(true),
            "$deleted" => flags.set_deleted(true),
            "$draft" => flags.set_draft(true),
            other => flags.add_custom(other.to_string()),
        }
    }
    flags
}

/// Merge a JMAP JSON patch into the current `MessageFlags` of a message.
///
/// Supported patch paths:
/// - `/keywords` (full replacement map)
/// - `/keywords/$flagName` (single-flag toggle)
///
/// Returns the resulting `MessageFlags` and whether any keyword change was
/// detected. Non-keyword paths (e.g. `/mailboxIds/…`) are silently ignored
/// here; the caller handles mailbox-move operations separately.
fn apply_keyword_patch(
    current: rusmes_storage::MessageFlags,
    patch: &serde_json::Value,
) -> (rusmes_storage::MessageFlags, bool) {
    let obj = match patch.as_object() {
        Some(o) => o,
        None => return (current, false),
    };

    let mut flags = current;
    let mut changed = false;

    // Full `/keywords` replacement takes priority over per-flag patches.
    if let Some(kw_val) = obj.get("keywords") {
        if let Some(kw_map) = kw_val.as_object() {
            let converted: HashMap<String, bool> = kw_map
                .iter()
                .filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b)))
                .collect();
            flags = keywords_to_flags(&converted);
            changed = true;
        }
        return (flags, changed);
    }

    // Per-flag patches like `"/keywords/$Seen": true`.
    for (path, value) in obj {
        if let Some(flag_name) = path.strip_prefix("/keywords/") {
            let active = value.as_bool().unwrap_or(false);
            match flag_name.to_lowercase().as_str() {
                "$seen" => {
                    flags.set_seen(active);
                    changed = true;
                }
                "$answered" => {
                    flags.set_answered(active);
                    changed = true;
                }
                "$flagged" => {
                    flags.set_flagged(active);
                    changed = true;
                }
                "$deleted" => {
                    flags.set_deleted(active);
                    changed = true;
                }
                "$draft" => {
                    flags.set_draft(active);
                    changed = true;
                }
                other => {
                    if active {
                        flags.add_custom(other.to_string());
                    } else {
                        flags.remove_custom(other);
                    }
                    changed = true;
                }
            }
        }
    }

    (flags, changed)
}

/// Read current flags for a message by scanning its mailbox.
///
/// `MessageStore` has no direct `get_flags(&MessageId)` call, so we scan
/// `get_mailbox_messages` for the owning mailbox and match by message ID.
/// Returns `None` when the message is not found.
async fn read_message_flags(
    message_store: &dyn MessageStore,
    message_id: &MessageId,
    mailbox_id: &MailboxId,
) -> anyhow::Result<Option<rusmes_storage::MessageFlags>> {
    let messages = message_store.get_mailbox_messages(mailbox_id).await?;
    for meta in messages {
        if meta.message_id() == message_id {
            return Ok(Some(meta.flags().clone()));
        }
    }
    Ok(None)
}

/// Sentinel error type used to signal "message not found" through the error chain.
#[derive(Debug)]
struct NotFoundError(String);

impl std::fmt::Display for NotFoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "notFound: {}", self.0)
    }
}

impl std::error::Error for NotFoundError {}

/// Create a single email, returning the JMAP `Email` object on success.
async fn handle_email_create(
    message_store: &dyn MessageStore,
    creation_id: &str,
    email_obj: &crate::types::EmailSetObject,
) -> anyhow::Result<crate::types::Email> {
    // Must have at least one mailbox.
    let primary_mailbox_id_str = email_obj
        .mailbox_ids
        .iter()
        .find_map(|(k, v)| if *v { Some(k.clone()) } else { None })
        .ok_or_else(|| anyhow::anyhow!("mailboxIds must contain at least one true entry"))?;

    let primary_mailbox_id = parse_mailbox_id(&primary_mailbox_id_str)?;

    // Build Mail from the set object.
    let mail = build_mail_from_set_object(email_obj)?;

    // Append to primary mailbox.
    let metadata = message_store
        .append_message(&primary_mailbox_id, mail)
        .await?;

    let message_id = *metadata.message_id();
    let message_id_str = message_id.to_string();

    // Copy to additional mailboxes.
    for (mailbox_id_str, active) in &email_obj.mailbox_ids {
        if !active || mailbox_id_str == &primary_mailbox_id_str {
            continue;
        }
        if let Ok(extra_mailbox_id) = parse_mailbox_id(mailbox_id_str) {
            message_store
                .copy_messages(&[message_id], &extra_mailbox_id)
                .await?;
        }
    }

    // Set initial keywords/flags if any.
    if let Some(keywords) = &email_obj.keywords {
        if !keywords.is_empty() {
            let flags = keywords_to_flags(keywords);
            message_store.set_flags(&[message_id], flags).await?;
        }
    }

    // Fetch the stored message to build the JMAP Email response.
    let mail_fetched = message_store
        .get_message(&message_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("message disappeared after append (id={})", message_id))?;

    // Build context with real values from the creation request and storage.
    let blob_id_str = compute_blob_id(message_id_str.as_bytes());
    let headers = mail_fetched.message().headers();
    let received_at = parse_received_header(headers)
        .or_else(|| parse_date_header(headers.get_first("date")))
        .unwrap_or_else(Utc::now);

    let ctx = EmailConversionContext {
        blob_id: std::borrow::Cow::Owned(blob_id_str),
        received_at,
        mailbox_ids: email_obj.mailbox_ids.clone(),
        keywords: email_obj.keywords.clone().unwrap_or_default(),
        thread_id: metadata.thread_id.clone(),
    };

    let email = convert_mail_to_email(&message_id_str, &mail_fetched, ctx).await?;

    tracing::debug!(
        "Email/set create: creation_id={} -> message_id={}",
        creation_id,
        message_id_str
    );
    Ok(email)
}

/// Apply a JMAP JSON patch to an email (update flags / mailbox memberships).
async fn handle_email_update(
    message_store: &dyn MessageStore,
    id: &str,
    patch: &serde_json::Value,
) -> anyhow::Result<()> {
    let message_id = parse_message_id(id)?;

    // Verify the message exists.
    if message_store.get_message(&message_id).await?.is_none() {
        return Err(anyhow::anyhow!(NotFoundError(id.to_string())));
    }

    let obj = match patch.as_object() {
        Some(o) => o,
        None => return Ok(()),
    };

    // Resolve owning mailbox for flag read-modify-write (best-effort).
    //
    // JMAP RFC 8621 uses JSON Pointer patch paths:
    //   "/mailboxIds"        → full replacement (value is a map)
    //   "/mailboxIds/<uuid>" → per-mailbox toggle (value is bool)
    let owning_mailbox_id_opt: Option<MailboxId> = {
        // Check "/mailboxIds" full replacement first.
        if let Some(full) = obj.get("/mailboxIds") {
            if let Some(map) = full.as_object() {
                map.iter().find_map(|(k, v)| {
                    if v.as_bool() == Some(true) {
                        parse_mailbox_id(k).ok()
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        } else {
            // Check per-mailbox patch paths "/mailboxIds/<uuid>".
            let mut found = None;
            for (path, value) in obj {
                if let Some(mbx_id_str) = path.strip_prefix("/mailboxIds/") {
                    if value.as_bool() == Some(true) {
                        if let Ok(id) = parse_mailbox_id(mbx_id_str) {
                            found = Some(id);
                            break;
                        }
                    }
                }
            }
            found
        }
    };

    // Determine current flags by scanning the owning mailbox (best-effort).
    // When no mailbox context is available, start from empty flags.
    let current_flags = if let Some(ref mbx_id) = owning_mailbox_id_opt {
        read_message_flags(message_store, &message_id, mbx_id)
            .await?
            .unwrap_or_default()
    } else {
        rusmes_storage::MessageFlags::new()
    };

    let (new_flags, flags_changed) = apply_keyword_patch(current_flags, patch);
    if flags_changed {
        message_store.set_flags(&[message_id], new_flags).await?;
    }

    // --- mailboxIds patches ---
    // Full "/mailboxIds" replacement: ensure message exists in each listed mailbox.
    if let Some(full) = obj.get("/mailboxIds") {
        if let Some(map) = full.as_object() {
            for (mbx_id_str, value) in map {
                if value.as_bool() == Some(true) {
                    if let Ok(dest_id) = parse_mailbox_id(mbx_id_str) {
                        // Copy only if not already in the target mailbox.
                        let already_there = message_store
                            .get_mailbox_messages(&dest_id)
                            .await
                            .ok()
                            .map(|ms| ms.iter().any(|m| m.message_id() == &message_id))
                            .unwrap_or(false);
                        if !already_there {
                            let _ = message_store.copy_messages(&[message_id], &dest_id).await;
                        }
                    }
                }
            }
        }
    }

    // Per-mailbox patch "/mailboxIds/<uuid>": add or remove membership.
    for (path, value) in obj {
        if let Some(mbx_id_str) = path.strip_prefix("/mailboxIds/") {
            let active = value.as_bool().unwrap_or(false);
            if active {
                if let Ok(dest_id) = parse_mailbox_id(mbx_id_str) {
                    let already_there = message_store
                        .get_mailbox_messages(&dest_id)
                        .await
                        .ok()
                        .map(|ms| ms.iter().any(|m| m.message_id() == &message_id))
                        .unwrap_or(false);
                    if !already_there {
                        let _ = message_store.copy_messages(&[message_id], &dest_id).await;
                    }
                }
            } else if let Ok(src_id) = parse_mailbox_id(mbx_id_str) {
                // Remove from mailbox — only delete when confirmed present.
                let msgs = message_store.get_mailbox_messages(&src_id).await;
                if let Ok(msgs) = msgs {
                    let in_box = msgs.iter().any(|m| m.message_id() == &message_id);
                    if in_box {
                        message_store.delete_messages(&[message_id]).await?;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Permanently delete a single email by its JMAP ID.
async fn handle_email_destroy(message_store: &dyn MessageStore, id: &str) -> anyhow::Result<()> {
    let message_id = parse_message_id(id)?;

    // Existence check — JMAP destroy is not idempotent: missing = notFound.
    if message_store.get_message(&message_id).await?.is_none() {
        return Err(anyhow::anyhow!(NotFoundError(id.to_string())));
    }

    message_store.delete_messages(&[message_id]).await?;
    Ok(())
}

/// Map an update error to a JMAP set error type + description.
fn classify_update_error(err: &anyhow::Error) -> (String, String) {
    if err.downcast_ref::<NotFoundError>().is_some() {
        ("notFound".to_string(), err.to_string())
    } else if err.to_string().contains("Invalid message ID")
        || err.to_string().contains("Invalid mailbox ID")
    {
        ("invalidArguments".to_string(), err.to_string())
    } else {
        ("serverFail".to_string(), err.to_string())
    }
}

/// Map a destroy error to a JMAP set error type + description.
fn classify_destroy_error(err: &anyhow::Error) -> (String, String) {
    if err.downcast_ref::<NotFoundError>().is_some() {
        ("notFound".to_string(), err.to_string())
    } else if err.to_string().contains("Invalid message ID") {
        ("invalidArguments".to_string(), err.to_string())
    } else {
        ("serverFail".to_string(), err.to_string())
    }
}

/// Handle Email/set method
pub async fn email_set(
    request: EmailSetRequest,
    message_store: &dyn MessageStore,
    principal: &Principal,
) -> anyhow::Result<EmailSetResponse> {
    ensure_account_ownership(&request.account_id, principal)?;
    let mut created: HashMap<String, crate::types::Email> = HashMap::new();
    let mut updated: HashMap<String, Option<crate::types::Email>> = HashMap::new();
    let mut destroyed: Vec<String> = Vec::new();
    let mut not_created = HashMap::new();
    let mut not_updated = HashMap::new();
    let mut not_destroyed = HashMap::new();

    let old_state = Utc::now().timestamp().to_string();

    // -----------------------------------------------------------------------
    // Handle creates
    // -----------------------------------------------------------------------
    if let Some(create_map) = request.create {
        for (creation_id, email_obj) in create_map {
            match handle_email_create(message_store, &creation_id, &email_obj).await {
                Ok(email) => {
                    created.insert(creation_id, email);
                }
                Err(err) => {
                    tracing::warn!("Email create failed for {}: {}", creation_id, err);
                    not_created.insert(
                        creation_id,
                        JmapSetError {
                            error_type: "serverFail".to_string(),
                            description: Some(err.to_string()),
                        },
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Handle updates
    // -----------------------------------------------------------------------
    if let Some(update_map) = request.update {
        for (id, patch) in update_map {
            match handle_email_update(message_store, &id, &patch).await {
                Ok(()) => {
                    updated.insert(id, None);
                }
                Err(err) => {
                    let (error_type, description) = classify_update_error(&err);
                    tracing::warn!("Email update failed for {}: {}", id, err);
                    not_updated.insert(
                        id,
                        JmapSetError {
                            error_type,
                            description: Some(description),
                        },
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Handle destroys
    // -----------------------------------------------------------------------
    if let Some(destroy_ids) = request.destroy {
        for id in destroy_ids {
            match handle_email_destroy(message_store, &id).await {
                Ok(()) => {
                    destroyed.push(id);
                }
                Err(err) => {
                    let (error_type, description) = classify_destroy_error(&err);
                    tracing::warn!("Email destroy failed for {}: {}", id, err);
                    not_destroyed.insert(
                        id,
                        JmapSetError {
                            error_type,
                            description: Some(description),
                        },
                    );
                }
            }
        }
    }

    let new_state = Utc::now().timestamp().to_string();

    Ok(EmailSetResponse {
        account_id: request.account_id,
        old_state,
        new_state,
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
    principal: &Principal,
) -> anyhow::Result<EmailQueryResponse> {
    ensure_account_ownership(&request.account_id, principal)?;
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

/// Convert a Mail object to an Email JMAP object.
///
/// All context-dependent fields (blob_id, received_at, mailbox_ids, keywords,
/// thread_id) are taken from `ctx` rather than being hardcoded.  Callers that
/// store the message first should build an [`EmailConversionContext`] with real
/// values; parse-only callers should use [`EmailConversionContext::placeholder`].
pub(crate) async fn convert_mail_to_email(
    id: &str,
    mail: &Mail,
    ctx: EmailConversionContext<'_>,
) -> anyhow::Result<Email> {
    let message = mail.message();
    let headers = message.headers();

    // Extract basic metadata
    let size = mail.size() as u64;
    let blob_id = ctx.blob_id.into_owned();
    let received_at = ctx.received_at;
    let mailbox_ids = ctx.mailbox_ids;
    let keywords = ctx.keywords;

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
    let preview = extract_preview(message).await.ok();

    Ok(Email {
        id: id.to_string(),
        blob_id,
        thread_id: ctx.thread_id,
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
pub(crate) fn parse_email_addresses(
    headers: &HeaderMap,
    header_name: &str,
) -> Option<Vec<EmailAddress>> {
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
pub(crate) fn parse_date_header(date_str: Option<&str>) -> Option<chrono::DateTime<Utc>> {
    date_str.and_then(|s| {
        // Try RFC 2822 format
        chrono::DateTime::parse_from_rfc2822(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    })
}

/// Parse the topmost `Received:` header for a delivery timestamp.
///
/// RFC 5321 appends `Received:` headers in LIFO order; the topmost entry is
/// the most recent (innermost) MTA.  The timestamp is the `; <date>` suffix.
///
/// Returns `None` when no `Received:` header is present or none carries a
/// parseable date.
fn parse_received_header(headers: &HeaderMap) -> Option<chrono::DateTime<Utc>> {
    headers.get("received").and_then(|values| {
        values.iter().find_map(|v| {
            // The date follows the last semicolon in the header value.
            v.rfind(';').and_then(|pos| {
                let date_part = v[pos + 1..].trim();
                chrono::DateTime::parse_from_rfc2822(date_part)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            })
        })
    })
}

/// Extract preview text from message
async fn extract_preview(message: &MimeMessage) -> anyhow::Result<String> {
    let text = message.extract_text().await?;
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
pub(crate) fn parse_mailbox_id(id: &str) -> anyhow::Result<MailboxId> {
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

#[cfg(test)]
mod email_context_tests {
    use super::*;
    use crate::blob::compute_blob_id;
    use rusmes_proto::MimeMessage;

    /// Build a minimal `Mail` from raw RFC 5322 bytes.
    fn mail_from_raw(raw: &[u8]) -> Mail {
        let mime = MimeMessage::parse_from_bytes(raw).expect("test: parse raw mail");
        Mail::new(None, vec![], mime, None, None)
    }

    #[test]
    fn test_compute_blob_id_deterministic() {
        let id1 = compute_blob_id(b"hello world");
        let id2 = compute_blob_id(b"hello world");
        assert_eq!(id1, id2);
        // SHA-256 hex is always 64 characters.
        assert_eq!(id1.len(), 64);
    }

    #[test]
    fn test_compute_blob_id_differs_for_different_inputs() {
        let id1 = compute_blob_id(b"foo");
        let id2 = compute_blob_id(b"bar");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_jmap_keywords_from_flags_canonical_mapping() {
        let mut flags = rusmes_storage::MessageFlags::new();
        flags.set_seen(true);
        flags.set_flagged(true);
        let kw = jmap_keywords_from_flags(&flags);
        assert_eq!(kw.get("$seen"), Some(&true));
        assert_eq!(kw.get("$flagged"), Some(&true));
        assert_eq!(kw.get("$answered"), None);
    }

    #[test]
    fn test_jmap_keywords_from_flags_all_set() {
        let mut flags = rusmes_storage::MessageFlags::new();
        flags.set_seen(true);
        flags.set_answered(true);
        flags.set_flagged(true);
        flags.set_deleted(true);
        flags.set_draft(true);
        let kw = jmap_keywords_from_flags(&flags);
        assert_eq!(kw.get("$seen"), Some(&true));
        assert_eq!(kw.get("$answered"), Some(&true));
        assert_eq!(kw.get("$flagged"), Some(&true));
        assert_eq!(kw.get("$deleted"), Some(&true));
        assert_eq!(kw.get("$draft"), Some(&true));
    }

    #[tokio::test]
    async fn test_convert_mail_to_email_uses_real_blob_id() {
        let raw = b"Subject: Test\r\nFrom: alice@example.com\r\n\r\nHello";
        let mail = mail_from_raw(raw);
        // The context carries the blob_id; the function must round-trip it
        // unchanged (callers compute SHA-256 before building the context).
        let caller_blob_id = compute_blob_id(b"test-id-123");
        let ctx = EmailConversionContext {
            blob_id: std::borrow::Cow::Owned(caller_blob_id.clone()),
            received_at: Utc::now(),
            mailbox_ids: [("inbox".to_string(), true)].into_iter().collect(),
            keywords: HashMap::new(),
            thread_id: None,
        };
        let email = convert_mail_to_email("msg-1", &mail, ctx)
            .await
            .expect("convert should succeed");
        assert_eq!(email.blob_id, caller_blob_id);
    }

    #[tokio::test]
    async fn test_convert_mail_to_email_received_at_from_context() {
        use chrono::TimeZone;
        let fixed_time = chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let raw = b"Subject: Time test\r\n\r\nBody";
        let mail = mail_from_raw(raw);
        let ctx = EmailConversionContext {
            blob_id: std::borrow::Cow::Borrowed("blob-time-test"),
            received_at: fixed_time,
            mailbox_ids: [("inbox".to_string(), true)].into_iter().collect(),
            keywords: HashMap::new(),
            thread_id: None,
        };
        let email = convert_mail_to_email("msg-2", &mail, ctx)
            .await
            .expect("convert should succeed");
        assert_eq!(email.received_at, fixed_time);
    }

    #[tokio::test]
    async fn test_convert_mail_to_email_keywords_from_context() {
        let raw = b"Subject: Keywords test\r\n\r\nBody";
        let mail = mail_from_raw(raw);
        let mut keywords = HashMap::new();
        keywords.insert("$seen".to_string(), true);
        let ctx = EmailConversionContext {
            blob_id: std::borrow::Cow::Borrowed("blob-kw"),
            received_at: Utc::now(),
            mailbox_ids: [("inbox".to_string(), true)].into_iter().collect(),
            keywords,
            thread_id: None,
        };
        let email = convert_mail_to_email("msg-3", &mail, ctx)
            .await
            .expect("convert should succeed");
        assert_eq!(email.keywords.get("$seen"), Some(&true));
    }

    #[tokio::test]
    async fn test_convert_mail_to_email_thread_id_present() {
        let raw = b"Subject: Thread test\r\n\r\nBody";
        let mail = mail_from_raw(raw);
        let ctx = EmailConversionContext {
            blob_id: std::borrow::Cow::Borrowed("blob-thread"),
            received_at: Utc::now(),
            mailbox_ids: [("inbox".to_string(), true)].into_iter().collect(),
            keywords: HashMap::new(),
            thread_id: Some("T123".to_string()),
        };
        let email = convert_mail_to_email("msg-4", &mail, ctx)
            .await
            .expect("convert should succeed");
        assert_eq!(email.thread_id, Some("T123".to_string()));
    }

    #[tokio::test]
    async fn test_convert_mail_to_email_mailbox_ids_multi() {
        let raw = b"Subject: Mailbox test\r\n\r\nBody";
        let mail = mail_from_raw(raw);
        let mut mailbox_ids = HashMap::new();
        mailbox_ids.insert("inbox".to_string(), true);
        mailbox_ids.insert("starred".to_string(), true);
        let ctx = EmailConversionContext {
            blob_id: std::borrow::Cow::Borrowed("blob-mb"),
            received_at: Utc::now(),
            mailbox_ids,
            keywords: HashMap::new(),
            thread_id: None,
        };
        let email = convert_mail_to_email("msg-5", &mail, ctx)
            .await
            .expect("convert should succeed");
        assert_eq!(email.mailbox_ids.get("inbox"), Some(&true));
        assert_eq!(email.mailbox_ids.get("starred"), Some(&true));
    }

    #[tokio::test]
    async fn test_convert_mail_to_email_thread_id_none() {
        let raw = b"Subject: No thread\r\n\r\nBody";
        let mail = mail_from_raw(raw);
        let ctx = EmailConversionContext::placeholder("blob-no-thread");
        let email = convert_mail_to_email("msg-6", &mail, ctx)
            .await
            .expect("convert should succeed");
        assert_eq!(email.thread_id, None);
    }

    #[test]
    fn test_placeholder_context_has_inbox() {
        let ctx = EmailConversionContext::placeholder("some-blob-id");
        assert_eq!(ctx.mailbox_ids.get("inbox"), Some(&true));
        assert!(ctx.keywords.is_empty());
        assert!(ctx.thread_id.is_none());
        assert_eq!(ctx.blob_id, "some-blob-id");
    }
}
