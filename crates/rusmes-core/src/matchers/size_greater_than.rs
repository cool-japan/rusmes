//! Matcher for messages exceeding size threshold

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};

/// Matches messages larger than a specified size threshold
pub struct SizeGreaterThanMatcher {
    threshold_bytes: usize,
}

impl SizeGreaterThanMatcher {
    /// Create a new SizeGreaterThan matcher
    pub fn new(threshold_bytes: usize) -> Self {
        Self { threshold_bytes }
    }
}

#[async_trait]
impl Matcher for SizeGreaterThanMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        if mail.size() > self.threshold_bytes {
            Ok(mail.recipients().to_vec())
        } else {
            Ok(Vec::new())
        }
    }

    fn name(&self) -> &str {
        "SizeGreaterThan"
    }
}
