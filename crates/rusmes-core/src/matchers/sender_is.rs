//! Matcher for specific sender addresses

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};

/// Matches messages from specific senders
pub struct SenderIsMatcher {
    senders: Vec<String>,
}

impl SenderIsMatcher {
    /// Create a new SenderIs matcher
    pub fn new(senders: Vec<String>) -> Self {
        Self { senders }
    }
}

#[async_trait]
impl Matcher for SenderIsMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        if let Some(sender) = mail.sender() {
            let sender_str = sender.as_string();
            if self.senders.iter().any(|s| sender_str.contains(s)) {
                return Ok(mail.recipients().to_vec());
            }
        }
        Ok(Vec::new())
    }

    fn name(&self) -> &str {
        "SenderIs"
    }
}
