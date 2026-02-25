//! POP3 response builder

use std::fmt;

/// POP3 response status
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pop3Status {
    /// Positive response (+OK)
    Ok,
    /// Negative response (-ERR)
    Err,
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
}
