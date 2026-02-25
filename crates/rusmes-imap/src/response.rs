//! IMAP response types

/// IMAP response
#[derive(Debug, Clone)]
pub struct ImapResponse {
    tag: Option<String>,
    status: String,
    message: String,
}

impl ImapResponse {
    /// Create a new response
    pub fn new(tag: Option<String>, status: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tag,
            status: status.into(),
            message: message.into(),
        }
    }

    /// OK response
    pub fn ok(tag: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Some(tag.into()), "OK", message)
    }

    /// BAD response
    pub fn bad(tag: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Some(tag.into()), "BAD", message)
    }

    /// NO response
    pub fn no(tag: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Some(tag.into()), "NO", message)
    }

    /// Format for transmission
    pub fn format(&self) -> String {
        if let Some(ref tag) = self.tag {
            format!("{} {} {}\r\n", tag, self.status, self.message)
        } else {
            format!("* {} {}\r\n", self.status, self.message)
        }
    }
}
