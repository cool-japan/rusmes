//! Matcher trait for filtering mail recipients

use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};

/// Matcher trait - filter recipients based on criteria
#[async_trait]
pub trait Matcher: Send + Sync {
    /// Returns subset of recipients matching criteria
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>>;

    /// Matcher name for logging
    fn name(&self) -> &str;
}

/// Match all recipients
#[allow(dead_code)]
pub struct AllMatcher;

#[async_trait]
impl Matcher for AllMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        Ok(mail.recipients().to_vec())
    }

    fn name(&self) -> &str {
        "All"
    }
}

/// Match no recipients
#[allow(dead_code)]
pub struct NoneMatcher;

#[async_trait]
impl Matcher for NoneMatcher {
    async fn match_mail(&self, _mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        Ok(Vec::new())
    }

    fn name(&self) -> &str {
        "None"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

    #[tokio::test]
    async fn test_all_matcher() {
        let recipients = vec![
            "user1@example.com".parse().unwrap(),
            "user2@example.com".parse().unwrap(),
        ];
        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));
        let mail = Mail::new(None, recipients.clone(), message, None, None);

        let matcher = AllMatcher;
        let matched = matcher.match_mail(&mail).await.unwrap();
        assert_eq!(matched.len(), 2);
    }

    #[tokio::test]
    async fn test_none_matcher() {
        let recipients = vec!["user@example.com".parse().unwrap()];
        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));
        let mail = Mail::new(None, recipients, message, None, None);

        let matcher = NoneMatcher;
        let matched = matcher.match_mail(&mail).await.unwrap();
        assert_eq!(matched.len(), 0);
    }
}
