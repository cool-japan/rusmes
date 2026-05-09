//! IMAP command handler dispatcher
//!
//! This module contains the top-level [`HandlerContext`] and the [`handle_command`]
//! dispatcher. Actual implementations live in sub-modules:
//!
//! - [`crate::handler_auth`]    – LOGIN, LOGOUT
//! - [`crate::handler_mailbox`] – SELECT/EXAMINE, LIST, LSUB, SUBSCRIBE/UNSUBSCRIBE,
//!   CREATE, DELETE, RENAME, IDLE, NAMESPACE
//! - [`crate::handler_message`] – FETCH, STORE, SEARCH, APPEND, COPY, MOVE, EXPUNGE,
//!   CLOSE, UID sub-commands

use crate::command::ImapCommand;
use crate::handler_auth::{handle_login, handle_logout};
use crate::handler_mailbox::{
    handle_create, handle_create_special_use, handle_delete, handle_idle, handle_list, handle_lsub,
    handle_namespace, handle_rename, handle_select, handle_subscribe, handle_unsubscribe,
};
use crate::handler_message::{
    handle_append, handle_close, handle_copy, handle_expunge, handle_fetch, handle_move,
    handle_search, handle_store, handle_uid,
};
use crate::mailbox_registry::MailboxRegistry;
use crate::response::ImapResponse;
use crate::session::ImapSession;
use rusmes_auth::AuthBackend;
use rusmes_storage::{MailboxStore, MessageStore, MetadataStore};
use std::sync::Arc;

/// Handler context for IMAP commands
pub struct HandlerContext {
    pub mailbox_store: Arc<dyn MailboxStore>,
    pub message_store: Arc<dyn MessageStore>,
    pub metadata_store: Arc<dyn MetadataStore>,
    pub auth_backend: Arc<dyn AuthBackend>,
    /// Cross-session mailbox notification registry (Cluster 10).
    pub mailbox_registry: Arc<MailboxRegistry>,
}

impl HandlerContext {
    /// Create a new handler context
    pub fn new(
        mailbox_store: Arc<dyn MailboxStore>,
        message_store: Arc<dyn MessageStore>,
        metadata_store: Arc<dyn MetadataStore>,
        auth_backend: Arc<dyn AuthBackend>,
    ) -> Self {
        Self {
            mailbox_store,
            message_store,
            metadata_store,
            auth_backend,
            mailbox_registry: Arc::new(MailboxRegistry::new()),
        }
    }

    /// Create a handler context with a pre-existing registry (for sharing across server instances).
    pub fn with_registry(
        mailbox_store: Arc<dyn MailboxStore>,
        message_store: Arc<dyn MessageStore>,
        metadata_store: Arc<dyn MetadataStore>,
        auth_backend: Arc<dyn AuthBackend>,
        mailbox_registry: Arc<MailboxRegistry>,
    ) -> Self {
        Self {
            mailbox_store,
            message_store,
            metadata_store,
            auth_backend,
            mailbox_registry,
        }
    }
}

/// Handle an IMAP command — dispatches to the appropriate sub-handler
#[allow(clippy::too_many_arguments)]
pub async fn handle_command(
    ctx: &HandlerContext,
    session: &mut ImapSession,
    tag: &str,
    command: ImapCommand,
) -> anyhow::Result<ImapResponse> {
    match command {
        ImapCommand::Capability => handle_capability(tag).await,
        ImapCommand::Noop => handle_noop(tag).await,
        ImapCommand::Login { user, password } => {
            handle_login(ctx, session, tag, &user, &password).await
        }
        ImapCommand::Authenticate {
            mechanism,
            initial_response,
        } => {
            // Note: This is a placeholder. AUTHENTICATE requires special handling
            // in the server loop because it's a multi-step process.
            // The actual implementation is in crate::authenticate module.
            let _ = (mechanism, initial_response);
            Ok(ImapResponse::bad(
                tag,
                "AUTHENTICATE must be handled by server loop",
            ))
        }
        ImapCommand::Logout => handle_logout(session, tag).await,
        ImapCommand::Select { mailbox } => handle_select(ctx, session, tag, &mailbox, false).await,
        ImapCommand::Examine { mailbox } => handle_select(ctx, session, tag, &mailbox, true).await,
        ImapCommand::Fetch { sequence, items } => {
            handle_fetch(ctx, session, tag, &sequence, &items).await
        }
        ImapCommand::Store {
            sequence,
            mode,
            flags,
        } => handle_store(ctx, session, tag, &sequence, mode, &flags).await,
        ImapCommand::Search { criteria } => handle_search(ctx, session, tag, &criteria).await,
        ImapCommand::List { reference, mailbox } => {
            handle_list(ctx, session, tag, &reference, &mailbox).await
        }
        ImapCommand::Lsub { reference, mailbox } => {
            handle_lsub(ctx, session, tag, &reference, &mailbox).await
        }
        ImapCommand::Subscribe { mailbox } => handle_subscribe(ctx, session, tag, &mailbox).await,
        ImapCommand::Unsubscribe { mailbox } => {
            handle_unsubscribe(ctx, session, tag, &mailbox).await
        }
        ImapCommand::Create { mailbox } => handle_create(ctx, session, tag, &mailbox).await,
        ImapCommand::CreateSpecialUse {
            mailbox,
            special_use,
        } => handle_create_special_use(ctx, session, tag, &mailbox, &special_use).await,
        ImapCommand::Delete { mailbox } => handle_delete(ctx, session, tag, &mailbox).await,
        ImapCommand::Rename { old, new } => handle_rename(ctx, session, tag, &old, &new).await,
        ImapCommand::Append {
            mailbox,
            flags,
            date_time,
            message_literal,
        } => {
            handle_append(
                ctx,
                session,
                tag,
                &mailbox,
                &flags,
                date_time.as_deref(),
                &message_literal,
            )
            .await
        }
        ImapCommand::Copy { sequence, mailbox } => {
            handle_copy(ctx, session, tag, &sequence, &mailbox).await
        }
        ImapCommand::Move { sequence, mailbox } => {
            handle_move(ctx, session, tag, &sequence, &mailbox).await
        }
        ImapCommand::Expunge => handle_expunge(ctx, session, tag).await,
        ImapCommand::Close => handle_close(ctx, session, tag).await,
        ImapCommand::Idle => handle_idle(ctx, session, tag).await,
        ImapCommand::Namespace => handle_namespace(tag, session).await,
        ImapCommand::Uid { subcommand } => handle_uid(ctx, session, tag, subcommand.as_ref()).await,
        ImapCommand::Compress { mechanism } => handle_compress(session, tag, &mechanism).await,
    }
}

/// Handle CAPABILITY command
async fn handle_capability(tag: &str) -> anyhow::Result<ImapResponse> {
    // Return basic IMAP4rev1 capabilities
    let capabilities = vec![
        "IMAP4rev1",
        "LITERAL+",
        "SASL-IR",
        "LOGIN-REFERRALS",
        "ID",
        "ENABLE",
        "IDLE",
        "NAMESPACE",
        "UIDPLUS",
        "LIST-EXTENDED",
        "UNSELECT",
        "CHILDREN",
        "SPECIAL-USE",
        "MOVE",
        // Compression (RFC 4978)
        "COMPRESS=DEFLATE",
        // SASL authentication mechanisms (RFC 3501 Section 6.2.2)
        "AUTH=PLAIN",
        "AUTH=LOGIN",
        "AUTH=CRAM-MD5",
        "AUTH=SCRAM-SHA-256",
        "AUTH=XOAUTH2",
    ];

    let cap_list = capabilities.join(" ");
    Ok(ImapResponse::ok(
        tag,
        format!("[CAPABILITY {}] Capability completed", cap_list),
    ))
}

/// Handle NOOP command
async fn handle_noop(tag: &str) -> anyhow::Result<ImapResponse> {
    Ok(ImapResponse::ok(tag, "NOOP completed"))
}

/// Handle COMPRESS command (RFC 4978).
///
/// This handler only validates the mechanism and sets the `compress_pending` flag on the session.
/// The actual stream wrapping is done by `server::imap_session_loop` immediately after this
/// response is sent and flushed, using `oxiarc_deflate::raw_stream::{RawInflateReader,
/// RawDeflateWriter}`.
async fn handle_compress(
    session: &mut ImapSession,
    tag: &str,
    mechanism: &str,
) -> anyhow::Result<ImapResponse> {
    if !mechanism.eq_ignore_ascii_case("DEFLATE") {
        return Ok(ImapResponse::no(
            tag,
            format!("COMPRESS: unsupported mechanism {mechanism}"),
        ));
    }
    // Only allow compression once per session (RFC 4978 §3).
    if session.compress_pending {
        return Ok(ImapResponse::no(tag, "COMPRESS: already active"));
    }
    // Signal the server loop to swap the stream after the OK is written.
    session.compress_pending = true;
    Ok(ImapResponse::ok(
        tag,
        "[COMPRESSIONACTIVE] Begin DEFLATE compression",
    ))
}
