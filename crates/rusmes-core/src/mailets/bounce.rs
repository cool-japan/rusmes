//! Bounce message generation mailet (RFC 3464 - DSN)

use crate::dsn::{smtp_diagnostic_text, smtp_to_enhanced_code};
use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::Mail;
use std::time::{SystemTime, UNIX_EPOCH};

/// SMTP status code
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SmtpStatusCode {
    /// 4xx - Temporary failure
    TemporaryFailure(u16),
    /// 5xx - Permanent failure
    PermanentFailure(u16),
}

impl SmtpStatusCode {
    /// Parse from status code
    pub fn from_code(code: u16) -> Option<Self> {
        match code {
            400..=499 => Some(SmtpStatusCode::TemporaryFailure(code)),
            500..=599 => Some(SmtpStatusCode::PermanentFailure(code)),
            _ => None,
        }
    }

    /// Get numeric code
    pub fn code(&self) -> u16 {
        match self {
            SmtpStatusCode::TemporaryFailure(c) | SmtpStatusCode::PermanentFailure(c) => *c,
        }
    }

    /// Check if permanent
    pub fn is_permanent(&self) -> bool {
        matches!(self, SmtpStatusCode::PermanentFailure(_))
    }

    /// Get enhanced status code (RFC 3463)
    pub fn enhanced_code(&self) -> String {
        smtp_to_enhanced_code(self.code()).to_string()
    }

    /// Get diagnostic text
    pub fn diagnostic_text(&self) -> &str {
        smtp_diagnostic_text(self.code())
    }
}

/// DSN action type (RFC 3464)
#[derive(Debug, Clone, PartialEq)]
pub enum DsnAction {
    Failed,
    Delayed,
    Delivered,
    Relayed,
    Expanded,
}

impl DsnAction {
    pub fn as_str(&self) -> &str {
        match self {
            DsnAction::Failed => "failed",
            DsnAction::Delayed => "delayed",
            DsnAction::Delivered => "delivered",
            DsnAction::Relayed => "relayed",
            DsnAction::Expanded => "expanded",
        }
    }
}

/// Delivery Status Notification (DSN)
#[derive(Debug, Clone)]
pub struct DeliveryStatusNotification {
    /// Reporting MTA
    pub reporting_mta: String,
    /// Arrival date
    pub arrival_date: u64,
    /// Per-recipient fields
    pub recipients: Vec<DsnRecipient>,
}

/// Per-recipient DSN information
#[derive(Debug, Clone)]
pub struct DsnRecipient {
    /// Final recipient
    pub final_recipient: String,
    /// Action taken
    pub action: DsnAction,
    /// Status code (enhanced)
    pub status: String,
    /// Diagnostic code (optional)
    pub diagnostic_code: Option<String>,
    /// Remote MTA (optional)
    pub remote_mta: Option<String>,
    /// Last attempt date
    pub last_attempt_date: Option<u64>,
}

impl DeliveryStatusNotification {
    /// Create a new DSN
    pub fn new(reporting_mta: String) -> Self {
        let arrival_date = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            reporting_mta,
            arrival_date,
            recipients: Vec::new(),
        }
    }

    /// Add a recipient
    pub fn add_recipient(&mut self, recipient: DsnRecipient) {
        self.recipients.push(recipient);
    }

    /// Generate RFC 3464 message body (machine-readable delivery status)
    pub fn generate_message_body(&self) -> String {
        let mut body = String::new();

        // Per-message DSN fields
        body.push_str(&format!("Reporting-MTA: dns; {}\n", self.reporting_mta));
        body.push_str(&format!(
            "Arrival-Date: {}\n",
            self.format_date(self.arrival_date)
        ));
        body.push('\n');

        // Per-recipient DSN fields
        for recipient in &self.recipients {
            body.push_str(&format!(
                "Final-Recipient: rfc822; {}\n",
                recipient.final_recipient
            ));
            body.push_str(&format!("Action: {}\n", recipient.action.as_str()));
            body.push_str(&format!("Status: {}\n", recipient.status));

            if let Some(ref diagnostic) = recipient.diagnostic_code {
                body.push_str(&format!("Diagnostic-Code: smtp; {}\n", diagnostic));
            }

            if let Some(ref remote_mta) = recipient.remote_mta {
                body.push_str(&format!("Remote-MTA: dns; {}\n", remote_mta));
            }

            if let Some(date) = recipient.last_attempt_date {
                body.push_str(&format!("Last-Attempt-Date: {}\n", self.format_date(date)));
            }

            body.push('\n');
        }

        body
    }

    /// Generate human-readable explanation text
    pub fn generate_human_text(&self, error_code: SmtpStatusCode, error_message: &str) -> String {
        let mut text = String::new();

        text.push_str("This is an automatically generated Delivery Status Notification.\n\n");

        if error_code.is_permanent() {
            text.push_str("YOUR MESSAGE COULD NOT BE DELIVERED to the following recipients:\n\n");
        } else {
            text.push_str(
                "DELIVERY OF YOUR MESSAGE HAS BEEN DELAYED to the following recipients:\n\n",
            );
        }

        for recipient in &self.recipients {
            text.push_str(&format!("  {}\n", recipient.final_recipient));
        }

        text.push('\n');
        text.push_str(&format!(
            "Reason: {} {}\n",
            error_code.code(),
            error_message
        ));
        text.push_str(&format!(
            "Enhanced Status Code: {}\n",
            error_code.enhanced_code()
        ));
        text.push_str(&format!("Diagnostic: {}\n", error_code.diagnostic_text()));
        text.push('\n');

        if error_code.is_permanent() {
            text.push_str("No further delivery attempts will be made.\n");
        } else {
            text.push_str(
                "Delivery will be retried. You will be notified if delivery continues to fail.\n",
            );
        }

        text
    }

    fn format_date(&self, timestamp: u64) -> String {
        use chrono::{DateTime, Utc};

        if let Some(dt) = DateTime::<Utc>::from_timestamp(timestamp as i64, 0) {
            dt.to_rfc2822()
        } else {
            format!("timestamp:{}", timestamp)
        }
    }
}

/// Bounce mailet - generates DSN messages
pub struct BounceMailet {
    name: String,
    /// Reporting MTA hostname
    reporting_mta: String,
    /// Postmaster address
    postmaster: String,
    /// Include original message headers
    include_headers: bool,
    /// Include original message body
    include_body: bool,
    /// Maximum body size to include
    max_body_size: usize,
}

impl BounceMailet {
    /// Create a new bounce mailet
    pub fn new() -> Self {
        Self {
            name: "Bounce".to_string(),
            reporting_mta: "localhost".to_string(),
            postmaster: "postmaster@localhost".to_string(),
            include_headers: true,
            include_body: false,
            max_body_size: 1024,
        }
    }

    /// Generate a bounce message (RFC 3464 compliant multipart/report)
    pub fn generate_bounce(
        &self,
        mail: &Mail,
        error_code: SmtpStatusCode,
        error_message: &str,
    ) -> String {
        let mut dsn = DeliveryStatusNotification::new(self.reporting_mta.clone());

        // Add recipient information
        for recipient in mail.recipients() {
            let action = if error_code.is_permanent() {
                DsnAction::Failed
            } else {
                DsnAction::Delayed
            };

            let recipient_dsn = DsnRecipient {
                final_recipient: recipient.to_string(),
                action,
                status: error_code.enhanced_code(),
                diagnostic_code: Some(format!("{} {}", error_code.code(), error_message)),
                remote_mta: mail
                    .get_attribute("smtp.remote_mta")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                last_attempt_date: Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                ),
            };

            dsn.add_recipient(recipient_dsn);
        }

        // Generate RFC 3464 compliant multipart/report message
        self.generate_multipart_report(&dsn, mail, error_code, error_message)
    }

    /// Generate RFC 3464 compliant multipart/report message
    fn generate_multipart_report(
        &self,
        dsn: &DeliveryStatusNotification,
        mail: &Mail,
        error_code: SmtpStatusCode,
        error_message: &str,
    ) -> String {
        let mut message = String::new();
        let boundary = "rusmes-dsn-boundary-7b3a9f1e";

        // Main MIME headers for multipart/report
        message.push_str("MIME-Version: 1.0\n");
        message.push_str(&format!(
            "Content-Type: multipart/report; report-type=delivery-status; boundary=\"{}\"\n",
            boundary
        ));
        message.push('\n');
        message.push_str("This is a MIME-encapsulated message.\n");
        message.push('\n');

        // Part 1: Human-readable explanation (text/plain)
        message.push_str(&format!("--{}\n", boundary));
        message.push_str("Content-Type: text/plain; charset=utf-8\n");
        message.push_str("Content-Description: Notification\n");
        message.push('\n');
        message.push_str(&dsn.generate_human_text(error_code, error_message));
        message.push('\n');

        let msg_id = mail.message_id();
        message.push_str(&format!("Original Message-ID: {}\n", msg_id));
        message.push('\n');

        // Part 2: Machine-readable delivery status (message/delivery-status)
        message.push_str(&format!("--{}\n", boundary));
        message.push_str("Content-Type: message/delivery-status\n");
        message.push_str("Content-Description: Delivery Report\n");
        message.push('\n');
        message.push_str(&dsn.generate_message_body());

        // Part 3: Original message or headers (text/rfc822-headers or message/rfc822)
        message.push_str(&format!("--{}\n", boundary));

        if self.include_body {
            // Include entire original message
            message.push_str("Content-Type: message/rfc822\n");
            message.push_str("Content-Description: Undelivered Message\n");
            message.push('\n');

            // Include headers
            self.append_original_headers(&mut message, mail);

            // Include body
            if let Some(body) = mail.get_attribute("message.body").and_then(|v| v.as_str()) {
                message.push('\n');
                let truncated_body = if body.len() > self.max_body_size {
                    format!("{}... (truncated)", &body[..self.max_body_size])
                } else {
                    body.to_string()
                };
                message.push_str(&truncated_body);
                message.push('\n');
            }
        } else if self.include_headers {
            // Include headers only
            message.push_str("Content-Type: text/rfc822-headers\n");
            message.push_str("Content-Description: Undelivered Message Headers\n");
            message.push('\n');
            self.append_original_headers(&mut message, mail);
        }

        // End boundary
        message.push_str(&format!("--{}--\n", boundary));

        message
    }

    /// Append original message headers to the bounce message
    fn append_original_headers(&self, message: &mut String, mail: &Mail) {
        if let Some(subject) = mail
            .get_attribute("header.Subject")
            .and_then(|v| v.as_str())
        {
            message.push_str(&format!("Subject: {}\n", subject));
        }
        if let Some(from) = mail.get_attribute("header.From").and_then(|v| v.as_str()) {
            message.push_str(&format!("From: {}\n", from));
        }
        if let Some(to) = mail.get_attribute("header.To").and_then(|v| v.as_str()) {
            message.push_str(&format!("To: {}\n", to));
        }
        if let Some(date) = mail.get_attribute("header.Date").and_then(|v| v.as_str()) {
            message.push_str(&format!("Date: {}\n", date));
        }
        if let Some(cc) = mail.get_attribute("header.Cc").and_then(|v| v.as_str()) {
            message.push_str(&format!("Cc: {}\n", cc));
        }
        if let Some(msg_id) = mail
            .get_attribute("header.Message-ID")
            .and_then(|v| v.as_str())
        {
            message.push_str(&format!("Message-ID: {}\n", msg_id));
        }
    }
}

impl Default for BounceMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for BounceMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        if let Some(mta) = config.get_param("reporting_mta") {
            self.reporting_mta = mta.to_string();
        }

        if let Some(postmaster) = config.get_param("postmaster") {
            self.postmaster = postmaster.to_string();
        }

        if let Some(include) = config.get_param("include_headers") {
            self.include_headers = include.parse().unwrap_or(true);
        }

        if let Some(include) = config.get_param("include_body") {
            self.include_body = include.parse().unwrap_or(false);
        }

        if let Some(max_size) = config.get_param("max_body_size") {
            self.max_body_size = max_size.parse().unwrap_or(1024);
        }

        tracing::info!("Initialized BounceMailet");
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        // Check if this mail needs a bounce
        let should_bounce = mail
            .get_attribute("bounce.required")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !should_bounce {
            return Ok(MailetAction::Continue);
        }

        // Get error information
        let error_code_num = mail
            .get_attribute("bounce.error_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(550) as u16;

        let error_message = mail
            .get_attribute("bounce.error_message")
            .and_then(|v| v.as_str())
            .unwrap_or("Delivery failed");

        let error_code = SmtpStatusCode::from_code(error_code_num)
            .unwrap_or(SmtpStatusCode::PermanentFailure(550));

        tracing::info!(
            "Generating bounce for mail {} with code {}",
            mail.id(),
            error_code.code()
        );

        // Generate bounce message
        let bounce_body = self.generate_bounce(mail, error_code, error_message);

        // Store bounce in mail attributes for delivery
        mail.set_attribute("bounce.generated", true);
        mail.set_attribute("bounce.body", bounce_body);

        Ok(MailetAction::Continue)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};
    use std::str::FromStr;

    #[test]
    fn test_smtp_status_code_parsing() {
        assert_eq!(
            SmtpStatusCode::from_code(450),
            Some(SmtpStatusCode::TemporaryFailure(450))
        );
        assert_eq!(
            SmtpStatusCode::from_code(550),
            Some(SmtpStatusCode::PermanentFailure(550))
        );
        assert_eq!(SmtpStatusCode::from_code(250), None);
    }

    #[test]
    fn test_smtp_status_code_permanent() {
        assert!(SmtpStatusCode::PermanentFailure(550).is_permanent());
        assert!(!SmtpStatusCode::TemporaryFailure(450).is_permanent());
    }

    #[test]
    fn test_smtp_enhanced_codes() {
        assert_eq!(
            SmtpStatusCode::PermanentFailure(550).enhanced_code(),
            "5.1.1"
        );
        assert_eq!(
            SmtpStatusCode::TemporaryFailure(450).enhanced_code(),
            "4.2.1"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(552).enhanced_code(),
            "5.2.2"
        );
    }

    #[test]
    fn test_smtp_diagnostic_text() {
        assert_eq!(
            SmtpStatusCode::PermanentFailure(550).diagnostic_text(),
            "Requested action not taken: mailbox unavailable"
        );
    }

    #[test]
    fn test_dsn_action_as_str() {
        assert_eq!(DsnAction::Failed.as_str(), "failed");
        assert_eq!(DsnAction::Delayed.as_str(), "delayed");
        assert_eq!(DsnAction::Delivered.as_str(), "delivered");
    }

    #[test]
    fn test_dsn_creation() {
        let dsn = DeliveryStatusNotification::new("mail.example.com".to_string());
        assert_eq!(dsn.reporting_mta, "mail.example.com");
        assert!(dsn.recipients.is_empty());
    }

    #[test]
    fn test_dsn_add_recipient() {
        let mut dsn = DeliveryStatusNotification::new("mail.example.com".to_string());
        let recipient = DsnRecipient {
            final_recipient: "user@example.com".to_string(),
            action: DsnAction::Failed,
            status: "5.1.1".to_string(),
            diagnostic_code: Some("550 User unknown".to_string()),
            remote_mta: None,
            last_attempt_date: None,
        };

        dsn.add_recipient(recipient);
        assert_eq!(dsn.recipients.len(), 1);
    }

    #[test]
    fn test_dsn_message_body_generation() {
        let mut dsn = DeliveryStatusNotification::new("mail.example.com".to_string());
        let recipient = DsnRecipient {
            final_recipient: "user@example.com".to_string(),
            action: DsnAction::Failed,
            status: "5.1.1".to_string(),
            diagnostic_code: Some("550 User unknown".to_string()),
            remote_mta: Some("remote.example.com".to_string()),
            last_attempt_date: None,
        };

        dsn.add_recipient(recipient);
        let body = dsn.generate_message_body();

        assert!(body.contains("Reporting-MTA: dns; mail.example.com"));
        assert!(body.contains("Final-Recipient: rfc822; user@example.com"));
        assert!(body.contains("Action: failed"));
        assert!(body.contains("Status: 5.1.1"));
        assert!(body.contains("Diagnostic-Code: smtp; 550 User unknown"));
        assert!(body.contains("Remote-MTA: dns; remote.example.com"));
    }

    #[tokio::test]
    async fn test_bounce_mailet_init() {
        let mut mailet = BounceMailet::new();
        let config = MailetConfig::new("Bounce");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.name(), "Bounce");
    }

    #[tokio::test]
    async fn test_bounce_mailet_no_bounce_required() {
        let mailet = BounceMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("rcpt@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert!(mail.get_attribute("bounce.generated").is_none());
    }

    #[tokio::test]
    async fn test_bounce_mailet_generate_bounce() {
        let mailet = BounceMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("unknown@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mail.set_attribute("bounce.required", true);
        mail.set_attribute("bounce.error_code", 550_i64);
        mail.set_attribute("bounce.error_message", "User unknown");

        let action = mailet.service(&mut mail).await.unwrap();
        assert_eq!(action, MailetAction::Continue);
        assert_eq!(
            mail.get_attribute("bounce.generated")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(mail.get_attribute("bounce.body").is_some());
    }

    #[tokio::test]
    async fn test_bounce_message_content() {
        let mailet = BounceMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mail.set_attribute("header.Subject", "Test message");
        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Delivery Status Notification"));
        assert!(bounce.contains("user@test.com"));
        assert!(bounce.contains("550"));
        assert!(bounce.contains("User unknown"));
        assert!(bounce.contains("5.1.1")); // Enhanced status code
    }

    #[tokio::test]
    async fn test_bounce_config_reporting_mta() {
        let mut mailet = BounceMailet::new();
        let config = MailetConfig::new("Bounce").with_param("reporting_mta", "mail.example.com");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.reporting_mta, "mail.example.com");
    }

    #[tokio::test]
    async fn test_bounce_config_postmaster() {
        let mut mailet = BounceMailet::new();
        let config = MailetConfig::new("Bounce").with_param("postmaster", "postmaster@example.com");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.postmaster, "postmaster@example.com");
    }

    #[tokio::test]
    async fn test_bounce_config_include_headers() {
        let mut mailet = BounceMailet::new();
        let config = MailetConfig::new("Bounce").with_param("include_headers", "false");

        mailet.init(config).await.unwrap();
        assert!(!mailet.include_headers);
    }

    #[tokio::test]
    async fn test_bounce_config_include_body() {
        let mut mailet = BounceMailet::new();
        let config = MailetConfig::new("Bounce").with_param("include_body", "true");

        mailet.init(config).await.unwrap();
        assert!(mailet.include_body);
    }

    #[tokio::test]
    async fn test_bounce_config_max_body_size() {
        let mut mailet = BounceMailet::new();
        let config = MailetConfig::new("Bounce").with_param("max_body_size", "2048");

        mailet.init(config).await.unwrap();
        assert_eq!(mailet.max_body_size, 2048);
    }

    #[tokio::test]
    async fn test_bounce_multipart_structure() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        // Check multipart structure (old style boundary for backwards compatibility)
        assert!(bounce.contains("Content-Type: text/plain"));
        assert!(bounce.contains("Content-Type: message/delivery-status"));
    }

    #[tokio::test]
    async fn test_bounce_temporary_failure() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::TemporaryFailure(450);
        let bounce = mailet.generate_bounce(&mail, error_code, "Mailbox unavailable");

        assert!(bounce.contains("450"));
        assert!(bounce.contains("4.2.1")); // Enhanced code for temporary
        assert!(bounce.contains("Mailbox unavailable"));
    }

    #[tokio::test]
    async fn test_bounce_include_original_headers() {
        let mut mailet = BounceMailet::new();
        mailet.include_headers = true;

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "Test Subject");
        mail.set_attribute("header.From", "sender@test.com");

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Subject: Test Subject"));
        assert!(bounce.contains("From: sender@test.com"));
    }

    #[tokio::test]
    async fn test_bounce_include_body_truncated() {
        let mut mailet = BounceMailet::new();
        mailet.include_body = true;
        mailet.max_body_size = 10;

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute(
            "message.body",
            "This is a very long message body that should be truncated",
        );

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("(truncated)"));
    }

    #[tokio::test]
    async fn test_bounce_multiple_recipients() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![
                MailAddress::from_str("user1@test.com").unwrap(),
                MailAddress::from_str("user2@test.com").unwrap(),
            ],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "Users unknown");

        assert!(bounce.contains("user1@test.com"));
        assert!(bounce.contains("user2@test.com"));
    }

    #[tokio::test]
    async fn test_bounce_with_remote_mta() {
        let mailet = BounceMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("smtp.remote_mta", "remote.example.com");

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Remote-MTA: dns; remote.example.com"));
    }

    #[test]
    fn test_smtp_status_code_all_enhanced_codes() {
        // Test all specific mappings
        assert_eq!(
            SmtpStatusCode::TemporaryFailure(421).enhanced_code(),
            "4.4.2"
        );
        assert_eq!(
            SmtpStatusCode::TemporaryFailure(450).enhanced_code(),
            "4.2.1"
        );
        assert_eq!(
            SmtpStatusCode::TemporaryFailure(451).enhanced_code(),
            "4.3.0"
        );
        assert_eq!(
            SmtpStatusCode::TemporaryFailure(452).enhanced_code(),
            "4.2.2"
        );
        assert_eq!(
            SmtpStatusCode::TemporaryFailure(454).enhanced_code(),
            "4.7.0"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(550).enhanced_code(),
            "5.1.1"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(551).enhanced_code(),
            "5.1.6"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(552).enhanced_code(),
            "5.2.2"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(553).enhanced_code(),
            "5.1.3"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(554).enhanced_code(),
            "5.7.1"
        );
    }

    #[test]
    fn test_smtp_status_code_500_codes() {
        assert_eq!(
            SmtpStatusCode::PermanentFailure(500).enhanced_code(),
            "5.5.2"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(501).enhanced_code(),
            "5.5.4"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(502).enhanced_code(),
            "5.5.1"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(503).enhanced_code(),
            "5.5.1"
        );
        assert_eq!(
            SmtpStatusCode::PermanentFailure(504).enhanced_code(),
            "5.5.4"
        );
    }

    #[test]
    fn test_dsn_human_text_permanent() {
        let mut dsn = DeliveryStatusNotification::new("mail.example.com".to_string());
        dsn.add_recipient(DsnRecipient {
            final_recipient: "user@test.com".to_string(),
            action: DsnAction::Failed,
            status: "5.1.1".to_string(),
            diagnostic_code: None,
            remote_mta: None,
            last_attempt_date: None,
        });

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let text = dsn.generate_human_text(error_code, "User unknown");

        assert!(text.contains("COULD NOT BE DELIVERED"));
        assert!(text.contains("user@test.com"));
        assert!(text.contains("550"));
        assert!(text.contains("User unknown"));
        assert!(text.contains("No further delivery attempts"));
    }

    #[test]
    fn test_dsn_human_text_temporary() {
        let mut dsn = DeliveryStatusNotification::new("mail.example.com".to_string());
        dsn.add_recipient(DsnRecipient {
            final_recipient: "user@test.com".to_string(),
            action: DsnAction::Delayed,
            status: "4.2.1".to_string(),
            diagnostic_code: None,
            remote_mta: None,
            last_attempt_date: None,
        });

        let error_code = SmtpStatusCode::TemporaryFailure(450);
        let text = dsn.generate_human_text(error_code, "Mailbox unavailable");

        assert!(text.contains("DELAYED"));
        assert!(text.contains("user@test.com"));
        assert!(text.contains("450"));
        assert!(text.contains("Mailbox unavailable"));
        assert!(text.contains("Delivery will be retried"));
    }

    #[tokio::test]
    async fn test_multipart_report_structure() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(
                HeaderMap::new(),
                MessageBody::Small(Bytes::from("Test body")),
            ),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        // Check RFC 3464 compliance
        assert!(bounce.contains("MIME-Version: 1.0"));
        assert!(bounce.contains("multipart/report"));
        assert!(bounce.contains("report-type=delivery-status"));
        assert!(bounce.contains("Content-Type: text/plain"));
        assert!(bounce.contains("Content-Type: message/delivery-status"));
    }

    #[tokio::test]
    async fn test_multipart_with_full_message() {
        let mut mailet = BounceMailet::new();
        mailet.include_body = true;

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("message.body", "This is the original message body");

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Content-Type: message/rfc822"));
        assert!(bounce.contains("Undelivered Message"));
        assert!(bounce.contains("original message body"));
    }

    #[tokio::test]
    async fn test_multipart_headers_only() {
        let mut mailet = BounceMailet::new();
        mailet.include_headers = true;
        mailet.include_body = false;

        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "Test Subject");

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Content-Type: text/rfc822-headers"));
        assert!(bounce.contains("Undelivered Message Headers"));
        assert!(bounce.contains("Subject: Test Subject"));
    }

    // Additional comprehensive tests for RFC 3464 compliance

    #[test]
    fn test_smtp_code_421_connection_timeout() {
        let code = SmtpStatusCode::TemporaryFailure(421);
        assert_eq!(code.enhanced_code(), "4.4.2");
        assert!(!code.is_permanent());
    }

    #[test]
    fn test_smtp_code_450_mailbox_unavailable() {
        let code = SmtpStatusCode::TemporaryFailure(450);
        assert_eq!(code.enhanced_code(), "4.2.1");
        assert!(!code.is_permanent());
    }

    #[test]
    fn test_smtp_code_451_local_error() {
        let code = SmtpStatusCode::TemporaryFailure(451);
        assert_eq!(code.enhanced_code(), "4.3.0");
        assert!(!code.is_permanent());
    }

    #[test]
    fn test_smtp_code_452_quota_exceeded_temp() {
        let code = SmtpStatusCode::TemporaryFailure(452);
        assert_eq!(code.enhanced_code(), "4.2.2");
        assert!(!code.is_permanent());
    }

    #[test]
    fn test_smtp_code_554_policy_rejection() {
        let code = SmtpStatusCode::PermanentFailure(554);
        assert_eq!(code.enhanced_code(), "5.7.1");
        assert!(code.is_permanent());
    }

    #[tokio::test]
    async fn test_bounce_quota_exceeded() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(552);
        let bounce = mailet.generate_bounce(&mail, error_code, "Quota exceeded");

        assert!(bounce.contains("552"));
        assert!(bounce.contains("5.2.2"));
        assert!(bounce.contains("Quota exceeded"));
    }

    #[tokio::test]
    async fn test_bounce_message_too_large() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(552);
        let bounce = mailet.generate_bounce(&mail, error_code, "Message too large");

        assert!(bounce.contains("Message too large"));
        assert!(bounce.contains("5.2.2"));
    }

    #[tokio::test]
    async fn test_bounce_spam_rejection() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(554);
        let bounce = mailet.generate_bounce(&mail, error_code, "Spam detected");

        assert!(bounce.contains("Spam detected"));
        assert!(bounce.contains("5.7.1"));
    }

    #[tokio::test]
    async fn test_bounce_virus_rejection() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(554);
        let bounce = mailet.generate_bounce(&mail, error_code, "Virus detected");

        assert!(bounce.contains("Virus detected"));
        assert!(bounce.contains("5.7.1"));
    }

    #[tokio::test]
    async fn test_bounce_relay_denied() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(554);
        let bounce = mailet.generate_bounce(&mail, error_code, "Relay access denied");

        assert!(bounce.contains("Relay access denied"));
    }

    #[tokio::test]
    async fn test_bounce_network_unreachable() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::TemporaryFailure(421);
        let bounce = mailet.generate_bounce(&mail, error_code, "Network unreachable");

        assert!(bounce.contains("Network unreachable"));
        assert!(bounce.contains("4.4.2"));
    }

    #[tokio::test]
    async fn test_bounce_invalid_address() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(553);
        let bounce = mailet.generate_bounce(&mail, error_code, "Invalid address");

        assert!(bounce.contains("Invalid address"));
        assert!(bounce.contains("5.1.3"));
    }

    #[tokio::test]
    async fn test_bounce_mailbox_moved() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(551);
        let bounce = mailet.generate_bounce(&mail, error_code, "Mailbox moved");

        assert!(bounce.contains("Mailbox moved"));
        assert!(bounce.contains("5.1.6"));
    }

    #[test]
    fn test_dsn_recipient_all_fields() {
        let recipient = DsnRecipient {
            final_recipient: "user@test.com".to_string(),
            action: DsnAction::Failed,
            status: "5.1.1".to_string(),
            diagnostic_code: Some("550 User unknown".to_string()),
            remote_mta: Some("mx.test.com".to_string()),
            last_attempt_date: Some(1609459200),
        };

        assert_eq!(recipient.final_recipient, "user@test.com");
        assert_eq!(recipient.action, DsnAction::Failed);
        assert_eq!(recipient.status, "5.1.1");
        assert_eq!(
            recipient.diagnostic_code,
            Some("550 User unknown".to_string())
        );
        assert_eq!(recipient.remote_mta, Some("mx.test.com".to_string()));
        assert_eq!(recipient.last_attempt_date, Some(1609459200));
    }

    #[test]
    fn test_dsn_generation_with_all_recipients() {
        let mut dsn = DeliveryStatusNotification::new("mail.example.com".to_string());

        for i in 1..=3 {
            dsn.add_recipient(DsnRecipient {
                final_recipient: format!("user{}@test.com", i),
                action: DsnAction::Failed,
                status: "5.1.1".to_string(),
                diagnostic_code: Some("550 User unknown".to_string()),
                remote_mta: None,
                last_attempt_date: None,
            });
        }

        let body = dsn.generate_message_body();
        assert!(body.contains("user1@test.com"));
        assert!(body.contains("user2@test.com"));
        assert!(body.contains("user3@test.com"));
    }

    #[tokio::test]
    async fn test_bounce_all_headers_present() {
        let mailet = BounceMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        mail.set_attribute("header.Subject", "Test Subject");
        mail.set_attribute("header.From", "sender@test.com");
        mail.set_attribute("header.To", "recipient@test.com");
        mail.set_attribute("header.Date", "Mon, 1 Jan 2024 00:00:00 +0000");
        mail.set_attribute("header.Cc", "cc@test.com");
        mail.set_attribute("header.Message-ID", "<test@example.com>");

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Subject: Test Subject"));
        assert!(bounce.contains("From: sender@test.com"));
        assert!(bounce.contains("To: recipient@test.com"));
        assert!(bounce.contains("Date: Mon, 1 Jan 2024 00:00:00 +0000"));
        assert!(bounce.contains("Cc: cc@test.com"));
        assert!(bounce.contains("Message-ID: <test@example.com>"));
    }

    #[tokio::test]
    async fn test_bounce_rfc3464_multipart_structure() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        // RFC 3464 requires specific structure
        assert!(bounce.contains("MIME-Version: 1.0"));
        assert!(bounce.contains("multipart/report"));
        assert!(bounce.contains("report-type=delivery-status"));
        assert!(bounce.contains("boundary="));

        // Part 1: Human-readable
        assert!(bounce.contains("Content-Type: text/plain"));
        assert!(bounce.contains("Content-Description: Notification"));

        // Part 2: Machine-readable
        assert!(bounce.contains("Content-Type: message/delivery-status"));
        assert!(bounce.contains("Content-Description: Delivery Report"));

        // Part 3: Original headers
        assert!(bounce.contains("Content-Type: text/rfc822-headers"));
        assert!(bounce.contains("Content-Description: Undelivered Message Headers"));
    }

    #[test]
    fn test_smtp_status_code_edge_cases() {
        // Test boundary values
        assert_eq!(
            SmtpStatusCode::from_code(400),
            Some(SmtpStatusCode::TemporaryFailure(400))
        );
        assert_eq!(
            SmtpStatusCode::from_code(499),
            Some(SmtpStatusCode::TemporaryFailure(499))
        );
        assert_eq!(
            SmtpStatusCode::from_code(500),
            Some(SmtpStatusCode::PermanentFailure(500))
        );
        assert_eq!(
            SmtpStatusCode::from_code(599),
            Some(SmtpStatusCode::PermanentFailure(599))
        );

        // Invalid codes
        assert_eq!(SmtpStatusCode::from_code(200), None);
        assert_eq!(SmtpStatusCode::from_code(300), None);
        assert_eq!(SmtpStatusCode::from_code(600), None);
    }

    #[tokio::test]
    async fn test_bounce_default_values() {
        let mailet = BounceMailet::default();
        assert_eq!(mailet.reporting_mta, "localhost");
        assert_eq!(mailet.postmaster, "postmaster@localhost");
        assert!(mailet.include_headers);
        assert!(!mailet.include_body);
        assert_eq!(mailet.max_body_size, 1024);
    }

    #[tokio::test]
    async fn test_bounce_empty_body() {
        let mut mailet = BounceMailet::new();
        mailet.include_body = true;

        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from(""))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        // Should not crash with empty body
        assert!(bounce.contains("User unknown"));
    }

    #[test]
    fn test_dsn_action_debug() {
        let action = DsnAction::Failed;
        let debug_str = format!("{:?}", action);
        assert!(debug_str.contains("Failed"));
    }

    #[test]
    fn test_smtp_status_code_debug() {
        let code = SmtpStatusCode::PermanentFailure(550);
        let debug_str = format!("{:?}", code);
        assert!(debug_str.contains("550"));
    }

    #[tokio::test]
    async fn test_dsn_action_relayed() {
        assert_eq!(DsnAction::Relayed.as_str(), "relayed");
    }

    #[tokio::test]
    async fn test_dsn_action_expanded() {
        assert_eq!(DsnAction::Expanded.as_str(), "expanded");
    }

    #[tokio::test]
    async fn test_dsn_with_cc_header() {
        let mailet = BounceMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Cc", "cc@test.com");

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Cc: cc@test.com"));
    }

    #[tokio::test]
    async fn test_dsn_with_message_id_header() {
        let mailet = BounceMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Message-ID", "<12345@test.com>");

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Message-ID: <12345@test.com>"));
    }

    #[test]
    fn test_format_date_rfc2822() {
        let dsn = DeliveryStatusNotification::new("mail.example.com".to_string());
        let timestamp = 1609459200_u64; // 2021-01-01 00:00:00 UTC
        let formatted = dsn.format_date(timestamp);

        // Should be RFC 2822 format
        assert!(formatted.contains("2021") || formatted.contains("timestamp:"));
    }

    #[test]
    fn test_format_date_invalid() {
        let dsn = DeliveryStatusNotification::new("mail.example.com".to_string());
        let timestamp = i64::MAX as u64; // timestamp that may fail conversion
        let formatted = dsn.format_date(timestamp);

        // Should either format correctly or fall back to simple format
        assert!(!formatted.is_empty());
    }

    #[tokio::test]
    async fn test_bounce_boundary_string() {
        let mailet = BounceMailet::new();
        let mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        // Check for proper boundary markers
        assert!(bounce.contains("boundary=\"rusmes-dsn-boundary"));
        assert!(bounce.contains("--rusmes-dsn-boundary"));
    }

    #[test]
    fn test_smtp_status_code_code_method() {
        let temp = SmtpStatusCode::TemporaryFailure(450);
        assert_eq!(temp.code(), 450);

        let perm = SmtpStatusCode::PermanentFailure(550);
        assert_eq!(perm.code(), 550);
    }

    #[test]
    fn test_dsn_recipient_clone() {
        let recipient = DsnRecipient {
            final_recipient: "user@test.com".to_string(),
            action: DsnAction::Failed,
            status: "5.1.1".to_string(),
            diagnostic_code: Some("550 User unknown".to_string()),
            remote_mta: None,
            last_attempt_date: None,
        };

        let cloned = recipient.clone();
        assert_eq!(cloned.final_recipient, recipient.final_recipient);
        assert_eq!(cloned.action, recipient.action);
    }

    #[tokio::test]
    async fn test_bounce_multiple_headers() {
        let mailet = BounceMailet::new();
        let mut mail = Mail::new(
            Some(MailAddress::from_str("sender@test.com").unwrap()),
            vec![MailAddress::from_str("user@test.com").unwrap()],
            MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test"))),
            None,
            None,
        );
        mail.set_attribute("header.Subject", "Important Message");
        mail.set_attribute("header.From", "sender@test.com");
        mail.set_attribute("header.To", "user@test.com");
        mail.set_attribute("header.Date", "Mon, 1 Jan 2024 00:00:00 +0000");

        let error_code = SmtpStatusCode::PermanentFailure(550);
        let bounce = mailet.generate_bounce(&mail, error_code, "User unknown");

        assert!(bounce.contains("Subject: Important Message"));
        assert!(bounce.contains("From: sender@test.com"));
        assert!(bounce.contains("To: user@test.com"));
        assert!(bounce.contains("Date: Mon, 1 Jan 2024 00:00:00 +0000"));
    }
}
