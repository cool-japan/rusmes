//! Matcher for local recipients

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};

/// Matches recipients in local domains
pub struct RecipientIsLocalMatcher {
    local_domains: Vec<String>,
}

impl RecipientIsLocalMatcher {
    /// Create a new RecipientIsLocal matcher
    pub fn new(local_domains: Vec<String>) -> Self {
        Self { local_domains }
    }
}

#[async_trait]
impl Matcher for RecipientIsLocalMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        let matched: Vec<MailAddress> = mail
            .recipients()
            .iter()
            .filter(|addr| {
                self.local_domains
                    .iter()
                    .any(|domain| addr.domain().as_str() == domain)
            })
            .cloned()
            .collect();

        Ok(matched)
    }

    fn name(&self) -> &str {
        "RecipientIsLocal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

    #[tokio::test]
    async fn test_recipient_is_local() {
        let matcher = RecipientIsLocalMatcher::new(vec!["example.com".to_string()]);

        let recipients = vec![
            "local@example.com".parse().unwrap(),
            "remote@other.com".parse().unwrap(),
        ];

        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));
        let mail = Mail::new(None, recipients, message, None, None);

        let matched = matcher.match_mail(&mail).await.unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].domain().as_str(), "example.com");
    }
}
