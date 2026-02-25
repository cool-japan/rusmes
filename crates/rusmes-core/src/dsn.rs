//! DSN (Delivery Status Notification) utilities - RFC 3464 and RFC 3463
//!
//! This module provides utilities for generating RFC-compliant DSN messages
//! and handling enhanced status codes.

use std::fmt;

/// Enhanced Status Code (RFC 3463)
///
/// Format: X.Y.Z where:
/// - X: Class (2=Success, 4=Transient Failure, 5=Permanent Failure)
/// - Y: Subject (0=Other, 1=Addressing, 2=Mailbox, 3=Mail System, 4=Network, 5=Protocol, 6=Content, 7=Security)
/// - Z: Detail (specific detail code)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnhancedStatusCode {
    pub class: u8,
    pub subject: u8,
    pub detail: u8,
}

impl EnhancedStatusCode {
    /// Create a new enhanced status code
    pub const fn new(class: u8, subject: u8, detail: u8) -> Self {
        Self {
            class,
            subject,
            detail,
        }
    }

    /// Parse enhanced status code from string (e.g., "5.1.1")
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return None;
        }

        let class = parts[0].parse().ok()?;
        let subject = parts[1].parse().ok()?;
        let detail = parts[2].parse().ok()?;

        Some(Self::new(class, subject, detail))
    }

    /// Check if this is a success code (2.X.X)
    pub fn is_success(&self) -> bool {
        self.class == 2
    }

    /// Check if this is a transient failure (4.X.X)
    pub fn is_transient(&self) -> bool {
        self.class == 4
    }

    /// Check if this is a permanent failure (5.X.X)
    pub fn is_permanent(&self) -> bool {
        self.class == 5
    }
}

impl fmt::Display for EnhancedStatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.class, self.subject, self.detail)
    }
}

/// Common enhanced status codes (RFC 3463)
#[allow(dead_code)]
impl EnhancedStatusCode {
    // Success codes (2.X.X)
    pub const SUCCESS: Self = Self::new(2, 0, 0);

    // Addressing status (X.1.X)
    pub const BAD_DESTINATION_MAILBOX: Self = Self::new(5, 1, 1); // 5.1.1 - User unknown
    pub const BAD_DESTINATION_SYSTEM: Self = Self::new(5, 1, 2); // 5.1.2 - System not accepting mail
    pub const BAD_DESTINATION_SYNTAX: Self = Self::new(5, 1, 3); // 5.1.3 - Invalid address syntax
    pub const DESTINATION_AMBIGUOUS: Self = Self::new(5, 1, 4); // 5.1.4 - Ambiguous address
    pub const DESTINATION_VALID: Self = Self::new(2, 1, 5); // 2.1.5 - Destination valid
    pub const MAILBOX_MOVED: Self = Self::new(5, 1, 6); // 5.1.6 - Mailbox moved
    pub const BAD_SENDER_ADDRESS: Self = Self::new(5, 1, 7); // 5.1.7 - Bad sender address
    pub const BAD_SENDER_SYSTEM: Self = Self::new(5, 1, 8); // 5.1.8 - Bad sender system

    // Mailbox status (X.2.X)
    pub const MAILBOX_DISABLED: Self = Self::new(5, 2, 1); // 5.2.1 - Mailbox disabled
    pub const MAILBOX_FULL: Self = Self::new(5, 2, 2); // 5.2.2 - Mailbox full
    pub const MAILBOX_FULL_TEMP: Self = Self::new(4, 2, 2); // 4.2.2 - Mailbox full (temporary)
    pub const MESSAGE_TOO_LARGE: Self = Self::new(5, 2, 3); // 5.2.3 - Message too large
    pub const MAILING_LIST_EXPANSION: Self = Self::new(5, 2, 4); // 5.2.4 - Mailing list expansion problem

    // Mail system status (X.3.X)
    pub const SYSTEM_FULL: Self = Self::new(4, 3, 1); // 4.3.1 - System full
    pub const SYSTEM_NOT_ACCEPTING: Self = Self::new(4, 3, 2); // 4.3.2 - System not accepting messages
    pub const SYSTEM_CAPABILITY: Self = Self::new(5, 3, 3); // 5.3.3 - System capability not supported
    pub const MESSAGE_TOO_BIG: Self = Self::new(5, 3, 4); // 5.3.4 - Message too big for system
    pub const SYSTEM_INCORRECTLY_CONFIGURED: Self = Self::new(5, 3, 5); // 5.3.5 - System incorrectly configured

    // Network and routing status (X.4.X)
    pub const NO_ANSWER: Self = Self::new(4, 4, 1); // 4.4.1 - No answer from host
    pub const CONNECTION_DROPPED: Self = Self::new(4, 4, 2); // 4.4.2 - Connection dropped
    pub const ROUTING_SERVER_FAILURE: Self = Self::new(4, 4, 3); // 4.4.3 - Routing server failure
    pub const NETWORK_CONGESTION: Self = Self::new(4, 4, 5); // 4.4.5 - Network congestion
    pub const ROUTING_LOOP: Self = Self::new(5, 4, 6); // 5.4.6 - Routing loop detected
    pub const DELIVERY_TIME_EXPIRED: Self = Self::new(4, 4, 7); // 4.4.7 - Delivery time expired

    // Mail delivery protocol status (X.5.X)
    pub const INVALID_COMMAND: Self = Self::new(5, 5, 1); // 5.5.1 - Invalid command
    pub const SYNTAX_ERROR: Self = Self::new(5, 5, 2); // 5.5.2 - Syntax error
    pub const TOO_MANY_RECIPIENTS: Self = Self::new(5, 5, 3); // 5.5.3 - Too many recipients
    pub const INVALID_PARAMETERS: Self = Self::new(5, 5, 4); // 5.5.4 - Invalid command arguments
    pub const WRONG_PROTOCOL: Self = Self::new(5, 5, 5); // 5.5.5 - Wrong protocol version

    // Content/media status (X.6.X)
    pub const MEDIA_NOT_SUPPORTED: Self = Self::new(5, 6, 1); // 5.6.1 - Media not supported
    pub const CONVERSION_REQUIRED: Self = Self::new(5, 6, 2); // 5.6.2 - Conversion required
    pub const CONVERSION_NOT_POSSIBLE: Self = Self::new(5, 6, 3); // 5.6.3 - Conversion not possible
    pub const CONVERSION_LOST: Self = Self::new(5, 6, 4); // 5.6.4 - Conversion with loss
    pub const CONVERSION_FAILED: Self = Self::new(5, 6, 5); // 5.6.5 - Conversion failed

    // Security/policy status (X.7.X)
    pub const DELIVERY_NOT_AUTHORIZED: Self = Self::new(5, 7, 1); // 5.7.1 - Delivery not authorized
    pub const MAILING_LIST_EXPANSION_PROHIBITED: Self = Self::new(5, 7, 2); // 5.7.2 - Mailing list expansion prohibited
    pub const SECURITY_CONVERSION_REQUIRED: Self = Self::new(5, 7, 3); // 5.7.3 - Security conversion required
    pub const SECURITY_FEATURES_NOT_SUPPORTED: Self = Self::new(5, 7, 4); // 5.7.4 - Security features not supported
    pub const CRYPTOGRAPHIC_FAILURE: Self = Self::new(5, 7, 5); // 5.7.5 - Cryptographic failure
    pub const CRYPTOGRAPHIC_ALGORITHM_NOT_SUPPORTED: Self = Self::new(5, 7, 6); // 5.7.6 - Cryptographic algorithm not supported
    pub const MESSAGE_INTEGRITY_FAILURE: Self = Self::new(5, 7, 7); // 5.7.7 - Message integrity failure
    pub const AUTHENTICATION_CREDENTIALS_INVALID: Self = Self::new(5, 7, 8); // 5.7.8 - Authentication credentials invalid
    pub const AUTHENTICATION_MECHANISM_TOO_WEAK: Self = Self::new(5, 7, 9); // 5.7.9 - Authentication mechanism too weak
    pub const ENCRYPTION_NEEDED: Self = Self::new(5, 7, 11); // 5.7.11 - Encryption needed
    pub const SENDER_ADDRESS_INVALID: Self = Self::new(5, 7, 12); // 5.7.12 - Sender address has null MX
    pub const MESSAGE_REFUSED: Self = Self::new(5, 7, 13); // 5.7.13 - Message refused
    pub const TRUST_RELATIONSHIP_REQUIRED: Self = Self::new(5, 7, 14); // 5.7.14 - Trust relationship required
    pub const PRIORITY_TOO_LOW: Self = Self::new(5, 7, 15); // 5.7.15 - Priority too low
    pub const MESSAGE_TOO_BIG_FOR_POLICY: Self = Self::new(5, 7, 17); // 5.7.17 - Message too big
    pub const MAILBOX_OWNER_CHANGED: Self = Self::new(5, 7, 18); // 5.7.18 - Mailbox owner has changed
    pub const RRVS_CANNOT_VALIDATE: Self = Self::new(5, 7, 19); // 5.7.19 - RRVS cannot validate
}

/// Delivery failure reasons
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// User unknown / Mailbox does not exist
    UserUnknown,
    /// Quota exceeded
    QuotaExceeded,
    /// Message too large
    MessageTooLarge,
    /// Content rejected (spam, virus)
    ContentRejected,
    /// Relay denied
    RelayDenied,
    /// Temporary failure (try again later)
    TemporaryFailure,
    /// Network unreachable
    NetworkUnreachable,
    /// Connection timeout
    ConnectionTimeout,
    /// Invalid address
    InvalidAddress,
    /// Mailbox disabled
    MailboxDisabled,
    /// System not accepting mail
    SystemNotAccepting,
    /// Authentication required
    AuthenticationRequired,
    /// Spam detected
    SpamDetected,
    /// Virus detected
    VirusDetected,
    /// Other/unknown error
    Other,
}

impl FailureReason {
    /// Get the enhanced status code for this failure reason
    pub fn enhanced_code(&self) -> EnhancedStatusCode {
        match self {
            Self::UserUnknown => EnhancedStatusCode::BAD_DESTINATION_MAILBOX,
            Self::QuotaExceeded => EnhancedStatusCode::MAILBOX_FULL,
            Self::MessageTooLarge => EnhancedStatusCode::MESSAGE_TOO_LARGE,
            Self::ContentRejected => EnhancedStatusCode::MESSAGE_REFUSED,
            Self::RelayDenied => EnhancedStatusCode::DELIVERY_NOT_AUTHORIZED,
            Self::TemporaryFailure => EnhancedStatusCode::new(4, 0, 0),
            Self::NetworkUnreachable => EnhancedStatusCode::NO_ANSWER,
            Self::ConnectionTimeout => EnhancedStatusCode::CONNECTION_DROPPED,
            Self::InvalidAddress => EnhancedStatusCode::BAD_DESTINATION_SYNTAX,
            Self::MailboxDisabled => EnhancedStatusCode::MAILBOX_DISABLED,
            Self::SystemNotAccepting => EnhancedStatusCode::SYSTEM_NOT_ACCEPTING,
            Self::AuthenticationRequired => EnhancedStatusCode::DELIVERY_NOT_AUTHORIZED,
            Self::SpamDetected => EnhancedStatusCode::MESSAGE_REFUSED,
            Self::VirusDetected => EnhancedStatusCode::MESSAGE_REFUSED,
            Self::Other => EnhancedStatusCode::new(5, 0, 0),
        }
    }

    /// Get a human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            Self::UserUnknown => "The recipient's email address does not exist",
            Self::QuotaExceeded => "The recipient's mailbox is full",
            Self::MessageTooLarge => "The message is too large to be delivered",
            Self::ContentRejected => "The message content was rejected by policy",
            Self::RelayDenied => "Relay access denied",
            Self::TemporaryFailure => "Temporary failure, will retry delivery",
            Self::NetworkUnreachable => "The destination mail server could not be reached",
            Self::ConnectionTimeout => "Connection to the mail server timed out",
            Self::InvalidAddress => "The recipient's email address is invalid",
            Self::MailboxDisabled => "The recipient's mailbox is disabled",
            Self::SystemNotAccepting => "The mail system is not accepting messages",
            Self::AuthenticationRequired => "Authentication is required for this delivery",
            Self::SpamDetected => "The message was identified as spam",
            Self::VirusDetected => "The message contains a virus",
            Self::Other => "An unknown error occurred",
        }
    }

    /// Check if this is a permanent failure
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::UserUnknown
                | Self::QuotaExceeded
                | Self::MessageTooLarge
                | Self::ContentRejected
                | Self::RelayDenied
                | Self::InvalidAddress
                | Self::MailboxDisabled
                | Self::SpamDetected
                | Self::VirusDetected
        )
    }

    /// Convert from SMTP status code
    pub fn from_smtp_code(code: u16) -> Self {
        match code {
            421 => Self::ConnectionTimeout,
            450 => Self::TemporaryFailure,
            451 => Self::TemporaryFailure,
            452 => Self::QuotaExceeded,
            550 => Self::UserUnknown,
            551 => Self::RelayDenied,
            552 => Self::QuotaExceeded,
            553 => Self::InvalidAddress,
            554 => Self::ContentRejected,
            _ if (400..500).contains(&code) => Self::TemporaryFailure,
            _ => Self::Other,
        }
    }
}

impl fmt::Display for FailureReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Map SMTP status code to enhanced status code
pub fn smtp_to_enhanced_code(smtp_code: u16) -> EnhancedStatusCode {
    match smtp_code {
        // 2xx - Success
        250 => EnhancedStatusCode::SUCCESS,
        251 => EnhancedStatusCode::DESTINATION_VALID,

        // 4xx - Transient failures
        421 => EnhancedStatusCode::CONNECTION_DROPPED,
        450 => EnhancedStatusCode::new(4, 2, 1), // Mailbox unavailable
        451 => EnhancedStatusCode::new(4, 3, 0), // Local error in processing
        452 => EnhancedStatusCode::MAILBOX_FULL_TEMP,
        454 => EnhancedStatusCode::new(4, 7, 0), // Temporary authentication failure

        // 5xx - Permanent failures
        500 => EnhancedStatusCode::SYNTAX_ERROR,
        501 => EnhancedStatusCode::INVALID_PARAMETERS,
        502 => EnhancedStatusCode::INVALID_COMMAND,
        503 => EnhancedStatusCode::INVALID_COMMAND,
        504 => EnhancedStatusCode::INVALID_PARAMETERS,
        550 => EnhancedStatusCode::BAD_DESTINATION_MAILBOX,
        551 => EnhancedStatusCode::MAILBOX_MOVED,
        552 => EnhancedStatusCode::MAILBOX_FULL,
        553 => EnhancedStatusCode::BAD_DESTINATION_SYNTAX,
        554 => EnhancedStatusCode::DELIVERY_NOT_AUTHORIZED,

        // Default cases
        _ if (200..300).contains(&smtp_code) => EnhancedStatusCode::SUCCESS,
        _ if (400..500).contains(&smtp_code) => EnhancedStatusCode::new(4, 0, 0),
        _ if (500..600).contains(&smtp_code) => EnhancedStatusCode::new(5, 0, 0),
        _ => EnhancedStatusCode::new(5, 0, 0),
    }
}

/// Get diagnostic text for SMTP code
pub fn smtp_diagnostic_text(smtp_code: u16) -> &'static str {
    match smtp_code {
        421 => "Service not available, closing transmission channel",
        450 => "Requested mail action not taken: mailbox unavailable",
        451 => "Requested action aborted: local error in processing",
        452 => "Requested action not taken: insufficient system storage",
        454 => "Temporary authentication failure",
        500 => "Syntax error, command unrecognized",
        501 => "Syntax error in parameters or arguments",
        502 => "Command not implemented",
        503 => "Bad sequence of commands",
        504 => "Command parameter not implemented",
        550 => "Requested action not taken: mailbox unavailable",
        551 => "User not local; please try forward path",
        552 => "Requested mail action aborted: exceeded storage allocation",
        553 => "Requested action not taken: mailbox name not allowed",
        554 => "Transaction failed",
        _ => "Unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_status_code_display() {
        let code = EnhancedStatusCode::new(5, 1, 1);
        assert_eq!(code.to_string(), "5.1.1");
    }

    #[test]
    fn test_enhanced_status_code_parse() {
        let code = EnhancedStatusCode::parse("5.1.1").unwrap();
        assert_eq!(code.class, 5);
        assert_eq!(code.subject, 1);
        assert_eq!(code.detail, 1);
    }

    #[test]
    fn test_enhanced_status_code_parse_invalid() {
        assert!(EnhancedStatusCode::parse("5.1").is_none());
        assert!(EnhancedStatusCode::parse("5.1.1.1").is_none());
        assert!(EnhancedStatusCode::parse("abc").is_none());
    }

    #[test]
    fn test_enhanced_status_code_is_success() {
        assert!(EnhancedStatusCode::new(2, 0, 0).is_success());
        assert!(!EnhancedStatusCode::new(4, 0, 0).is_success());
        assert!(!EnhancedStatusCode::new(5, 0, 0).is_success());
    }

    #[test]
    fn test_enhanced_status_code_is_transient() {
        assert!(!EnhancedStatusCode::new(2, 0, 0).is_transient());
        assert!(EnhancedStatusCode::new(4, 0, 0).is_transient());
        assert!(!EnhancedStatusCode::new(5, 0, 0).is_transient());
    }

    #[test]
    fn test_enhanced_status_code_is_permanent() {
        assert!(!EnhancedStatusCode::new(2, 0, 0).is_permanent());
        assert!(!EnhancedStatusCode::new(4, 0, 0).is_permanent());
        assert!(EnhancedStatusCode::new(5, 0, 0).is_permanent());
    }

    #[test]
    fn test_failure_reason_enhanced_code() {
        assert_eq!(
            FailureReason::UserUnknown.enhanced_code(),
            EnhancedStatusCode::BAD_DESTINATION_MAILBOX
        );
        assert_eq!(
            FailureReason::QuotaExceeded.enhanced_code(),
            EnhancedStatusCode::MAILBOX_FULL
        );
        assert_eq!(
            FailureReason::MessageTooLarge.enhanced_code(),
            EnhancedStatusCode::MESSAGE_TOO_LARGE
        );
    }

    #[test]
    fn test_failure_reason_is_permanent() {
        assert!(FailureReason::UserUnknown.is_permanent());
        assert!(!FailureReason::TemporaryFailure.is_permanent());
        assert!(FailureReason::InvalidAddress.is_permanent());
    }

    #[test]
    fn test_failure_reason_from_smtp_code() {
        assert_eq!(
            FailureReason::from_smtp_code(550),
            FailureReason::UserUnknown
        );
        assert_eq!(
            FailureReason::from_smtp_code(452),
            FailureReason::QuotaExceeded
        );
        assert_eq!(
            FailureReason::from_smtp_code(421),
            FailureReason::ConnectionTimeout
        );
    }

    #[test]
    fn test_smtp_to_enhanced_code_421() {
        assert_eq!(
            smtp_to_enhanced_code(421),
            EnhancedStatusCode::CONNECTION_DROPPED
        );
    }

    #[test]
    fn test_smtp_to_enhanced_code_450() {
        let code = smtp_to_enhanced_code(450);
        assert_eq!(code.class, 4);
        assert_eq!(code.subject, 2);
        assert_eq!(code.detail, 1);
    }

    #[test]
    fn test_smtp_to_enhanced_code_451() {
        let code = smtp_to_enhanced_code(451);
        assert_eq!(code.class, 4);
        assert_eq!(code.subject, 3);
        assert_eq!(code.detail, 0);
    }

    #[test]
    fn test_smtp_to_enhanced_code_452() {
        assert_eq!(
            smtp_to_enhanced_code(452),
            EnhancedStatusCode::MAILBOX_FULL_TEMP
        );
    }

    #[test]
    fn test_smtp_to_enhanced_code_550() {
        assert_eq!(
            smtp_to_enhanced_code(550),
            EnhancedStatusCode::BAD_DESTINATION_MAILBOX
        );
    }

    #[test]
    fn test_smtp_to_enhanced_code_551() {
        assert_eq!(
            smtp_to_enhanced_code(551),
            EnhancedStatusCode::MAILBOX_MOVED
        );
    }

    #[test]
    fn test_smtp_to_enhanced_code_552() {
        assert_eq!(smtp_to_enhanced_code(552), EnhancedStatusCode::MAILBOX_FULL);
    }

    #[test]
    fn test_smtp_to_enhanced_code_553() {
        assert_eq!(
            smtp_to_enhanced_code(553),
            EnhancedStatusCode::BAD_DESTINATION_SYNTAX
        );
    }

    #[test]
    fn test_smtp_to_enhanced_code_554() {
        assert_eq!(
            smtp_to_enhanced_code(554),
            EnhancedStatusCode::DELIVERY_NOT_AUTHORIZED
        );
    }

    #[test]
    fn test_smtp_diagnostic_text() {
        assert_eq!(
            smtp_diagnostic_text(421),
            "Service not available, closing transmission channel"
        );
        assert_eq!(
            smtp_diagnostic_text(550),
            "Requested action not taken: mailbox unavailable"
        );
    }

    #[test]
    fn test_smtp_diagnostic_text_unknown() {
        assert_eq!(smtp_diagnostic_text(999), "Unknown error");
    }

    #[test]
    fn test_enhanced_code_constants() {
        assert_eq!(EnhancedStatusCode::SUCCESS.to_string(), "2.0.0");
        assert_eq!(
            EnhancedStatusCode::BAD_DESTINATION_MAILBOX.to_string(),
            "5.1.1"
        );
        assert_eq!(EnhancedStatusCode::MAILBOX_FULL.to_string(), "5.2.2");
    }

    #[test]
    fn test_failure_reason_description() {
        assert!(!FailureReason::UserUnknown.description().is_empty());
        assert!(!FailureReason::QuotaExceeded.description().is_empty());
    }

    #[test]
    fn test_failure_reason_display() {
        let reason = FailureReason::UserUnknown;
        assert_eq!(reason.to_string(), reason.description());
    }

    #[test]
    fn test_smtp_to_enhanced_code_success_range() {
        let code = smtp_to_enhanced_code(250);
        assert_eq!(code, EnhancedStatusCode::SUCCESS);
    }

    #[test]
    fn test_smtp_to_enhanced_code_temp_failure_range() {
        let code = smtp_to_enhanced_code(499);
        assert!(code.is_transient());
    }

    #[test]
    fn test_smtp_to_enhanced_code_perm_failure_range() {
        let code = smtp_to_enhanced_code(599);
        assert!(code.is_permanent());
    }

    #[test]
    fn test_all_failure_reasons() {
        // Test all variants exist and work
        let reasons = [
            FailureReason::UserUnknown,
            FailureReason::QuotaExceeded,
            FailureReason::MessageTooLarge,
            FailureReason::ContentRejected,
            FailureReason::RelayDenied,
            FailureReason::TemporaryFailure,
            FailureReason::NetworkUnreachable,
            FailureReason::ConnectionTimeout,
            FailureReason::InvalidAddress,
            FailureReason::MailboxDisabled,
            FailureReason::SystemNotAccepting,
            FailureReason::AuthenticationRequired,
            FailureReason::SpamDetected,
            FailureReason::VirusDetected,
            FailureReason::Other,
        ];

        for reason in &reasons {
            let _ = reason.enhanced_code();
            let _ = reason.description();
            let _ = reason.is_permanent();
        }
    }
}
