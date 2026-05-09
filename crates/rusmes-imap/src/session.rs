//! IMAP session state machine

use crate::mailbox_registry::MailboxEvent;
use rusmes_proto::Username;
use rusmes_storage::MailboxId;
use std::time::Duration;
use tokio::sync::broadcast;

/// IMAP session state
#[derive(Debug, Clone, PartialEq)]
pub enum ImapState {
    /// Not authenticated
    NotAuthenticated,
    /// Authenticated but no mailbox selected
    Authenticated,
    /// Mailbox selected
    Selected { mailbox_id: MailboxId },
    /// In IDLE mode (RFC 2177)
    Idle { mailbox_id: MailboxId },
    /// Logout
    Logout,
}

/// Mailbox snapshot for change detection during IDLE
#[derive(Debug, Clone)]
pub struct MailboxSnapshot {
    pub exists: u32,
    pub recent: u32,
}

/// IMAP session
pub struct ImapSession {
    pub state: ImapState,
    pub tag: Option<String>,
    pub username: Option<Username>,
    pub mailbox_snapshot: Option<MailboxSnapshot>,
    pub idle_timeout: Duration,
    /// Receiver for cross-session mailbox notifications. Set when a mailbox is SELECTed,
    /// cleared on CLOSE/LOGOUT.
    pub mailbox_event_rx: Option<broadcast::Receiver<MailboxEvent>>,
    /// Whether the client has negotiated COMPRESS=DEFLATE (RFC 4978).
    /// The server loop reads this flag immediately after sending the OK response and
    /// performs the stream swap before processing any further commands.
    pub compress_pending: bool,
}

impl ImapSession {
    /// Create a new IMAP session with default timeout
    pub fn new() -> Self {
        Self::new_with_timeout(Duration::from_secs(1800))
    }

    /// Create a new IMAP session with custom timeout
    pub fn new_with_timeout(idle_timeout: Duration) -> Self {
        Self {
            state: ImapState::NotAuthenticated,
            tag: None,
            username: None,
            mailbox_snapshot: None,
            idle_timeout,
            mailbox_event_rx: None,
            compress_pending: false,
        }
    }

    /// Get current state
    pub fn state(&self) -> &ImapState {
        &self.state
    }

    /// Update mailbox snapshot
    pub fn update_snapshot(&mut self, exists: u32, recent: u32) {
        self.mailbox_snapshot = Some(MailboxSnapshot { exists, recent });
    }

    /// Get mailbox ID from current state
    pub fn mailbox_id(&self) -> Option<&MailboxId> {
        match &self.state {
            ImapState::Selected { mailbox_id } | ImapState::Idle { mailbox_id } => Some(mailbox_id),
            _ => None,
        }
    }

    /// Drain all pending cross-session mailbox events and return them as untagged IMAP response lines.
    ///
    /// Call this before/after every command so that non-IDLE sessions still receive notifications
    /// (RFC 3501 §5.2: "the server SHOULD send untagged responses on an opportunistic basis").
    pub fn drain_mailbox_events(&mut self) -> Vec<String> {
        let rx = match self.mailbox_event_rx.as_mut() {
            Some(r) => r,
            None => return Vec::new(),
        };
        let mut lines = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(event) => {
                    if let Some(line) = format_mailbox_event(&event) {
                        lines.push(line);
                    }
                }
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    tracing::warn!(
                        "IMAP session lagged {n} mailbox events — some notifications dropped"
                    );
                }
                Err(broadcast::error::TryRecvError::Closed) => {
                    self.mailbox_event_rx = None;
                    break;
                }
            }
        }
        lines
    }
}

/// Format a [`MailboxEvent`] as an untagged IMAP response line (without trailing CRLF).
fn format_mailbox_event(event: &MailboxEvent) -> Option<String> {
    match event {
        MailboxEvent::Exists { count } => Some(format!("* {count} EXISTS")),
        MailboxEvent::Recent { count } => Some(format!("* {count} RECENT")),
        MailboxEvent::Expunge { seq } => Some(format!("* {seq} EXPUNGE")),
        MailboxEvent::FlagsChanged { uid, flags } => {
            let flag_str = flags.join(" ");
            // We emit a UID FETCH response so the client can correlate by UID.
            Some(format!("* {uid} FETCH (UID {uid} FLAGS ({flag_str}))"))
        }
    }
}

/// Public wrapper around `format_mailbox_event` for use by the server's IDLE loop.
///
/// Returns the untagged IMAP response line for `event` (without trailing CRLF),
/// or `None` if the event type produces no untagged response.
pub fn format_mailbox_event_pub(event: &MailboxEvent) -> Option<String> {
    format_mailbox_event(event)
}

impl Default for ImapSession {
    fn default() -> Self {
        Self::new()
    }
}
