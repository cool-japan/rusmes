//! IMAP protocol implementation for RusMES
//!
// The `BufReader<RawInflateReader<...>>` type used in imap_session_loop after
// COMPRESS=DEFLATE negotiation is deeply nested enough that the default recursion
// limit (128) is exhausted when the trait solver proves `Send` for the
// tokio::spawn future in server.rs. Bump to 512 to give the solver enough headroom.
#![recursion_limit = "1024"]
//!
//! This crate provides a full-featured, RFC-compliant IMAP server implementation
//! built on Tokio for asynchronous I/O.
//!
//! # RFC Compliance
//!
//! - **RFC 9051** (IMAP4rev2) / **RFC 3501** (IMAP4rev1): Core IMAP protocol
//! - **RFC 7162** (CONDSTORE / QRESYNC): Efficient mailbox synchronization with
//!   `MODSEQ` tracking, `CHANGEDSINCE`/`UNCHANGEDSINCE` modifiers, and VANISHED responses
//! - **RFC 6154** (SPECIAL-USE): `\Drafts`, `\Sent`, `\Trash`, `\Junk`, etc.
//! - **RFC 4315** (UIDPLUS): `APPENDUID` and `COPYUID` response codes
//! - **RFC 6851** (MOVE): `MOVE` command and `COPYUID` response for moves
//! - **RFC 2177** (IDLE): Server-push idle notification
//! - **RFC 2342** (NAMESPACE): Personal, shared, and other-users namespaces
//! - **RFC 2449** (CAPABILITY): Capability advertisement
//!
//! # Supported Commands
//!
//! ## Not Authenticated State
//! - `CAPABILITY` — List server capabilities
//! - `NOOP` — No-operation (keep-alive)
//! - `LOGOUT` — Disconnect
//! - `LOGIN user password` — Plain-text authentication
//! - `AUTHENTICATE mechanism` — SASL authentication (PLAIN, LOGIN, CRAM-MD5,
//!   SCRAM-SHA-256, XOAUTH2)
//!
//! ## Authenticated State
//! - `SELECT mailbox` — Open mailbox read-write
//! - `EXAMINE mailbox` — Open mailbox read-only
//! - `CREATE mailbox` — Create new mailbox (with optional `SPECIAL-USE` flag)
//! - `DELETE mailbox` — Delete a mailbox
//! - `RENAME old new` — Rename a mailbox
//! - `SUBSCRIBE mailbox` — Subscribe to a mailbox
//! - `UNSUBSCRIBE mailbox` — Unsubscribe from a mailbox
//! - `LIST reference mailbox` — List mailboxes matching a pattern
//! - `LSUB reference mailbox` — List subscribed mailboxes matching a pattern
//! - `NAMESPACE` — Query server namespaces
//! - `APPEND mailbox ...` — Append a message to a mailbox
//!
//! ## Selected State
//! - `FETCH sequence items` — Retrieve message data
//! - `STORE sequence flags` — Modify message flags
//! - `SEARCH criteria` — Search for messages
//! - `COPY sequence mailbox` — Copy messages to another mailbox
//! - `MOVE sequence mailbox` — Move messages to another mailbox
//! - `EXPUNGE` — Permanently delete messages with `\Deleted` flag
//! - `CLOSE` — Implicit expunge and deselect
//! - `IDLE` — Enter server-push idle mode (RFC 2177)
//! - `UID FETCH/STORE/SEARCH/COPY/MOVE/EXPUNGE` — UID-based variants
//!
//! # Modules
//!
//! - [`authenticate`]: SASL multi-step authentication (PLAIN, LOGIN, CRAM-MD5,
//!   SCRAM-SHA-256, XOAUTH2)
//! - [`command`]: IMAP command enumeration and types
//! - [`condstore`]: CONDSTORE extension — `MODSEQ`, `CHANGEDSINCE`, `UNCHANGEDSINCE`
//! - [`config`]: Server configuration (`ImapConfig`)
//! - [`handler`]: Command dispatcher (`HandlerContext`, `handle_command`)
//! - [`handler_auth`]: LOGIN and LOGOUT handlers
//! - [`handler_mailbox`]: Mailbox-level command handlers
//! - [`handler_message`]: Message-level command handlers (FETCH, STORE, etc.)
//! - [`mailbox_watcher`]: Watches for mailbox changes to support IDLE push notifications
//! - [`parser`]: IMAP command-line parser including APPEND literal handling
//! - [`qresync`]: QRESYNC extension — UID range sets, vanished responses
//! - [`response`]: IMAP response formatting (tagged OK/NO/BAD, untagged `*`)
//! - [`server`]: Async TCP listener accepting IMAP connections
//! - [`session`]: Per-connection state machine (NotAuthenticated → Authenticated →
//!   Selected ↔ Idle → Logout)
//! - [`special_use`]: `SPECIAL-USE` mailbox attributes (RFC 6154)
//!
//! # Security
//!
//! All random challenge material (CRAM-MD5, SCRAM nonces) is generated via
//! `getrandom` for cryptographic-quality entropy. In production deployments,
//! TLS should be enabled to protect LOGIN/PLAIN credentials in transit.

pub mod authenticate;
pub mod command;
pub mod condstore;
pub mod config;
pub mod handler;
pub mod handler_auth;
pub mod handler_mailbox;
pub mod handler_message;
pub mod mailbox_registry;
pub mod mailbox_watcher;
pub mod parser;
pub mod qresync;
pub mod response;
pub mod server;
pub mod session;
pub mod special_use;

pub use authenticate::{
    create_default_sasl_server, handle_authenticate, handle_authenticate_continue,
    parse_authenticate_args, AuthenticateContext, AuthenticateState,
};
pub use command::{ImapCommand, StoreMode};
pub use condstore::{
    ChangedSince, CondStoreError, CondStoreResponse, CondStoreState, CondStoreStatus,
    UnchangedSince,
};
pub use config::ImapConfig;
pub use handler::{handle_command, HandlerContext};
pub use mailbox_registry::{MailboxEvent, MailboxRegistry};
pub use mailbox_watcher::{MailboxChanges, MailboxWatcher};
pub use parser::{has_literal, parse_append_command, parse_command, LiteralType};
pub use qresync::{
    QResyncError, QResyncLogic, QResyncParams, QResyncState, SeqMatchData, UidRange, UidSet,
    VanishedResponse,
};
pub use response::ImapResponse;
pub use server::ImapServer;
pub use session::{format_mailbox_event_pub, ImapSession, ImapState, MailboxSnapshot};
pub use special_use::{
    format_capability_response, format_list_extended, parse_create_special_use,
    suggest_special_use, validate_special_use_flags, SpecialUse, SpecialUseError, SpecialUseFlags,
    LIST_EXTENDED_CAPABILITY, SPECIAL_USE_CAPABILITY,
};
