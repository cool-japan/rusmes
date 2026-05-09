//! Mail transport abstraction for outgoing delivery
//!
//! The [`MailTransport`] trait decouples JMAP submission from a specific SMTP
//! client implementation, enabling dependency injection and testing with mock
//! transports.

use async_trait::async_trait;
use rusmes_proto::Mail;

/// SMTP envelope carrying sender/recipients for outgoing mail.
///
/// Separate from the [`Mail`] message so that the transport layer can override
/// RFC 5321 envelope addresses independently of RFC 5322 headers.
#[derive(Debug, Clone)]
pub struct SmtpEnvelope {
    /// RFC 5321 `MAIL FROM` address (bare, no angle brackets)
    pub mail_from: String,
    /// RFC 5321 `RCPT TO` addresses (bare, no angle brackets)
    pub rcpt_to: Vec<String>,
}

/// Abstraction over mail delivery — allows mocking in tests and swapping
/// delivery backends without touching higher-level code.
#[async_trait]
pub trait MailTransport: Send + Sync {
    /// Deliver a message immediately.
    ///
    /// Returns a server-assigned submission ID (typically a UUID).
    async fn send(&self, envelope: SmtpEnvelope, mail: &Mail) -> anyhow::Result<String>;

    /// Schedule delivery at a specific UTC instant.
    ///
    /// If `at` is in the past or within 5 seconds of now, implementations
    /// SHOULD deliver immediately.  Returns a submission ID that can be passed
    /// to [`cancel`](Self::cancel).
    async fn send_at(
        &self,
        envelope: SmtpEnvelope,
        mail: &Mail,
        at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<String>;

    /// Cancel a queued send.
    ///
    /// Returns `true` if the submission was still queued and has been removed,
    /// `false` if it has already been delivered or is unknown.
    async fn cancel(&self, submission_id: &str) -> anyhow::Result<bool>;
}

// ── NullMailTransport ─────────────────────────────────────────────────────────

/// A no-op [`MailTransport`] that records nothing and always reports success.
///
/// Useful as a default in contexts where outgoing SMTP delivery has not been
/// configured yet (e.g. `dispatch_method` in the JMAP handler layer before a
/// real relay has been wired in).
#[derive(Debug, Default)]
pub struct NullMailTransport;

#[async_trait]
impl MailTransport for NullMailTransport {
    async fn send(&self, _envelope: SmtpEnvelope, _mail: &Mail) -> anyhow::Result<String> {
        Ok(uuid::Uuid::new_v4().to_string())
    }

    async fn send_at(
        &self,
        _envelope: SmtpEnvelope,
        _mail: &Mail,
        _at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<String> {
        Ok(uuid::Uuid::new_v4().to_string())
    }

    async fn cancel(&self, _submission_id: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
}
