//! SMTP response types and codes

use std::fmt;

/// SMTP response with code and message
#[derive(Debug, Clone, PartialEq)]
pub struct SmtpResponse {
    code: u16,
    lines: Vec<String>,
}

impl SmtpResponse {
    /// Create a new SMTP response
    pub fn new(code: u16, message: impl Into<String>) -> Self {
        Self {
            code,
            lines: vec![message.into()],
        }
    }

    /// Create a multi-line response
    pub fn multiline(code: u16, lines: Vec<String>) -> Self {
        Self { code, lines }
    }

    /// Get response code
    pub fn code(&self) -> u16 {
        self.code
    }

    /// Check if response indicates success (2xx)
    pub fn is_success(&self) -> bool {
        self.code >= 200 && self.code < 300
    }

    /// Check if response is permanent error (5xx)
    pub fn is_permanent_error(&self) -> bool {
        self.code >= 500 && self.code < 600
    }

    /// Check if response is temporary error (4xx)
    pub fn is_temporary_error(&self) -> bool {
        self.code >= 400 && self.code < 500
    }

    /// Format response for transmission (with CRLF)
    pub fn format(&self) -> String {
        if self.lines.len() == 1 {
            format!("{} {}\r\n", self.code, self.lines[0])
        } else {
            let mut result = String::new();
            let last_idx = self.lines.len() - 1;
            for (idx, line) in self.lines.iter().enumerate() {
                if idx == last_idx {
                    result.push_str(&format!("{} {}\r\n", self.code, line));
                } else {
                    result.push_str(&format!("{}-{}\r\n", self.code, line));
                }
            }
            result
        }
    }

    // Standard responses

    /// 220 Service ready
    pub fn service_ready(domain: &str) -> Self {
        Self::new(220, format!("{} RusMES SMTP Server ready", domain))
    }

    /// 221 Service closing
    pub fn closing() -> Self {
        Self::new(221, "Bye")
    }

    /// 250 OK
    pub fn ok(message: impl Into<String>) -> Self {
        Self::new(250, message)
    }

    /// 250 OK (simple)
    pub fn ok_simple() -> Self {
        Self::new(250, "OK")
    }

    /// 250 EHLO response
    pub fn ehlo(domain: &str, extensions: Vec<String>) -> Self {
        let mut lines = vec![domain.to_string()];
        lines.extend(extensions);
        Self::multiline(250, lines)
    }

    /// 354 Start mail input
    pub fn start_data() -> Self {
        Self::new(354, "Start mail input; end with <CRLF>.<CRLF>")
    }

    /// 421 Service not available
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(421, message)
    }

    /// 450 Mailbox unavailable
    pub fn mailbox_unavailable(message: impl Into<String>) -> Self {
        Self::new(450, message)
    }

    /// 451 Local error
    pub fn local_error(message: impl Into<String>) -> Self {
        Self::new(451, message)
    }

    /// 452 Insufficient storage
    pub fn insufficient_storage() -> Self {
        Self::new(452, "Insufficient system storage")
    }

    /// 500 Syntax error
    pub fn syntax_error(message: impl Into<String>) -> Self {
        Self::new(500, message)
    }

    /// 501 Syntax error in parameters
    pub fn parameter_error(message: impl Into<String>) -> Self {
        Self::new(501, message)
    }

    /// 502 Command not implemented
    pub fn not_implemented(message: impl Into<String>) -> Self {
        Self::new(502, message)
    }

    /// 503 Bad sequence of commands
    pub fn bad_sequence(message: impl Into<String>) -> Self {
        Self::new(503, message)
    }

    /// 504 Parameter not implemented
    pub fn parameter_not_implemented(message: impl Into<String>) -> Self {
        Self::new(504, message)
    }

    /// 550 Mailbox unavailable (permanent)
    pub fn mailbox_not_found(message: impl Into<String>) -> Self {
        Self::new(550, message)
    }

    /// 551 User not local
    pub fn user_not_local(message: impl Into<String>) -> Self {
        Self::new(551, message)
    }

    /// 552 Storage allocation exceeded
    pub fn storage_exceeded(message: impl Into<String>) -> Self {
        Self::new(552, message)
    }

    /// 553 Mailbox name not allowed
    pub fn mailbox_name_invalid(message: impl Into<String>) -> Self {
        Self::new(553, message)
    }

    /// 554 Transaction failed
    pub fn transaction_failed(message: impl Into<String>) -> Self {
        Self::new(554, message)
    }

    /// 555 MAIL FROM/RCPT TO parameters not recognized
    pub fn parameters_not_recognized(message: impl Into<String>) -> Self {
        Self::new(555, message)
    }
}

impl fmt::Display for SmtpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format().trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_line_response() {
        let resp = SmtpResponse::new(250, "OK");
        assert_eq!(resp.format(), "250 OK\r\n");
        assert!(resp.is_success());
    }

    #[test]
    fn test_multiline_response() {
        let resp = SmtpResponse::multiline(
            250,
            vec![
                "mail.example.com".to_string(),
                "SIZE 10240000".to_string(),
                "STARTTLS".to_string(),
            ],
        );
        let formatted = resp.format();
        assert!(formatted.contains("250-mail.example.com\r\n"));
        assert!(formatted.contains("250-SIZE 10240000\r\n"));
        assert!(formatted.contains("250 STARTTLS\r\n"));
    }

    #[test]
    fn test_ehlo_response() {
        let resp = SmtpResponse::ehlo(
            "mail.example.com",
            vec!["SIZE 10240000".to_string(), "STARTTLS".to_string()],
        );
        assert_eq!(resp.code(), 250);
        let formatted = resp.format();
        assert!(formatted.contains("mail.example.com"));
        assert!(formatted.contains("SIZE"));
        assert!(formatted.contains("STARTTLS"));
    }

    #[test]
    fn test_error_types() {
        let temp_error = SmtpResponse::new(450, "Temporary error");
        assert!(temp_error.is_temporary_error());
        assert!(!temp_error.is_permanent_error());
        assert!(!temp_error.is_success());

        let perm_error = SmtpResponse::new(550, "Permanent error");
        assert!(perm_error.is_permanent_error());
        assert!(!perm_error.is_temporary_error());
        assert!(!perm_error.is_success());

        let success = SmtpResponse::new(250, "Success");
        assert!(success.is_success());
        assert!(!success.is_temporary_error());
        assert!(!success.is_permanent_error());
    }

    #[test]
    fn test_standard_responses() {
        assert_eq!(SmtpResponse::ok_simple().code(), 250);
        assert_eq!(SmtpResponse::closing().code(), 221);
        assert_eq!(SmtpResponse::start_data().code(), 354);
        assert_eq!(SmtpResponse::syntax_error("test").code(), 500);
        assert_eq!(SmtpResponse::mailbox_not_found("test").code(), 550);
    }
}
