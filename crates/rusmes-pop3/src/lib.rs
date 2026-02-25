//! POP3 protocol implementation for RusMES
//!
//! This crate provides a full-featured, RFC 1939-compliant POP3 server
//! implementation built on Tokio for asynchronous I/O.
//!
//! # RFC Compliance
//!
//! - **RFC 1939**: Post Office Protocol version 3 (full command set)
//! - **RFC 2449**: POP3 Extension Mechanism (CAPA command)
//! - **RFC 2595**: STLS extension for TLS upgrade
//!
//! # Supported Commands
//!
//! ## Authorization State
//! - `USER` / `PASS` — Username and password authentication
//! - `APOP` — MD5 digest authentication with timestamp challenge
//! - `CAPA` — Capability listing
//! - `STLS` — Initiate TLS upgrade (if enabled)
//! - `QUIT` — Disconnect without applying changes
//!
//! ## Transaction State
//! - `STAT` — Mailbox status (message count and total octets)
//! - `LIST` — Message listing with sizes
//! - `RETR` — Retrieve a complete message
//! - `DELE` — Mark a message for deletion
//! - `RSET` — Reset all deletion marks
//! - `TOP` — Retrieve message headers and N body lines
//! - `UIDL` — Unique ID listing per message
//! - `NOOP` — Keep connection alive
//! - `QUIT` — Enter Update state, commit deletions, then disconnect
//!
//! # Modules
//!
//! - `command`: POP3 command enumeration.
//! - `parser`: POP3 command line parser.
//! - `response`: POP3 response formatting (`+OK` / `-ERR`).
//! - `server`: Async TCP listener accepting POP3 connections.
//! - `session`: Per-connection state machine (Authorization → Transaction → Update).
//!
//! # Security
//!
//! APOP challenge generation uses `getrandom` for cryptographic quality randomness.
//! In production deployments, STLS should be enabled to protect USER/PASS credentials.

mod command;
mod parser;
mod response;
mod server;
mod session;

pub use command::Pop3Command;
pub use parser::parse_command;
pub use response::{Pop3Response, Pop3Status};
pub use server::Pop3Server;
pub use session::{Pop3Config, Pop3Session, Pop3State};
