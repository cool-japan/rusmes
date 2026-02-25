//! # rusmes-core — Mailet Processing Engine for RusMES
//!
//! `rusmes-core` is the central mail-processing library for [RusMES], providing a composable
//! pipeline of *matchers* and *mailets* that evaluate and transform incoming mail messages.
//! It is modelled after the Apache James mailet API but implemented from scratch in async Rust.
//!
//! ## Architecture
//!
//! Mail processing flows through a [`Processor`] that applies a sequence of
//! [`ProcessingStep`]s.  Each step pairs a [`Matcher`] (decides *whether* to act) with a
//! [`Mailet`] (decides *what* to do).
//!
//! ```text
//!  ┌─────────┐     ┌───────────────────────────────────────────────────┐
//!  │  Mail   │────▶│  Processor                                        │
//!  └─────────┘     │   Step 1: Matcher → Mailet (e.g. SpfCheck)        │
//!                  │   Step 2: Matcher → Mailet (e.g. DkimVerify)      │
//!                  │   Step 3: Matcher → Mailet (e.g. SieveMailet)     │
//!                  │   Step N: …                                       │
//!                  └───────────────────────────────────────────────────┘
//! ```
//!
//! ## Included Mailets
//!
//! | Mailet | Purpose |
//! |--------|---------|
//! | [`mailets::AddHeaderMailet`] | Append arbitrary headers |
//! | [`mailets::BounceMailet`] | RFC 3464 DSN bounce generation |
//! | [`mailets::DkimVerifyMailet`] | DKIM signature verification (RFC 6376) |
//! | [`mailets::DmarcVerifyMailet`] | DMARC policy enforcement (RFC 7489) |
//! | [`mailets::DnsblMailet`] | DNS-based block-list lookups |
//! | [`mailets::ForwardMailet`] | Forwarding with loop detection |
//! | [`mailets::GreylistMailet`] | Greylisting anti-spam |
//! | [`mailets::LegalisMailet`] | Legal archiving with RFC 3161 timestamps |
//! | [`mailets::LocalDeliveryMailet`] | Local mailbox delivery |
//! | `mailets::OxifyMailet` | AI-powered mail enrichment |
//! | [`mailets::RemoteDeliveryMailet`] | SMTP relay delivery (Pure Rust / rustls) |
//! | [`mailets::RemoveMimeHeaderMailet`] | Strip MIME headers by pattern |
//! | [`mailets::SieveMailet`] | RFC 5228 Sieve script filtering |
//! | [`mailets::SpamAssassinMailet`] | SpamAssassin integration |
//! | [`mailets::SpfCheckMailet`] | SPF policy checking (RFC 7208) |
//! | [`mailets::VirusScanMailet`] | ClamAV / virus-scan integration |
//!
//! ## Included Matchers
//!
//! | Matcher | Purpose |
//! |---------|---------|
//! | `CompositeAll` / `CompositeAny` | Boolean composition of other matchers |
//! | `HasAttachment` | True when mail has MIME attachments |
//! | `HeaderContains` | True when a header contains a substring |
//! | `IsInBlacklist` | Sender/recipient on a configurable deny-list |
//! | `IsInWhitelist` | Sender/recipient on a configurable allow-list |
//! | `RecipientIsLocal` | True when recipient is a local domain |
//! | `RemoteAddress` | True when client IP matches a CIDR range |
//! | `SenderIs` | Exact or wildcard sender address match |
//! | `SizeGreaterThan` | True when message exceeds a byte threshold |
//!
//! ## Protocol Support
//!
//! - **SMTP** – mail reception and relay (`rusmes-smtp`)
//! - **IMAP4** – mail access (`rusmes-imap`)
//! - **JMAP** – JSON Meta Application Protocol (`rusmes-jmap`)
//! - **POP3** – legacy mail retrieval (`rusmes-pop3`)
//!
//! ## Feature Flags
//!
//! This crate currently has no optional feature flags; all components are always compiled.
//!
//! ## Quick-start Example
//!
//! ```rust,no_run
//! use rusmes_core::{Processor, ProcessingStep};
//! use rusmes_proto::MailState;
//! use std::sync::Arc;
//!
//! # async fn build_processor() {
//! // Build a basic processor that evaluates mail in the Root state
//! let mut processor = Processor::new("transport", MailState::Root);
//!
//! // Steps are (matcher, mailet) pairs; add them via add_step(ProcessingStep::new(…))
//! // See individual mailet and matcher types in the rusmes_core::mailets /
//! // rusmes_core::matchers modules for concrete implementations.
//! # }
//! ```
//!
//! [RusMES]: https://github.com/cool-japan/rusmes

pub mod bounce;
pub mod dsn;
pub mod factory;
pub mod mailets;
pub mod matchers;
pub mod queue;
pub mod rate_limit;
pub mod sieve;

mod matcher;
mod processor;

mod mailet;
mod router;

pub use bounce::generate_bounce;
pub use mailet::{Mailet, MailetAction, MailetConfig};
pub use matcher::Matcher;
pub use processor::{ProcessingStep, Processor};
pub use queue::{
    FilesystemQueueStore, MailQueue, Priority, PriorityConfig, PriorityQueue, PriorityStats,
    QueueEntry, QueueEntryData, QueueStats, QueueStore,
};
pub use rate_limit::{RateLimitConfig, RateLimiter};
pub use router::MailProcessorRouter;
pub use sieve::{
    SieveAction, SieveCommand, SieveContext, SieveInterpreter, SieveScript, SieveTest,
};

#[cfg(test)]
pub mod test_helpers {
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, Mail, MailAddress, MessageBody, MimeMessage};
    use std::str::FromStr;

    /// Create a test mail with minimal setup
    pub fn create_test_mail(sender: &str, recipients: Vec<&str>) -> Mail {
        let sender_addr = MailAddress::from_str(sender).ok();
        let recipient_addrs: Vec<MailAddress> = recipients
            .iter()
            .filter_map(|r| MailAddress::from_str(r).ok())
            .collect();

        let message = MimeMessage::new(
            HeaderMap::new(),
            MessageBody::Small(Bytes::from("Test message")),
        );

        Mail::new(sender_addr, recipient_addrs, message, None, None)
    }
}
