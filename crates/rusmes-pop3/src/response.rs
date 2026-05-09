//! POP3 response builder

use std::fmt;

/// POP3 response status
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pop3Status {
    /// Positive response (+OK)
    Ok,
    /// Negative response (-ERR)
    Err,
    /// SASL continuation response (`+ <base64>`) per RFC 1734 / RFC 5034.
    ///
    /// Used during multi-step SASL exchanges such as LOGIN, CRAM-MD5, and
    /// SCRAM-SHA-256 to deliver server challenges to the client.
    Continue,
}

/// POP3 response
#[derive(Debug, Clone)]
pub struct Pop3Response {
    status: Pop3Status,
    message: String,
    multiline_data: Option<Vec<String>>,
}

impl Pop3Response {
    /// Create a positive response
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            status: Pop3Status::Ok,
            message: message.into(),
            multiline_data: None,
        }
    }

    /// Create a negative response
    pub fn err(message: impl Into<String>) -> Self {
        Self {
            status: Pop3Status::Err,
            message: message.into(),
            multiline_data: None,
        }
    }

    /// Create a positive response with multiline data
    pub fn ok_multiline(message: impl Into<String>, data: Vec<String>) -> Self {
        Self {
            status: Pop3Status::Ok,
            message: message.into(),
            multiline_data: Some(data),
        }
    }

    /// Create a SASL continuation response carrying a base64-encoded server
    /// challenge per RFC 1734 / RFC 5034 (`+ <base64>\r\n`).
    ///
    /// Pass an empty string to emit a bare `+ ` continuation (used by the
    /// PLAIN initial-response variant).
    pub fn cont(challenge_b64: impl Into<String>) -> Self {
        Self {
            status: Pop3Status::Continue,
            message: challenge_b64.into(),
            multiline_data: None,
        }
    }

    /// Get the status
    pub fn status(&self) -> Pop3Status {
        self.status
    }

    /// Get the message
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Get multiline data if present
    pub fn multiline_data(&self) -> Option<&[String]> {
        self.multiline_data.as_deref()
    }

    /// Convert to wire format (RFC 1939)
    pub fn to_wire(&self) -> String {
        let mut result = String::new();

        // Status line
        match self.status {
            Pop3Status::Ok => result.push_str("+OK "),
            Pop3Status::Err => result.push_str("-ERR "),
            Pop3Status::Continue => {
                // RFC 1734: `+ <base64>` (note: no `OK`, no `ERR`). When the
                // payload is empty, the spec still requires the trailing space
                // to be present (`+\r\n` is also accepted by some clients but
                // `+ \r\n` is the canonical form).
                if self.message.is_empty() {
                    result.push_str("+ \r\n");
                } else {
                    result.push_str("+ ");
                    result.push_str(&self.message);
                    result.push_str("\r\n");
                }
                return result;
            }
        }
        result.push_str(&self.message);
        result.push_str("\r\n");

        // Multiline data if present
        if let Some(ref data) = self.multiline_data {
            for line in data {
                // Byte-stuff lines beginning with "." (RFC 1939 section 3)
                if line.starts_with('.') {
                    result.push('.');
                }
                result.push_str(line);
                result.push_str("\r\n");
            }
            // Termination octet
            result.push_str(".\r\n");
        }

        result
    }
}

impl fmt::Display for Pop3Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_wire())
    }
}

impl Pop3Status {
    /// Check if this is a success status
    pub fn is_ok(&self) -> bool {
        matches!(self, Pop3Status::Ok)
    }

    /// Check if this is an error status
    pub fn is_err(&self) -> bool {
        matches!(self, Pop3Status::Err)
    }

    /// Check if this is a SASL continuation
    pub fn is_continue(&self) -> bool {
        matches!(self, Pop3Status::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ok_response_wire_format() {
        let r = Pop3Response::ok("hello");
        assert_eq!(r.to_wire(), "+OK hello\r\n");
    }

    #[test]
    fn test_err_response_wire_format() {
        let r = Pop3Response::err("nope");
        assert_eq!(r.to_wire(), "-ERR nope\r\n");
    }

    #[test]
    fn test_continuation_with_payload_wire_format() {
        let r = Pop3Response::cont("VXNlcm5hbWU6");
        assert_eq!(r.to_wire(), "+ VXNlcm5hbWU6\r\n");
        assert!(r.status().is_continue());
    }

    #[test]
    fn test_empty_continuation_wire_format() {
        let r = Pop3Response::cont("");
        assert_eq!(r.to_wire(), "+ \r\n");
    }

    #[test]
    fn test_multiline_response_wire_format() {
        let r = Pop3Response::ok_multiline("CAPA", vec!["USER".into(), "TOP".into()]);
        let wire = r.to_wire();
        assert!(wire.starts_with("+OK CAPA\r\n"));
        assert!(wire.contains("USER\r\n"));
        assert!(wire.contains("TOP\r\n"));
        assert!(wire.ends_with(".\r\n"));
    }

    #[test]
    fn test_multiline_byte_stuffing() {
        let r = Pop3Response::ok_multiline("data", vec![".leading-dot".into()]);
        let wire = r.to_wire();
        // The line beginning with `.` must be byte-stuffed with an extra leading `.`
        assert!(wire.contains("..leading-dot\r\n"));
    }
}
