//! IMAP mailbox command handlers
//!
//! Covers: SELECT, EXAMINE, LIST, LSUB, SUBSCRIBE, UNSUBSCRIBE,
//!         CREATE, CREATE_SPECIAL_USE, DELETE, RENAME, NAMESPACE, IDLE

use crate::handler::HandlerContext;
use crate::response::ImapResponse;
use crate::session::{ImapSession, ImapState};
use rusmes_storage::MailboxPath;

/// Handle SELECT/EXAMINE command
pub(crate) async fn handle_select(
    ctx: &HandlerContext,
    session: &mut ImapSession,
    tag: &str,
    mailbox: &str,
    read_only: bool,
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

    // Optimize for INBOX - use direct lookup
    let mailbox_obj = if mailbox.eq_ignore_ascii_case("INBOX") {
        // Use optimized INBOX lookup
        if let Some(inbox_id) = ctx.mailbox_store.get_user_inbox(&username).await? {
            ctx.mailbox_store.get_mailbox(&inbox_id).await?
        } else {
            None
        }
    } else {
        // List mailboxes to find the requested one
        let mailboxes = ctx.mailbox_store.list_mailboxes(&username).await?;
        mailboxes
            .iter()
            .find(|m| m.path().name() == Some(mailbox))
            .cloned()
    };

    match mailbox_obj {
        Some(mb) => {
            let mailbox_id = *mb.id();

            // Get mailbox counters
            let counters = ctx.metadata_store.get_mailbox_counters(&mailbox_id).await?;

            // Update session state
            session.state = ImapState::Selected { mailbox_id };

            // Build response with untagged responses
            let mode = if read_only { "READ-ONLY" } else { "READ-WRITE" };
            let response_text = format!(
                "* {} EXISTS\r\n* {} RECENT\r\n* OK [UIDVALIDITY {}]\r\n* OK [UIDNEXT {}]\r\n* FLAGS (\\Seen \\Answered \\Flagged \\Deleted \\Draft)\r\n{} OK [{}] {} completed",
                counters.exists,
                counters.recent,
                mb.uid_validity(),
                mb.uid_next(),
                tag,
                mode,
                if read_only { "EXAMINE" } else { "SELECT" }
            );

            Ok(ImapResponse::new(None, "", response_text))
        }
        None => Ok(ImapResponse::no(tag, "Mailbox does not exist")),
    }
}

/// Handle LIST command
pub(crate) async fn handle_list(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    _reference: &str,
    pattern: &str,
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

    // List mailboxes
    let mailboxes = ctx.mailbox_store.list_mailboxes(&username).await?;

    // Filter by pattern (simplified - just check if pattern is * or mailbox name)
    let mut responses = Vec::new();
    for mailbox in mailboxes {
        if pattern == "*" || mailbox.path().name() == Some(pattern) {
            let name = mailbox.path().name().unwrap_or("INBOX");
            responses.push(format!(r#"* LIST () "/" "{}""#, name));
        }
    }

    // Build response
    let mut full_response = responses.join("\r\n");
    if !full_response.is_empty() {
        full_response.push_str("\r\n");
    }
    full_response.push_str(&format!("{} OK LIST completed", tag));

    Ok(ImapResponse::new(None, "", full_response))
}

/// Handle LSUB command
pub(crate) async fn handle_lsub(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    _reference: &str,
    pattern: &str,
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

    // Get subscribed mailboxes
    let subscriptions = ctx.mailbox_store.list_subscriptions(&username).await?;

    // Filter by pattern and build responses
    let mut responses = Vec::new();
    for mailbox_name in subscriptions {
        // Pattern matching: "*" matches all, otherwise exact match or wildcard matching
        let matches = if pattern == "*" {
            true
        } else if pattern.contains('*') || pattern.contains('%') {
            // Simplified wildcard matching
            match_mailbox_pattern(&mailbox_name, pattern)
        } else {
            mailbox_name == pattern
        };

        if matches {
            responses.push(format!(r#"* LSUB () "/" "{}""#, mailbox_name));
        }
    }

    // Build response
    let mut full_response = responses.join("\r\n");
    if !full_response.is_empty() {
        full_response.push_str("\r\n");
    }
    full_response.push_str(&format!("{} OK LSUB completed", tag));

    Ok(ImapResponse::new(None, "", full_response))
}

/// Handle SUBSCRIBE command
pub(crate) async fn handle_subscribe(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    mailbox: &str,
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

    // Strip quotes from mailbox name if present
    let mailbox_name = mailbox.trim_matches('"');

    // Subscribe to the mailbox
    ctx.mailbox_store
        .subscribe_mailbox(&username, mailbox_name.to_string())
        .await?;

    Ok(ImapResponse::ok(tag, "SUBSCRIBE completed"))
}

/// Handle UNSUBSCRIBE command
pub(crate) async fn handle_unsubscribe(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    mailbox: &str,
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

    // Strip quotes from mailbox name if present
    let mailbox_name = mailbox.trim_matches('"');

    // Unsubscribe from the mailbox
    ctx.mailbox_store
        .unsubscribe_mailbox(&username, mailbox_name)
        .await?;

    Ok(ImapResponse::ok(tag, "UNSUBSCRIBE completed"))
}

/// Handle CREATE command
pub(crate) async fn handle_create(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    mailbox: &str,
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

    // Create mailbox path
    let path = MailboxPath::new(username, vec![mailbox.to_string()]);

    // Create mailbox
    ctx.mailbox_store.create_mailbox(&path).await?;

    Ok(ImapResponse::ok(tag, "CREATE completed"))
}

/// Handle CREATE-SPECIAL-USE command (RFC 6154)
pub(crate) async fn handle_create_special_use(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    mailbox: &str,
    special_use: &str,
) -> anyhow::Result<ImapResponse> {
    use rusmes_storage::SpecialUseAttributes;

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

    // Create mailbox path
    let path = MailboxPath::new(username, vec![mailbox.to_string()]);

    // Create special-use attributes
    let attrs = SpecialUseAttributes::single(special_use.to_string());

    // Create mailbox with special-use attribute
    ctx.mailbox_store
        .create_mailbox_with_special_use(&path, attrs)
        .await?;

    Ok(ImapResponse::ok(tag, "CREATE completed"))
}

/// Handle DELETE command
pub(crate) async fn handle_delete(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    mailbox: &str,
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

    match mailbox_obj {
        Some(mb) => {
            ctx.mailbox_store.delete_mailbox(mb.id()).await?;
            Ok(ImapResponse::ok(tag, "DELETE completed"))
        }
        None => Ok(ImapResponse::no(tag, "Mailbox does not exist")),
    }
}

/// Handle RENAME command
pub(crate) async fn handle_rename(
    ctx: &HandlerContext,
    session: &ImapSession,
    tag: &str,
    old_mailbox: &str,
    new_mailbox: &str,
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

    // Find the old mailbox
    let mailboxes = ctx.mailbox_store.list_mailboxes(&username).await?;
    let mailbox_obj = mailboxes
        .iter()
        .find(|m| m.path().name() == Some(old_mailbox));

    match mailbox_obj {
        Some(mb) => {
            // Create new mailbox path
            let new_path = MailboxPath::new(username, vec![new_mailbox.to_string()]);
            ctx.mailbox_store.rename_mailbox(mb.id(), &new_path).await?;
            Ok(ImapResponse::ok(tag, "RENAME completed"))
        }
        None => Ok(ImapResponse::no(tag, "Mailbox does not exist")),
    }
}

/// Handle IDLE command (RFC 2177)
/// Prepares for IDLE mode - actual IDLE loop must be handled by server
pub(crate) async fn handle_idle(
    ctx: &HandlerContext,
    session: &mut ImapSession,
    tag: &str,
) -> anyhow::Result<ImapResponse> {
    // Must have a mailbox selected
    let mailbox_id = match session.state() {
        ImapState::Selected { mailbox_id } => *mailbox_id,
        _ => return Ok(ImapResponse::no(tag, "No mailbox selected")),
    };

    // Get current mailbox state for snapshot
    let counters = ctx.metadata_store.get_mailbox_counters(&mailbox_id).await?;
    session.update_snapshot(counters.exists, counters.recent);

    // Transition to IDLE state
    session.state = ImapState::Idle { mailbox_id };
    session.tag = Some(tag.to_string());

    // Return continuation response
    // The server will handle the IDLE loop and send the OK response later
    Ok(ImapResponse::new(None, "+", "idling"))
}

/// Handle NAMESPACE command (RFC 2342)
/// Returns namespace information for personal, other users, and shared mailboxes
pub(crate) async fn handle_namespace(
    tag: &str,
    session: &ImapSession,
) -> anyhow::Result<ImapResponse> {
    // Only works in Authenticated or Selected state
    match session.state() {
        ImapState::NotAuthenticated => {
            return Ok(ImapResponse::no(tag, "NAMESPACE requires authentication"));
        }
        ImapState::Logout => {
            return Ok(ImapResponse::no(tag, "Already logged out"));
        }
        _ => {}
    }

    // Personal namespace: empty prefix with "." delimiter
    // This means mailboxes like "INBOX", "Sent", "Drafts" etc. are at the root
    let personal = vec![("".to_string(), ".".to_string())];

    // Other users namespace: not supported (NIL)
    let other_users: Vec<(String, String)> = Vec::new();

    // Shared namespace: not supported (NIL)
    let shared: Vec<(String, String)> = Vec::new();

    // Format the namespace response
    let personal_str = format_namespace_list(&personal);
    let other_users_str = format_namespace_list(&other_users);
    let shared_str = format_namespace_list(&shared);

    // Build untagged NAMESPACE response
    let untagged_response = format!(
        "* NAMESPACE {} {} {}",
        personal_str, other_users_str, shared_str
    );

    // Build full response with untagged response followed by tagged OK
    let full_response = format!("{}\r\n{} OK NAMESPACE completed", untagged_response, tag);

    Ok(ImapResponse::new(None, "", full_response))
}

/// Format a list of namespaces according to RFC 2342
/// Each namespace is a tuple of (prefix, delimiter)
/// Returns "NIL" if the list is empty, otherwise returns a parenthesized list
fn format_namespace_list(namespaces: &[(String, String)]) -> String {
    if namespaces.is_empty() {
        "NIL".to_string()
    } else {
        let items: Vec<String> = namespaces
            .iter()
            .map(|(prefix, delim)| format!("(\"{}\" \"{}\")", prefix, delim))
            .collect();
        format!("({})", items.join(" "))
    }
}

/// Match mailbox name against pattern
/// Supports IMAP wildcards: * (matches any sequence) and % (matches any sequence except hierarchy delimiter)
pub(crate) fn match_mailbox_pattern(name: &str, pattern: &str) -> bool {
    // Simplified pattern matching for IMAP LIST/LSUB
    // * matches zero or more characters including hierarchy delimiter
    // % matches zero or more characters excluding hierarchy delimiter (/)

    if pattern == "*" {
        return true;
    }

    // Convert IMAP pattern to regex-like matching
    let mut pattern_chars = pattern.chars().peekable();
    let mut name_chars = name.chars().peekable();

    loop {
        match (pattern_chars.peek(), name_chars.peek()) {
            (None, None) => return true,
            (None, Some(_)) => return false,
            (Some(&'*'), _) => {
                pattern_chars.next();
                // * matches everything, so just continue with rest of pattern
                if pattern_chars.peek().is_none() {
                    return true;
                }
                // Try to match rest of pattern at each position
                let rest_pattern: String = pattern_chars.collect();
                for i in 0..=name_chars.clone().count() {
                    let rest_name: String = name_chars.clone().skip(i).collect();
                    if match_mailbox_pattern(&rest_name, &rest_pattern) {
                        return true;
                    }
                }
                return false;
            }
            (Some(&'%'), _) => {
                pattern_chars.next();
                // % matches everything except hierarchy delimiter
                if pattern_chars.peek().is_none() {
                    // % at end matches if no more hierarchy delimiters
                    return !name_chars.clone().any(|c| c == '/');
                }
                let rest_pattern: String = pattern_chars.collect();
                for i in 0..=name_chars.clone().count() {
                    let rest_name: String = name_chars.clone().skip(i).collect();
                    // Check if we crossed a hierarchy delimiter
                    let skipped: String = name_chars.clone().take(i).collect();
                    if !skipped.contains('/') && match_mailbox_pattern(&rest_name, &rest_pattern) {
                        return true;
                    }
                }
                return false;
            }
            (Some(&p), Some(&n)) => {
                if p == n {
                    pattern_chars.next();
                    name_chars.next();
                } else {
                    return false;
                }
            }
            (Some(_), None) => return false,
        }
    }
}
