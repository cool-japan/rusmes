//! IMAP command types

/// IMAP commands
#[derive(Debug, Clone)]
pub enum ImapCommand {
    /// LOGIN user password
    Login { user: String, password: String },
    /// SELECT mailbox
    Select { mailbox: String },
    /// EXAMINE mailbox
    Examine { mailbox: String },
    /// FETCH sequence data
    Fetch {
        sequence: String,
        items: Vec<String>,
    },
    /// STORE sequence flags (with mode: FLAGS, +FLAGS, -FLAGS)
    Store {
        sequence: String,
        mode: StoreMode,
        flags: Vec<String>,
    },
    /// SEARCH criteria
    Search { criteria: Vec<String> },
    /// LIST reference mailbox
    List { reference: String, mailbox: String },
    /// LSUB reference mailbox
    Lsub { reference: String, mailbox: String },
    /// SUBSCRIBE mailbox
    Subscribe { mailbox: String },
    /// UNSUBSCRIBE mailbox
    Unsubscribe { mailbox: String },
    /// CREATE mailbox
    Create { mailbox: String },
    /// CREATE-SPECIAL-USE mailbox special-use-attr (RFC 6154)
    CreateSpecialUse {
        mailbox: String,
        special_use: String,
    },
    /// DELETE mailbox
    Delete { mailbox: String },
    /// RENAME old new
    Rename { old: String, new: String },
    /// APPEND mailbox \[flags\] \[date-time\] literal
    Append {
        mailbox: String,
        flags: Vec<String>,
        date_time: Option<String>,
        message_literal: Vec<u8>,
    },
    /// COPY sequence mailbox
    Copy { sequence: String, mailbox: String },
    /// MOVE sequence mailbox (RFC 6851)
    Move { sequence: String, mailbox: String },
    /// EXPUNGE (permanently delete messages with \Deleted flag)
    Expunge,
    /// CLOSE (implicit expunge + deselect)
    Close,
    /// CAPABILITY
    Capability,
    /// LOGOUT
    Logout,
    /// NOOP
    Noop,
    /// IDLE (RFC 2177) - push notifications
    Idle,
    /// NAMESPACE (RFC 2342) - mailbox namespace discovery
    Namespace,
    /// AUTHENTICATE mechanism [initial-response] (RFC 3501 Section 6.2.2)
    Authenticate {
        mechanism: String,
        initial_response: Option<String>,
    },
    /// COMPRESS mechanism (RFC 4978) — e.g. `COMPRESS DEFLATE`
    Compress { mechanism: String },
    /// UID command variants (RFC 9051 Section 6.4.8)
    Uid { subcommand: Box<UidSubcommand> },
}

/// UID command subcommands
#[derive(Debug, Clone)]
pub enum UidSubcommand {
    /// UID FETCH sequence data
    Fetch {
        sequence: String,
        items: Vec<String>,
    },
    /// UID STORE sequence flags
    Store {
        sequence: String,
        mode: StoreMode,
        flags: Vec<String>,
    },
    /// UID SEARCH criteria
    Search { criteria: Vec<String> },
    /// UID COPY sequence mailbox
    Copy { sequence: String, mailbox: String },
    /// UID MOVE sequence mailbox
    Move { sequence: String, mailbox: String },
    /// UID EXPUNGE sequence (RFC 4315)
    Expunge { sequence: String },
}

/// STORE command mode
#[derive(Debug, Clone, PartialEq)]
pub enum StoreMode {
    /// Replace flags
    Replace,
    /// Add flags
    Add,
    /// Remove flags
    Remove,
}
