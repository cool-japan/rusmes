//! Matcher for blacklisted sender addresses

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};

/// Matches messages from blacklisted sender addresses
pub struct IsInBlacklistMatcher {
    blacklist: Vec<String>,
}

impl IsInBlacklistMatcher {
    /// Create a new IsInBlacklist matcher
    pub fn new(blacklist: Vec<String>) -> Self {
        Self { blacklist }
    }
}

#[async_trait]
impl Matcher for IsInBlacklistMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        if let Some(sender) = mail.sender() {
            let sender_str = sender.as_string();

            for blocked in &self.blacklist {
                // Support exact match or domain match
                if sender_str == *blocked || sender_str.ends_with(&format!("@{}", blocked)) {
                    return Ok(mail.recipients().to_vec());
                }
            }
        }
        Ok(Vec::new())
    }

    fn name(&self) -> &str {
        "IsInBlacklist"
    }
}
