//! Core protocol types and traits for RusMES
//!
//! This crate defines the fundamental data types shared across every component of the
//! RusMES mail server. It is intentionally free of network I/O and heavy dependencies
//! so that it can be compiled quickly and linked into all other crates.
//!
//! # Key Features
//!
//! - **Validated address types** — [`MailAddress`], [`Domain`], and [`Username`] enforce
//!   RFC constraints at construction time and expose infallible accessors.
//! - **Mail state machine** — [`MailState`] models the lifecycle of a mail item as it
//!   flows through the mailet processing pipeline (Root → Transport →
//!   LocalDelivery / Error / Ghost / Custom).
//! - **Mail envelope** — [`Mail`] wraps a [`MimeMessage`] with SMTP envelope data
//!   (sender, recipients, remote IP/host), custom attributes for mailet communication,
//!   and a split operation for fan-out delivery.
//! - **MIME message** — [`MimeMessage`] provides parsed header access and body handling
//!   for both in-memory (`Small`) and streaming (`Large`) messages, including multipart
//!   parsing, base64 / quoted-printable decoding, and Content-Type inspection.
//! - **Typed identifiers** — UUID-backed [`MailId`] and [`MessageId`] for unambiguous
//!   tracking of mail items and messages throughout the system.
//!
//! # Module Overview
//!
//! | Module       | Contents                                              |
//! |--------------|-------------------------------------------------------|
//! | [`address`]  | `MailAddress`, `Domain`, `Username`                  |
//! | [`error`]    | `MailError`, `Result` alias                          |
//! | [`mail`]     | `Mail`, `MailId`, `MailState`, `AttributeValue`      |
//! | [`message`]  | `MimeMessage`, `MessageId`, `HeaderMap`, `MessageBody`|
//! | [`mime`]     | MIME parsing helpers, `ContentType`, `MimePart`      |
//!
//! # Brief Usage Example
//!
//! ```rust
//! use rusmes_proto::{Domain, MailAddress, Mail, MailState};
//! use rusmes_proto::message::{HeaderMap, MessageBody, MimeMessage};
//! use bytes::Bytes;
//!
//! // Build a validated address
//! let domain = Domain::new("example.com").expect("valid domain");
//! let addr = MailAddress::new("alice", domain).expect("valid address");
//! assert_eq!(addr.to_string(), "alice@example.com");
//!
//! // Create a mail envelope
//! let headers = HeaderMap::new();
//! let body = MessageBody::Small(Bytes::from("Hello, world!"));
//! let msg = MimeMessage::new(headers, body);
//! let mail = Mail::new(Some(addr), vec![], msg, None, None);
//! assert_eq!(mail.state, MailState::Root);
//! ```
//!
//! # Relevant Standards
//!
//! - Email address syntax: [RFC 5321 §4.1.2](https://www.rfc-editor.org/rfc/rfc5321#section-4.1.2),
//!   [RFC 5322 §3.4](https://www.rfc-editor.org/rfc/rfc5322#section-3.4)
//! - MIME: [RFC 2045](https://www.rfc-editor.org/rfc/rfc2045),
//!   [RFC 2046](https://www.rfc-editor.org/rfc/rfc2046),
//!   [RFC 2047](https://www.rfc-editor.org/rfc/rfc2047)
//! - Domain names: [RFC 1035](https://www.rfc-editor.org/rfc/rfc1035),
//!   [RFC 5891](https://www.rfc-editor.org/rfc/rfc5891) (IDNA)

pub mod address;
pub mod error;
pub mod mail;
pub mod message;
pub mod mime;

pub use address::{Domain, MailAddress, Username};
pub use error::{MailError, Result};
pub use mail::{AttributeValue, Mail, MailId, MailState};
pub use message::{HeaderMap, MessageBody, MessageId, MimeMessage};
pub use mime::{ContentTransferEncoding, ContentType, MimePart};
