//! SMTP protocol implementation for RusMES
//!
//! This crate provides a full-featured, RFC 5321-compliant SMTP server
//! implementation built on Tokio for asynchronous I/O.
//!
//! # Features
//!
//! - **RFC 5321 Compliance**: Complete SMTP protocol including HELO/EHLO,
//!   MAIL FROM, RCPT TO, DATA, QUIT, RSET, NOOP, and VRFY.
//! - **STARTTLS** (RFC 3207): Opportunistic TLS upgrade for secure transport.
//! - **AUTH** (RFC 4954): Multiple SASL mechanisms:
//!   - PLAIN (RFC 4616)
//!   - LOGIN
//!   - CRAM-MD5 (RFC 2195)
//!   - SCRAM-SHA-256 (RFC 5802 / RFC 7677)
//! - **PIPELINING** (RFC 2920): Client-side pipelining support.
//! - **DSN** (RFC 3461): Delivery Status Notification extensions.
//! - **CHUNKING / BDAT** (RFC 3030): Binary data transfer without dot-stuffing.
//! - **SIZE** (RFC 1870): Message size declaration.
//! - **8BITMIME** (RFC 6152): 8-bit MIME content transfer.
//! - **SMTPUTF8** (RFC 6531): Unicode email addresses.
//! - **Submission** (RFC 4409 / RFC 6409): MSA mode on port 587.
//!
//! # Modules
//!
//! - [`auth`]: SASL authentication mechanism implementations.
//! - [`bdat`]: BDAT/CHUNKING extension state machine (RFC 3030).
//! - [`command`]: SMTP command types and parameters.
//! - [`dsn`]: Delivery Status Notification parameter parsing (RFC 3461).
//! - [`parser`]: Nom-based SMTP command parser.
//! - [`response`]: SMTP response code formatting.
//! - [`server`]: Async TCP listener and connection acceptor.
//! - [`session`]: Per-connection state machine and command dispatcher.
//! - [`submission`]: Mail Submission Agent (MSA) server variant.
//!
//! # Quick Start
//!
//! ```no_run
//! use std::sync::Arc;
//! use rusmes_smtp::{SmtpConfig, SmtpServer};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let config = SmtpConfig {
//!         hostname: "mail.example.com".to_string(),
//!         max_message_size: 10 * 1024 * 1024, // 10 MiB
//!         require_auth: false,
//!         enable_starttls: false,
//!         ..Default::default()
//!     };
//!     // Build and run the server (requires auth/storage backends)
//!     // let server = SmtpServer::new(config, auth_backend, storage_backend);
//!     // server.listen("0.0.0.0:25").await?;
//!     Ok(())
//! }
//! ```

pub mod auth;
pub mod bdat;
pub mod command;
pub mod dsn;
pub mod outbound_pool;
pub mod parser;
pub mod response;
pub mod server;
pub mod session;
pub mod submission;
pub mod transport;

#[cfg(test)]
mod tests;

use ipnetwork::IpNetwork;
use std::net::IpAddr;

pub use bdat::{BdatCommand, BdatError, BdatState};
pub use command::{MailParam, SmtpCommand};
pub use dsn::{DsnError, DsnMailParams, DsnNotify, DsnRcptParams, DsnRet};
pub use outbound_pool::{OutboundPool, OutboundPoolConfig, PooledConn, SmtpExtensions};
pub use parser::parse_command;
pub use response::SmtpResponse;
pub use server::SmtpServer;
pub use session::{SmtpConfig, SmtpSession, SmtpSessionHandler, SmtpState};
pub use submission::{SubmissionConfig, SubmissionServer};
pub use transport::SmtpMailTransport;

/// Check if an IP address is in any of the given CIDR networks
///
/// # Arguments
/// * `ip` - The IP address to check
/// * `networks` - Vector of CIDR network strings (e.g., "192.168.0.0/16")
///
/// # Returns
/// `true` if the IP is in any of the networks, `false` otherwise
pub fn is_ip_in_networks(ip: IpAddr, networks: &[String]) -> bool {
    for network_str in networks {
        if let Ok(network) = network_str.parse::<IpNetwork>() {
            if network.contains(ip) {
                return true;
            }
        } else {
            tracing::warn!("Invalid CIDR notation in relay_networks: {}", network_str);
        }
    }
    false
}
