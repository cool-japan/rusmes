//! RusMES server orchestration library
//!
//! This crate is the top-level runtime for the RusMES mail server. It brings together
//! all protocol servers (SMTP, IMAP, POP3, JMAP), the mailet processing pipeline,
//! storage backends, authentication, metrics collection, and connection management
//! into a single, cohesive process.
//!
//! # Key Features
//!
//! - **Multi-protocol**: Simultaneously runs SMTP, IMAP, POP3, and JMAP servers.
//! - **Mailet pipeline**: Configurable chain of mail-processing mailets (spam filtering,
//!   local delivery, remote delivery, DKIM/SPF/DMARC checking, etc.) driven by the
//!   [`rusmes_core`] processing engine.
//! - **Graceful shutdown**: Responds to SIGTERM and SIGINT with clean shutdown of all
//!   protocol listeners.
//! - **Hot configuration reload**: On SIGHUP the server re-reads `rusmes.toml` and
//!   applies changes to logging levels and rate-limit settings without restart.
//! - **Pluggable authentication**: Supports file-based, LDAP, SQL, and OAuth2 backends
//!   via the [`rusmes_auth`] crate.
//! - **Connection limiting**: Per-IP connection caps and idle-timeout enforcement via
//!   the `connection_limits` module.
//! - **Structured session logging**: UUID-based session IDs attached to every log event
//!   for easy per-connection trace reconstruction (see [`session_logging`]).
//!
//! # Binary entry point
//!
//! The crate ships a `rusmes-server` binary whose `main` function:
//! 1. Reads configuration from a TOML file (default `rusmes.toml`, overridden by the
//!    first CLI argument).
//! 2. Initialises storage, metrics, authentication, and rate-limiting.
//! 3. Spawns each enabled protocol server as an independent Tokio task.
//! 4. Enters a `select!` loop waiting for OS signals or server task failures.
//!
//! # Brief Usage Example
//!
//! ```bash
//! # Run the server with the default config path
//! rusmes-server
//!
//! # Or with a custom config:
//! rusmes-server /etc/rusmes/rusmes.toml
//! ```
//!
//! # Relevant Standards
//!
//! - SMTP: [RFC 5321](https://www.rfc-editor.org/rfc/rfc5321), [RFC 6531](https://www.rfc-editor.org/rfc/rfc6531)
//! - IMAP4rev2: [RFC 9051](https://www.rfc-editor.org/rfc/rfc9051)
//! - POP3: [RFC 1939](https://www.rfc-editor.org/rfc/rfc1939)
//! - JMAP: [RFC 8620](https://www.rfc-editor.org/rfc/rfc8620), [RFC 8621](https://www.rfc-editor.org/rfc/rfc8621)

pub mod session_logging;
