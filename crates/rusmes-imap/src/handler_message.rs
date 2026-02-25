//! IMAP message command handlers
//!
//! Covers: FETCH, STORE, SEARCH, APPEND, COPY, MOVE, EXPUNGE, CLOSE,
//!         UID FETCH, UID STORE, UID SEARCH, UID COPY, UID MOVE, UID EXPUNGE

use crate::command::{StoreMode, UidSubcommand};
use crate::handler::HandlerContext;
use crate::response::ImapResponse;
use crate::session::{ImapSession, ImapState};
use rusmes_proto::MessageId;
use rusmes_storage::{MessageFlags, MessageMetadata, SearchCriteria};

/// Handle FETCH command
pub(crate) async fn handle_fetch(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    sequence: &str,
    items: &[String],
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get all messages in the mailbox
    let all_messages = ctx.message_store.get_mailbox_messages(mailbox_id).await?;

    // Parse sequence set and map to messages
    let sequence_numbers = parse_sequence_numbers(sequence, all_messages.len())?;

    // Fetch each message
    let mut responses = Vec::new();
    for seq_num in sequence_numbers {
        // Sequence numbers are 1-based, vector indices are 0-based
        if seq_num > 0 && seq_num <= all_messages.len() {
            let metadata = &all_messages[seq_num - 1];

            // Get the full message content
            if let Some(mail) = ctx.message_store.get_message(metadata.message_id()).await? {
                // Build FETCH response based on requested items
                let fetch_items = build_fetch_items(&mail, metadata, items);
                responses.push(format!("* {} FETCH ({})", seq_num, fetch_items));
            }
        }
    }

    // Combine responses
    let mut full_response = responses.join("\r\n");
    if !full_response.is_empty() {
        full_response.push_str("\r\n");
    }
    full_response.push_str(&format!("{} OK FETCH completed", tag));

    Ok(ImapResponse::new(None, "", full_response))
}

/// Handle STORE command
pub(crate) async fn handle_store(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    sequence: &str,
    mode: StoreMode,
    flags: &[String],
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    match session.state() {
        ImapState::Selected { .. } => {}
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    }

    // Parse sequence set
    let message_ids = parse_sequence_set(sequence)?;

    // Build MessageFlags from string flags
    let msg_flags = build_message_flags(flags);

    // Apply flags based on mode
    // For now, we'll just set the flags (simplified implementation)
    // In a real implementation, we'd need to handle +FLAGS and -FLAGS properly
    match mode {
        StoreMode::Replace => {
            ctx.message_store.set_flags(&message_ids, msg_flags).await?;
        }
        StoreMode::Add => {
            // Would need to fetch current flags and merge
            ctx.message_store.set_flags(&message_ids, msg_flags).await?;
        }
        StoreMode::Remove => {
            // Would need to fetch current flags and remove
            ctx.message_store.set_flags(&message_ids, msg_flags).await?;
        }
    }

    Ok(ImapResponse::ok(tag, "STORE completed"))
}

/// Handle SEARCH command
pub(crate) async fn handle_search(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    criteria: &[String],
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Parse search criteria
    let search_criteria = parse_search_criteria(criteria);

    // Perform search
    let message_ids = ctx
        .message_store
        .search(mailbox_id, search_criteria)
        .await?;

    // Build response
    let ids_str: Vec<String> = message_ids.iter().map(|id| id.to_string()).collect();
    let response = format!(
        "* SEARCH {}\r\n{} OK SEARCH completed",
        ids_str.join(" "),
        tag
    );

    Ok(ImapResponse::new(None, "", response))
}

/// Handle APPEND command
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_append(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    mailbox: &str,
    flags: &[String],
    _date_time: Option<&str>,
    message_literal: &[u8],
) -> anyhow::Result<ImapResponse> {
    // Must be authenticated
    if !matches!(
        session.state(),
        ImapState::Authenticated | ImapState::Selected { .. }
    ) {
        return Ok(ImapResponse::no(tag, "Not authenticated"));
    }

    // Get username from session
    let username = match &session.username {
        Some(u) => u.clone(),
        None => return Ok(ImapResponse::no(tag, "No username in session")),
    };

    // Find the mailbox
    let mailboxes = ctx.mailbox_store.list_mailboxes(&username).await?;
    let mailbox_obj = mailboxes.iter().find(|m| m.path().name() == Some(mailbox));

    let mailbox_id = match mailbox_obj {
        Some(mb) => *mb.id(),
        None => return Ok(ImapResponse::no(tag, "[TRYCREATE] Mailbox does not exist")),
    };

    // Parse the message literal into a Mail object
    // For APPEND, we need to parse the raw message data
    let message_data = bytes::Bytes::from(message_literal.to_vec());

    // Parse headers and body from the raw message
    let (headers, body) = parse_message_data(&message_data)?;

    // Create MimeMessage
    use rusmes_proto::{MessageBody, MimeMessage};
    let mime_message = MimeMessage::new(headers, MessageBody::Small(body));

    // Extract sender and recipients from headers
    let sender = extract_sender_from_headers(mime_message.headers());
    let recipients = extract_recipients_from_headers(mime_message.headers());

    // Create Mail object
    use rusmes_proto::Mail;
    let mut mail = Mail::new(sender, recipients, mime_message, None, None);

    // Set message state to LocalDelivery since we're storing directly
    use rusmes_proto::MailState;
    mail.state = MailState::LocalDelivery;

    // Append message to mailbox
    let metadata = ctx.message_store.append_message(&mailbox_id, mail).await?;

    // Update flags if provided
    if !flags.is_empty() {
        let msg_flags = build_message_flags(flags);
        ctx.message_store
            .set_flags(&[*metadata.message_id()], msg_flags)
            .await?;
    }

    // Return success with APPENDUID response code (RFC 4315)
    let uid_validity = mailbox_obj.map(|mb| mb.uid_validity()).unwrap_or(0);
    Ok(ImapResponse::ok(
        tag,
        format!(
            "[APPENDUID {} {}] APPEND completed",
            uid_validity,
            metadata.uid()
        ),
    ))
}

/// Handle COPY command
pub(crate) async fn handle_copy(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    sequence: &str,
    dest_mailbox: &str,
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let _source_mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get username from session
    let username = match &session.username {
        Some(u) => u.clone(),
        None => return Ok(ImapResponse::no(tag, "No username in session")),
    };

    // Find the destination mailbox
    let mailboxes = ctx.mailbox_store.list_mailboxes(&username).await?;
    let dest_mailbox_obj = mailboxes
        .iter()
        .find(|m| m.path().name() == Some(dest_mailbox));

    let dest_mailbox_id = match dest_mailbox_obj {
        Some(mb) => mb.id(),
        None => {
            return Ok(ImapResponse::no(
                tag,
                "[TRYCREATE] Destination mailbox does not exist",
            ))
        }
    };

    // Parse sequence set
    let message_ids = parse_sequence_set(sequence)?;

    if message_ids.is_empty() {
        return Ok(ImapResponse::ok(tag, "COPY completed (no messages)"));
    }

    // Copy messages to destination mailbox
    let copied_metadata = ctx
        .message_store
        .copy_messages(&message_ids, dest_mailbox_id)
        .await?;

    // Build COPYUID response (RFC 4315)
    // Format: [COPYUID <uidvalidity> <source-uids> <dest-uids>]
    if !copied_metadata.is_empty() {
        let source_uids: Vec<String> = message_ids.iter().map(|id| id.to_string()).collect();
        let dest_uids: Vec<String> = copied_metadata
            .iter()
            .map(|m| m.uid().to_string())
            .collect();

        let uid_validity = dest_mailbox_obj.map(|mb| mb.uid_validity()).unwrap_or(0);
        Ok(ImapResponse::ok(
            tag,
            format!(
                "[COPYUID {} {} {}] COPY completed",
                uid_validity,
                source_uids.join(","),
                dest_uids.join(",")
            ),
        ))
    } else {
        Ok(ImapResponse::ok(tag, "COPY completed"))
    }
}

/// Handle MOVE command (RFC 6851)
pub(crate) async fn handle_move(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    sequence: &str,
    dest_mailbox: &str,
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let _source_mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get username from session
    let username = match &session.username {
        Some(u) => u.clone(),
        None => return Ok(ImapResponse::no(tag, "No username in session")),
    };

    // Find the destination mailbox
    let mailboxes = ctx.mailbox_store.list_mailboxes(&username).await?;
    let dest_mailbox_obj = mailboxes
        .iter()
        .find(|m| m.path().name() == Some(dest_mailbox));

    let dest_mailbox_id = match dest_mailbox_obj {
        Some(mb) => mb.id(),
        None => {
            return Ok(ImapResponse::no(
                tag,
                "[TRYCREATE] Destination mailbox does not exist",
            ))
        }
    };

    // Parse sequence set
    let message_ids = parse_sequence_set(sequence)?;

    if message_ids.is_empty() {
        return Ok(ImapResponse::ok(tag, "MOVE completed (no messages)"));
    }

    // Copy messages to destination mailbox
    let copied_metadata = ctx
        .message_store
        .copy_messages(&message_ids, dest_mailbox_id)
        .await?;

    // Mark messages as deleted in source mailbox (implicit expunge for MOVE)
    let mut delete_flags = MessageFlags::new();
    delete_flags.set_deleted(true);
    ctx.message_store
        .set_flags(&message_ids, delete_flags)
        .await?;

    // Actually delete the messages from the source mailbox
    ctx.message_store.delete_messages(&message_ids).await?;

    // Build MOVEUID response (similar to COPYUID but for MOVE)
    // Format: [COPYUID <uidvalidity> <source-uids> <dest-uids>]
    // Note: RFC 6851 still uses COPYUID response code for MOVE
    if !copied_metadata.is_empty() {
        let source_uids: Vec<String> = message_ids.iter().map(|id| id.to_string()).collect();
        let dest_uids: Vec<String> = copied_metadata
            .iter()
            .map(|m| m.uid().to_string())
            .collect();

        let uid_validity = dest_mailbox_obj.map(|mb| mb.uid_validity()).unwrap_or(0);
        Ok(ImapResponse::ok(
            tag,
            format!(
                "[COPYUID {} {} {}] MOVE completed",
                uid_validity,
                source_uids.join(","),
                dest_uids.join(",")
            ),
        ))
    } else {
        Ok(ImapResponse::ok(tag, "MOVE completed"))
    }
}

/// Handle EXPUNGE command (RFC 9051 Section 6.4.3)
/// Permanently removes all messages with \Deleted flag from the selected mailbox
pub(crate) async fn handle_expunge(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get all messages in the mailbox
    let messages = ctx.message_store.get_mailbox_messages(mailbox_id).await?;

    // Find messages with \Deleted flag and collect their sequence numbers
    let mut deleted_messages = Vec::new();
    let mut expunge_responses = Vec::new();

    for (seq_num, metadata) in messages.iter().enumerate() {
        if metadata.flags().is_deleted() {
            deleted_messages.push(*metadata.message_id());
            // IMAP sequence numbers are 1-based
            expunge_responses.push(format!("* {} EXPUNGE", seq_num + 1));
        }
    }

    // Delete the messages from storage
    if !deleted_messages.is_empty() {
        ctx.message_store.delete_messages(&deleted_messages).await?;
    }

    // Build response with untagged EXPUNGE responses
    let mut full_response = expunge_responses.join("\r\n");
    if !full_response.is_empty() {
        full_response.push_str("\r\n");
    }
    full_response.push_str(&format!("{} OK EXPUNGE completed", tag));

    Ok(ImapResponse::new(None, "", full_response))
}

/// Handle CLOSE command (RFC 9051 Section 6.4.2)
/// Performs implicit EXPUNGE and then deselects the mailbox
pub(crate) async fn handle_close(
    ctx: &HandlerContext,
    session: &mut ImapSession,
    tag: &str,
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Perform implicit expunge (delete messages with \Deleted flag)
    let messages = ctx.message_store.get_mailbox_messages(mailbox_id).await?;
    let deleted_messages: Vec<MessageId> = messages
        .iter()
        .filter(|m| m.flags().is_deleted())
        .map(|m| *m.message_id())
        .collect();

    if !deleted_messages.is_empty() {
        ctx.message_store.delete_messages(&deleted_messages).await?;
    }

    // Deselect mailbox - return to Authenticated state
    session.state = ImapState::Authenticated;

    // CLOSE does not send untagged EXPUNGE responses (unlike EXPUNGE command)
    Ok(ImapResponse::ok(tag, "CLOSE completed"))
}

/// Handle UID command (RFC 9051 Section 6.4.8)
pub(crate) async fn handle_uid(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    subcommand: &UidSubcommand,
) -> anyhow::Result<ImapResponse> {
    // All UID commands require Selected state
    match session.state() {
        ImapState::Selected { .. } => {}
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    }

    match subcommand {
        UidSubcommand::Fetch { sequence, items } => {
            handle_uid_fetch(ctx, session, tag, sequence, items).await
        }
        UidSubcommand::Store {
            sequence,
            mode,
            flags,
        } => handle_uid_store(ctx, session, tag, sequence, mode.clone(), flags).await,
        UidSubcommand::Search { criteria } => handle_uid_search(ctx, session, tag, criteria).await,
        UidSubcommand::Copy { sequence, mailbox } => {
            handle_uid_copy(ctx, session, tag, sequence, mailbox).await
        }
        UidSubcommand::Move { sequence, mailbox } => {
            handle_uid_move(ctx, session, tag, sequence, mailbox).await
        }
        UidSubcommand::Expunge { sequence } => {
            handle_uid_expunge(ctx, session, tag, sequence).await
        }
    }
}

/// Handle UID FETCH command
async fn handle_uid_fetch(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    uid_sequence: &str,
    items: &[String],
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get all messages in mailbox
    let all_metadata = ctx.message_store.get_mailbox_messages(mailbox_id).await?;

    // Parse UID sequence set and filter messages by UID
    let uid_set = parse_uid_sequence_set(uid_sequence, &all_metadata)?;
    let matching_metadata: Vec<_> = all_metadata
        .iter()
        .filter(|m| uid_set.contains(&m.uid()))
        .collect();

    // Fetch each message
    let mut responses = Vec::new();
    for (seq_num, metadata) in matching_metadata.iter().enumerate() {
        if let Some(mail) = ctx.message_store.get_message(metadata.message_id()).await? {
            // Build FETCH response based on requested items
            let fetch_items = build_fetch_items(&mail, metadata, items);
            // Include UID in response (already included in fetch_items if requested)
            // Sequence number is 1-based
            responses.push(format!("* {} FETCH ({})", seq_num + 1, fetch_items));
        }
    }

    // Combine responses
    let mut full_response = responses.join("\r\n");
    if !full_response.is_empty() {
        full_response.push_str("\r\n");
    }
    full_response.push_str(&format!("{} OK UID FETCH completed", tag));

    Ok(ImapResponse::new(None, "", full_response))
}

/// Handle UID STORE command
async fn handle_uid_store(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    uid_sequence: &str,
    mode: StoreMode,
    flags: &[String],
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get all messages in mailbox
    let all_metadata = ctx.message_store.get_mailbox_messages(mailbox_id).await?;

    // Parse UID sequence set and filter messages by UID
    let uid_set = parse_uid_sequence_set(uid_sequence, &all_metadata)?;
    let message_ids: Vec<MessageId> = all_metadata
        .iter()
        .filter(|m| uid_set.contains(&m.uid()))
        .map(|m| *m.message_id())
        .collect();

    if message_ids.is_empty() {
        return Ok(ImapResponse::ok(tag, "UID STORE completed (no messages)"));
    }

    // Build MessageFlags from string flags
    let msg_flags = build_message_flags(flags);

    // Apply flags based on mode
    match mode {
        StoreMode::Replace => {
            ctx.message_store.set_flags(&message_ids, msg_flags).await?;
        }
        StoreMode::Add => {
            // Would need to fetch current flags and merge
            ctx.message_store.set_flags(&message_ids, msg_flags).await?;
        }
        StoreMode::Remove => {
            // Would need to fetch current flags and remove
            ctx.message_store.set_flags(&message_ids, msg_flags).await?;
        }
    }

    Ok(ImapResponse::ok(tag, "UID STORE completed"))
}

/// Handle UID SEARCH command
async fn handle_uid_search(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    criteria: &[String],
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Parse search criteria
    let search_criteria = parse_search_criteria(criteria);

    // Perform search to get message IDs
    let message_ids = ctx
        .message_store
        .search(mailbox_id, search_criteria)
        .await?;

    // Get all messages to map message IDs to UIDs
    let all_metadata = ctx.message_store.get_mailbox_messages(mailbox_id).await?;
    let uids: Vec<String> = all_metadata
        .iter()
        .filter(|m| message_ids.contains(m.message_id()))
        .map(|m| m.uid().to_string())
        .collect();

    // Build response with UIDs instead of sequence numbers
    let response = format!(
        "* SEARCH {}\r\n{} OK UID SEARCH completed",
        uids.join(" "),
        tag
    );

    Ok(ImapResponse::new(None, "", response))
}

/// Handle UID COPY command
async fn handle_uid_copy(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    uid_sequence: &str,
    dest_mailbox: &str,
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let source_mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get username from session
    let username = match &session.username {
        Some(u) => u.clone(),
        None => return Ok(ImapResponse::no(tag, "No username in session")),
    };

    // Find the destination mailbox
    let mailboxes = ctx.mailbox_store.list_mailboxes(&username).await?;
    let dest_mailbox_obj = mailboxes
        .iter()
        .find(|m| m.path().name() == Some(dest_mailbox));

    let dest_mailbox_id = match dest_mailbox_obj {
        Some(mb) => mb.id(),
        None => {
            return Ok(ImapResponse::no(
                tag,
                "[TRYCREATE] Destination mailbox does not exist",
            ))
        }
    };

    // Get all messages in source mailbox
    let all_metadata = ctx
        .message_store
        .get_mailbox_messages(source_mailbox_id)
        .await?;

    // Parse UID sequence set and filter messages by UID
    let uid_set = parse_uid_sequence_set(uid_sequence, &all_metadata)?;
    let message_ids: Vec<MessageId> = all_metadata
        .iter()
        .filter(|m| uid_set.contains(&m.uid()))
        .map(|m| *m.message_id())
        .collect();

    if message_ids.is_empty() {
        return Ok(ImapResponse::ok(tag, "UID COPY completed (no messages)"));
    }

    // Copy messages to destination mailbox
    let copied_metadata = ctx
        .message_store
        .copy_messages(&message_ids, dest_mailbox_id)
        .await?;

    // Build COPYUID response (RFC 4315)
    if !copied_metadata.is_empty() {
        let source_uids: Vec<String> = all_metadata
            .iter()
            .filter(|m| message_ids.contains(m.message_id()))
            .map(|m| m.uid().to_string())
            .collect();
        let dest_uids: Vec<String> = copied_metadata
            .iter()
            .map(|m| m.uid().to_string())
            .collect();

        let uid_validity = dest_mailbox_obj.map(|mb| mb.uid_validity()).unwrap_or(0);
        Ok(ImapResponse::ok(
            tag,
            format!(
                "[COPYUID {} {} {}] UID COPY completed",
                uid_validity,
                source_uids.join(","),
                dest_uids.join(",")
            ),
        ))
    } else {
        Ok(ImapResponse::ok(tag, "UID COPY completed"))
    }
}

/// Handle UID MOVE command
async fn handle_uid_move(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    uid_sequence: &str,
    dest_mailbox: &str,
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let source_mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get username from session
    let username = match &session.username {
        Some(u) => u.clone(),
        None => return Ok(ImapResponse::no(tag, "No username in session")),
    };

    // Find the destination mailbox
    let mailboxes = ctx.mailbox_store.list_mailboxes(&username).await?;
    let dest_mailbox_obj = mailboxes
        .iter()
        .find(|m| m.path().name() == Some(dest_mailbox));

    let dest_mailbox_id = match dest_mailbox_obj {
        Some(mb) => mb.id(),
        None => {
            return Ok(ImapResponse::no(
                tag,
                "[TRYCREATE] Destination mailbox does not exist",
            ))
        }
    };

    // Get all messages in source mailbox
    let all_metadata = ctx
        .message_store
        .get_mailbox_messages(source_mailbox_id)
        .await?;

    // Parse UID sequence set and filter messages by UID
    let uid_set = parse_uid_sequence_set(uid_sequence, &all_metadata)?;
    let message_ids: Vec<MessageId> = all_metadata
        .iter()
        .filter(|m| uid_set.contains(&m.uid()))
        .map(|m| *m.message_id())
        .collect();

    if message_ids.is_empty() {
        return Ok(ImapResponse::ok(tag, "UID MOVE completed (no messages)"));
    }

    // Copy messages to destination mailbox
    let copied_metadata = ctx
        .message_store
        .copy_messages(&message_ids, dest_mailbox_id)
        .await?;

    // Mark messages as deleted in source mailbox
    let mut delete_flags = MessageFlags::new();
    delete_flags.set_deleted(true);
    ctx.message_store
        .set_flags(&message_ids, delete_flags)
        .await?;

    // Actually delete the messages from the source mailbox
    ctx.message_store.delete_messages(&message_ids).await?;

    // Build COPYUID response (RFC 6851 uses COPYUID for MOVE)
    if !copied_metadata.is_empty() {
        let source_uids: Vec<String> = all_metadata
            .iter()
            .filter(|m| message_ids.contains(m.message_id()))
            .map(|m| m.uid().to_string())
            .collect();
        let dest_uids: Vec<String> = copied_metadata
            .iter()
            .map(|m| m.uid().to_string())
            .collect();

        let uid_validity = dest_mailbox_obj.map(|mb| mb.uid_validity()).unwrap_or(0);
        Ok(ImapResponse::ok(
            tag,
            format!(
                "[COPYUID {} {} {}] UID MOVE completed",
                uid_validity,
                source_uids.join(","),
                dest_uids.join(",")
            ),
        ))
    } else {
        Ok(ImapResponse::ok(tag, "UID MOVE completed"))
    }
}

/// Handle UID EXPUNGE command (RFC 4315)
/// Permanently removes messages with specified UIDs that have the \Deleted flag
async fn handle_uid_expunge(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    uid_sequence: &str,
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get all messages in the mailbox
    let all_metadata = ctx.message_store.get_mailbox_messages(mailbox_id).await?;

    // Parse UID sequence set
    let uid_set = parse_uid_sequence_set(uid_sequence, &all_metadata)?;

    // Find messages matching UIDs that also have \Deleted flag
    let mut deleted_messages = Vec::new();
    let mut expunge_responses = Vec::new();

    for (seq_num, metadata) in all_metadata.iter().enumerate() {
        if uid_set.contains(&metadata.uid()) && metadata.flags().is_deleted() {
            deleted_messages.push(*metadata.message_id());
            // IMAP sequence numbers are 1-based
            expunge_responses.push(format!("* {} EXPUNGE", seq_num + 1));
        }
    }

    // Delete the messages from storage
    if !deleted_messages.is_empty() {
        ctx.message_store.delete_messages(&deleted_messages).await?;
    }

    // Build response with untagged EXPUNGE responses
    let mut full_response = expunge_responses.join("\r\n");
    if !full_response.is_empty() {
        full_response.push_str("\r\n");
    }
    full_response.push_str(&format!("{} OK UID EXPUNGE completed", tag));

    Ok(ImapResponse::new(None, "", full_response))
}

// ---- Helper functions ----

/// Parse sequence set (simplified implementation)
pub(crate) fn parse_sequence_set(sequence: &str) -> anyhow::Result<Vec<MessageId>> {
    // For now, just return empty vec as we don't have real message IDs
    // In a real implementation, this would parse "1", "1:5", "1,3,5", "1:*", etc.
    let _ = sequence;
    Ok(Vec::new())
}

/// Parse sequence numbers (e.g., "1", "1:5", "1,3,5", "1:*")
pub(crate) fn parse_sequence_numbers(sequence: &str, max: usize) -> anyhow::Result<Vec<usize>> {
    let mut numbers = Vec::new();

    for part in sequence.split(',') {
        let part = part.trim();
        if part.contains(':') {
            // Range (e.g., "1:5" or "1:*")
            let range_parts: Vec<&str> = part.split(':').collect();
            if range_parts.len() == 2 {
                let start = range_parts[0].parse::<usize>().unwrap_or(1);
                let end = if range_parts[1] == "*" {
                    max
                } else {
                    range_parts[1].parse::<usize>().unwrap_or(max)
                };

                for n in start..=end.min(max) {
                    if !numbers.contains(&n) {
                        numbers.push(n);
                    }
                }
            }
        } else if part == "*" {
            // Last message
            if max > 0 && !numbers.contains(&max) {
                numbers.push(max);
            }
        } else {
            // Single number
            if let Ok(n) = part.parse::<usize>() {
                if n > 0 && n <= max && !numbers.contains(&n) {
                    numbers.push(n);
                }
            }
        }
    }

    numbers.sort();
    Ok(numbers)
}

/// Build FETCH response items
pub(crate) fn build_fetch_items(
    mail: &rusmes_proto::Mail,
    metadata: &MessageMetadata,
    items: &[String],
) -> String {
    let mut fetch_items = Vec::new();

    for item in items {
        match item.to_uppercase().as_str() {
            "FLAGS" => {
                // Build flags string from metadata
                let flags = metadata.flags();
                let mut flag_list = Vec::new();
                if flags.is_seen() {
                    flag_list.push("\\Seen");
                }
                if flags.is_answered() {
                    flag_list.push("\\Answered");
                }
                if flags.is_flagged() {
                    flag_list.push("\\Flagged");
                }
                if flags.is_deleted() {
                    flag_list.push("\\Deleted");
                }
                if flags.is_draft() {
                    flag_list.push("\\Draft");
                }
                fetch_items.push(format!("FLAGS ({})", flag_list.join(" ")));
            }
            "UID" => {
                fetch_items.push(format!("UID {}", metadata.uid()));
            }
            "BODY[]" | "BODY.PEEK[]" => {
                // Get full message body
                let message = mail.message();
                if let Ok(body_text) = message.extract_text() {
                    let body_len = body_text.len();
                    fetch_items.push(format!("BODY[] {{{}}}\r\n{}", body_len, body_text));
                } else {
                    fetch_items.push("BODY[] {0}\r\n".to_string());
                }
            }
            "RFC822.SIZE" => {
                fetch_items.push(format!("RFC822.SIZE {}", metadata.size()));
            }
            _ => {
                // Unknown item, skip
            }
        }
    }

    fetch_items.join(" ")
}

/// Parse search criteria (simplified)
pub(crate) fn parse_search_criteria(criteria: &[String]) -> SearchCriteria {
    if criteria.is_empty() {
        return SearchCriteria::All;
    }

    // Handle simple criteria
    match criteria[0].to_uppercase().as_str() {
        "ALL" => SearchCriteria::All,
        "UNSEEN" => SearchCriteria::Unseen,
        "SEEN" => SearchCriteria::Seen,
        "FLAGGED" => SearchCriteria::Flagged,
        "UNFLAGGED" => SearchCriteria::Unflagged,
        "DELETED" => SearchCriteria::Deleted,
        "UNDELETED" => SearchCriteria::Undeleted,
        _ => SearchCriteria::All, // Default to all
    }
}

/// Build MessageFlags from string flags
fn build_message_flags(flags: &[String]) -> MessageFlags {
    let mut msg_flags = MessageFlags::new();
    for flag in flags {
        match flag.to_uppercase().as_str() {
            "\\SEEN" => msg_flags.set_seen(true),
            "\\ANSWERED" => msg_flags.set_answered(true),
            "\\FLAGGED" => msg_flags.set_flagged(true),
            "\\DELETED" => msg_flags.set_deleted(true),
            "\\DRAFT" => msg_flags.set_draft(true),
            custom => msg_flags.add_custom(custom.to_string()),
        }
    }
    msg_flags
}

/// Parse message data into headers and body
pub(crate) fn parse_message_data(
    data: &bytes::Bytes,
) -> anyhow::Result<(rusmes_proto::HeaderMap, bytes::Bytes)> {
    use rusmes_proto::HeaderMap;

    let data_str = String::from_utf8_lossy(data);
    let mut headers = HeaderMap::new();
    let mut body_start = 0;

    // Find the blank line separating headers from body
    let lines: Vec<&str> = data_str.split("\r\n").collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Empty line marks end of headers
        if line.is_empty() {
            body_start = data_str[..data_str.len()]
                .find("\r\n\r\n")
                .map(|pos| pos + 4)
                .unwrap_or(data.len());
            break;
        }

        // Parse header line
        if let Some(colon_pos) = line.find(':') {
            let name = line[..colon_pos].trim();
            let value = line[colon_pos + 1..].trim();
            headers.insert(name.to_string(), value.to_string());
        }

        i += 1;
    }

    // Extract body
    let body = if body_start < data.len() {
        data.slice(body_start..)
    } else {
        bytes::Bytes::new()
    };

    Ok((headers, body))
}

/// Extract sender from message headers
pub(crate) fn extract_sender_from_headers(
    headers: &rusmes_proto::HeaderMap,
) -> Option<rusmes_proto::MailAddress> {
    if let Some(from) = headers.get_first("from") {
        // Simplified parsing - extract email from "Name <email@domain>" format
        let email = extract_email_address(from)?;
        parse_email_address(&email)
    } else {
        None
    }
}

/// Extract recipients from message headers
pub(crate) fn extract_recipients_from_headers(
    headers: &rusmes_proto::HeaderMap,
) -> Vec<rusmes_proto::MailAddress> {
    let mut recipients = Vec::new();

    // Parse To: header
    if let Some(to_values) = headers.get("to") {
        for to in to_values {
            if let Some(email) = extract_email_address(to) {
                if let Some(addr) = parse_email_address(&email) {
                    recipients.push(addr);
                }
            }
        }
    }

    // Parse Cc: header
    if let Some(cc_values) = headers.get("cc") {
        for cc in cc_values {
            if let Some(email) = extract_email_address(cc) {
                if let Some(addr) = parse_email_address(&email) {
                    recipients.push(addr);
                }
            }
        }
    }

    recipients
}

/// Extract email address from string like "Name <email@domain>" or "email@domain"
fn extract_email_address(s: &str) -> Option<String> {
    if let Some(start) = s.find('<') {
        if let Some(end) = s.find('>') {
            return Some(s[start + 1..end].trim().to_string());
        }
    }

    // No angle brackets, might be plain email
    let trimmed = s.trim();
    if trimmed.contains('@') {
        return Some(trimmed.to_string());
    }

    None
}

/// Parse email address string into MailAddress
fn parse_email_address(email: &str) -> Option<rusmes_proto::MailAddress> {
    use rusmes_proto::{Domain, MailAddress};

    if let Some(at_pos) = email.find('@') {
        let local_part = &email[..at_pos];
        let domain_str = &email[at_pos + 1..];

        if let Ok(domain) = Domain::new(domain_str.to_string()) {
            if let Ok(addr) = MailAddress::new(local_part, domain) {
                return Some(addr);
            }
        }
    }

    None
}

/// Parse UID sequence set into a set of UIDs
/// Supports formats like "1", "1:5", "1,3,5", "1:*", "*"
fn parse_uid_sequence_set(
    sequence: &str,
    all_metadata: &[MessageMetadata],
) -> anyhow::Result<std::collections::HashSet<u32>> {
    use std::collections::HashSet;

    let mut uid_set = HashSet::new();

    // Find max UID for handling "*"
    let max_uid = all_metadata.iter().map(|m| m.uid()).max().unwrap_or(0);

    // Split by comma for multiple ranges/values
    for part in sequence.split(',') {
        let part = part.trim();

        if part.contains(':') {
            // Range specification
            let range_parts: Vec<&str> = part.split(':').collect();
            if range_parts.len() != 2 {
                return Err(anyhow::anyhow!("Invalid UID range: {}", part));
            }

            let start = if range_parts[0] == "*" {
                max_uid
            } else {
                range_parts[0].parse::<u32>()?
            };

            let end = if range_parts[1] == "*" {
                max_uid
            } else {
                range_parts[1].parse::<u32>()?
            };

            // Add all UIDs in range
            for uid in start..=end {
                uid_set.insert(uid);
            }
        } else {
            // Single UID
            let uid = if part == "*" {
                max_uid
            } else {
                part.parse::<u32>()?
            };
            uid_set.insert(uid);
        }
    }

    Ok(uid_set)
}
