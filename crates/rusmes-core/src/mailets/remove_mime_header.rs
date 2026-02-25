//! Remove MIME header mailet for privacy and security

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::Mail;
use std::sync::Arc;

/// Removes specified MIME headers from messages
/// Useful for privacy (removing Bcc, X-* headers) before delivery
pub struct RemoveMimeHeaderMailet {
    name: String,
    /// List of header patterns to remove (exact matches or wildcards)
    header_patterns: Vec<String>,
    /// Whether to use case-insensitive matching
    case_insensitive: bool,
}

impl RemoveMimeHeaderMailet {
    /// Create a new remove MIME header mailet
    pub fn new() -> Self {
        Self {
            name: "RemoveMimeHeader".to_string(),
            header_patterns: Vec::new(),
            case_insensitive: true,
        }
    }

    /// Create a preset mailet for removing Bcc headers
    pub fn remove_bcc() -> Self {
        let mut mailet = Self::new();
        mailet.header_patterns.push("Bcc".to_string());
        mailet
    }

    /// Create a preset mailet for removing trace headers
    pub fn remove_trace() -> Self {
        let mut mailet = Self::new();
        mailet.header_patterns.extend([
            "X-*".to_string(),
            "Received".to_string(),
            "Return-Path".to_string(),
        ]);
        mailet
    }

    /// Check if a header name matches a pattern
    fn matches_pattern(&self, header_name: &str, pattern: &str) -> bool {
        let header_cmp = if self.case_insensitive {
            header_name.to_lowercase()
        } else {
            header_name.to_string()
        };

        let pattern_cmp = if self.case_insensitive {
            pattern.to_lowercase()
        } else {
            pattern.to_string()
        };

        // Exact match
        if header_cmp == pattern_cmp {
            return true;
        }

        // Wildcard matching (simple * support)
        if pattern_cmp.contains('*') {
            // Convert wildcard pattern to regex-like matching
            if pattern_cmp.ends_with('*') {
                let prefix = pattern_cmp.trim_end_matches('*');
                return header_cmp.starts_with(prefix);
            }
            if pattern_cmp.starts_with('*') {
                let suffix = pattern_cmp.trim_start_matches('*');
                return header_cmp.ends_with(suffix);
            }
            // Middle wildcard: split on * and check start/end
            if let Some(star_pos) = pattern_cmp.find('*') {
                let prefix = &pattern_cmp[..star_pos];
                let suffix = &pattern_cmp[star_pos + 1..];
                return header_cmp.starts_with(prefix) && header_cmp.ends_with(suffix);
            }
        }

        false
    }

    /// Check if a header should be removed
    fn should_remove(&self, header_name: &str) -> bool {
        for pattern in &self.header_patterns {
            if self.matches_pattern(header_name, pattern) {
                return true;
            }
        }
        false
    }
}

impl Default for RemoveMimeHeaderMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for RemoveMimeHeaderMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        // Parse headers parameter - comma-separated list
        if let Some(headers_str) = config.get_param("headers") {
            self.header_patterns = headers_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        // Parse case_insensitive parameter (default: true)
        if let Some(case_str) = config.get_param("case_insensitive") {
            self.case_insensitive = case_str.parse().unwrap_or(true);
        }

        // Parse preset parameter
        if let Some(preset) = config.get_param("preset") {
            match preset {
                "bcc" | "remove_bcc" => {
                    self.header_patterns.push("Bcc".to_string());
                }
                "trace" | "remove_trace" => {
                    self.header_patterns.extend([
                        "X-*".to_string(),
                        "Received".to_string(),
                        "Return-Path".to_string(),
                    ]);
                }
                _ => {
                    tracing::warn!("Unknown preset: {}", preset);
                }
            }
        }

        if self.header_patterns.is_empty() {
            return Err(anyhow::anyhow!(
                "RemoveMimeHeaderMailet requires 'headers' parameter or 'preset' parameter"
            ));
        }

        tracing::info!(
            "Initialized RemoveMimeHeaderMailet with patterns: {:?}",
            self.header_patterns
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        tracing::debug!("Processing mail {} with RemoveMimeHeaderMailet", mail.id());

        // Get current message
        let message = mail.message();
        let headers = message.headers();

        // Collect headers to remove
        let mut headers_to_remove = Vec::new();
        for (header_name, _) in headers.iter() {
            if self.should_remove(header_name) {
                headers_to_remove.push(header_name.clone());
            }
        }

        if headers_to_remove.is_empty() {
            tracing::debug!("No headers matched removal patterns for mail {}", mail.id());
            return Ok(MailetAction::Continue);
        }

        // Clone the message to modify it (Arc::make_mut equivalent)
        let mut new_message = (**message).clone();
        let new_headers = new_message.headers_mut();

        // Remove matched headers
        for header_name in &headers_to_remove {
            tracing::debug!("Removing header '{}' from mail {}", header_name, mail.id());
            new_headers.remove(header_name);
        }

        // Replace the message in mail
        mail.set_message(Arc::new(new_message));

        tracing::info!(
            "Removed {} header(s) from mail {}: {:?}",
            headers_to_remove.len(),
            mail.id(),
            headers_to_remove
        );

        Ok(MailetAction::Continue)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    #[allow(dead_code)]
    fn placeholder() {
        // Tests omitted per user request
    }
}
