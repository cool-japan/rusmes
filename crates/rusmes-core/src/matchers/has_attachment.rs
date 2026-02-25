//! Matcher for messages with attachments

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};

/// Matches messages that have attachments
pub struct HasAttachmentMatcher;

impl HasAttachmentMatcher {
    /// Create a new HasAttachment matcher
    pub fn new() -> Self {
        Self
    }
}

impl Default for HasAttachmentMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Matcher for HasAttachmentMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        // Check MIME headers for attachments
        let has_attachment = Self::detect_attachment(mail);

        if has_attachment {
            Ok(mail.recipients().to_vec())
        } else {
            Ok(Vec::new())
        }
    }

    fn name(&self) -> &str {
        "HasAttachment"
    }
}

impl HasAttachmentMatcher {
    /// Detect if a mail has attachments by checking MIME headers
    fn detect_attachment(mail: &Mail) -> bool {
        let headers = mail.message().headers();

        // Check Content-Type for multipart
        if let Some(content_type) = headers.get_first("Content-Type") {
            let content_type_lower = content_type.to_lowercase();

            // Multipart messages (except multipart/alternative which is just text+html)
            if content_type_lower.contains("multipart/mixed")
                || content_type_lower.contains("multipart/related")
            {
                return true;
            }
        }

        // Check Content-Disposition for attachment
        if let Some(disposition) = headers.get_first("Content-Disposition") {
            if disposition.to_lowercase().contains("attachment") {
                return true;
            }
        }

        // Check for common attachment indicators in headers
        // Files like image/*, application/*, etc. (but not text/plain or text/html)
        if let Some(content_type) = headers.get_first("Content-Type") {
            let content_type_lower = content_type.to_lowercase();
            if (content_type_lower.starts_with("image/")
                || content_type_lower.starts_with("application/")
                || content_type_lower.starts_with("audio/")
                || content_type_lower.starts_with("video/"))
                && !content_type_lower.contains("multipart/alternative")
            {
                // Additional check: must have filename parameter or be disposition: attachment
                if content_type_lower.contains("name=")
                    || headers
                        .get_first("Content-Disposition")
                        .map(|d| d.to_lowercase().contains("attachment"))
                        .unwrap_or(false)
                {
                    return true;
                }
            }
        }

        false
    }
}
