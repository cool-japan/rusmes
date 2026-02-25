//! IMAP session state machine

use rusmes_proto::Username;
use rusmes_storage::MailboxId;
use std::time::Duration;

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
}

impl Default for ImapSession {
    fn default() -> Self {
        Self::new()
    }
}
