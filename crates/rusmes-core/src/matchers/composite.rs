//! Composite matchers for combining other matchers

use crate::matcher::Matcher;
use async_trait::async_trait;
use rusmes_proto::{Mail, MailAddress};
use std::sync::Arc;

/// Matches when all child matchers match
pub struct AndMatcher {
    matchers: Vec<Arc<dyn Matcher>>,
}

impl AndMatcher {
    /// Create a new And matcher
    pub fn new(matchers: Vec<Arc<dyn Matcher>>) -> Self {
        Self { matchers }
    }
}

#[async_trait]
impl Matcher for AndMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        if self.matchers.is_empty() {
            return Ok(Vec::new());
        }

        // Start with all recipients
        let mut result: Vec<MailAddress> = mail.recipients().to_vec();

        // Intersect with each matcher's results
        for matcher in &self.matchers {
            let matched = matcher.match_mail(mail).await?;

            // Keep only recipients that are in both result and matched
            result.retain(|r| matched.contains(r));

            // Early exit if no recipients remain
            if result.is_empty() {
                break;
            }
        }

        Ok(result)
    }

    fn name(&self) -> &str {
        "And"
    }
}

/// Matches when any child matcher matches
pub struct OrMatcher {
    matchers: Vec<Arc<dyn Matcher>>,
}

impl OrMatcher {
    /// Create a new Or matcher
    pub fn new(matchers: Vec<Arc<dyn Matcher>>) -> Self {
        Self { matchers }
    }
}

#[async_trait]
impl Matcher for OrMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        let mut result: Vec<MailAddress> = Vec::new();

        // Union of all matched recipients
        for matcher in &self.matchers {
            let matched = matcher.match_mail(mail).await?;
            for recipient in matched {
                if !result.contains(&recipient) {
                    result.push(recipient);
                }
            }
        }

        Ok(result)
    }

    fn name(&self) -> &str {
        "Or"
    }
}

/// Matches when the child matcher does NOT match
pub struct NotMatcher {
    matcher: Arc<dyn Matcher>,
}

impl NotMatcher {
    /// Create a new Not matcher
    pub fn new(matcher: Arc<dyn Matcher>) -> Self {
        Self { matcher }
    }
}

#[async_trait]
impl Matcher for NotMatcher {
    async fn match_mail(&self, mail: &Mail) -> anyhow::Result<Vec<MailAddress>> {
        let matched = self.matcher.match_mail(mail).await?;

        // Return recipients NOT in the matched set
        let result: Vec<MailAddress> = mail
            .recipients()
            .iter()
            .filter(|r| !matched.contains(r))
            .cloned()
            .collect();

        Ok(result)
    }

    fn name(&self) -> &str {
        "Not"
    }
}
