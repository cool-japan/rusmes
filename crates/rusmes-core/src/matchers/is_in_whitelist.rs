//! Matcher for whitelisted sender addresses

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};

/// Matches messages from whitelisted sender addresses
pub struct IsInWhitelistMatcher {
    whitelist: Vec<String>,
}

impl IsInWhitelistMatcher {
    /// Create a new IsInWhitelist matcher
    pub fn new(whitelist: Vec<String>) -> Self {
        Self { whitelist }
    }
}

#[async_trait]
impl Matcher for IsInWhitelistMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        if let Some(sender) = mail.sender() {
            let sender_str = sender.as_string();

            for allowed in &self.whitelist {
                // Support exact match or domain match
                if sender_str == *allowed || sender_str.ends_with(&format!("@{}", allowed)) {
                    return Ok(mail.recipients().to_vec());
                }
            }
        }
        Ok(Vec::new())
    }

    fn name(&self) -> &str {
        "IsInWhitelist"
    }
}
