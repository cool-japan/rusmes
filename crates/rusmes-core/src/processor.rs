//! Processor chain for mailet execution

use crate::mailet::Mailet;
use crate::matcher::Matcher;
use rusmes_proto::{Mail, MailState};
use std::sync::Arc;

/// Matcher-Mailet pair
pub struct ProcessingStep {
    pub matcher: Arc<dyn Matcher>,
    pub mailet: Arc<dyn Mailet>,
}

impl ProcessingStep {
    /// Create a new processing step
    pub fn new(matcher: Arc<dyn Matcher>, mailet: Arc<dyn Mailet>) -> Self {
        Self { matcher, mailet }
    }
}

/// Named processor chain for a specific state
pub struct Processor {
    name: String,
    state: MailState,
    steps: Vec<ProcessingStep>,
    thread_pool_size: usize,
}

impl Processor {
    /// Create a new processor
    pub fn new(name: impl Into<String>, state: MailState) -> Self {
        Self {
            name: name.into(),
            state,
            steps: Vec::new(),
            thread_pool_size: 4,
        }
    }

    /// Add a processing step
    pub fn add_step(&mut self, step: ProcessingStep) {
        self.steps.push(step);
    }

    /// Set thread pool size
    pub fn set_thread_pool_size(&mut self, size: usize) {
        self.thread_pool_size = size;
    }

    /// Get processor name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get processor state
    pub fn state(&self) -> &MailState {
        &self.state
    }

    /// Process a mail through the chain
    pub async fn process(&self, mut mail: Mail) -> anyhow::Result<Mail> {
        tracing::debug!(
            "Processing mail {} in processor {} (state: {})",
            mail.id(),
            self.name,
            self.state
        );

        for step in &self.steps {
            // Get matching recipients
            let matched = step.matcher.match_mail(&mail).await?;

            if matched.is_empty() {
                tracing::trace!(
                    "Matcher {} matched no recipients, skipping mailet {}",
                    step.matcher.name(),
                    step.mailet.name()
                );
                continue; // No match, skip this mailet
            }

            // Fork mail if partial match
            if matched.len() < mail.recipients().len() {
                tracing::trace!(
                    "Matcher {} partially matched {}/{} recipients",
                    step.matcher.name(),
                    matched.len(),
                    mail.recipients().len()
                );

                let (mut matched_mail, unmatched_mail) = mail.split(matched);

                // Process matched portion
                let action = step.mailet.service(&mut matched_mail).await?;
                tracing::debug!(
                    "Mailet {} returned action: {:?}",
                    step.mailet.name(),
                    action
                );

                // Continue with unmatched
                mail = unmatched_mail;

                // Handle state changes from matched portion
                if matched_mail.state != self.state {
                    // Would need to re-route matched_mail here in a real implementation
                    tracing::debug!(
                        "Matched mail state changed from {:?} to {:?}",
                        self.state,
                        matched_mail.state
                    );
                }
            } else {
                // All recipients match
                tracing::trace!(
                    "Matcher {} matched all {} recipients",
                    step.matcher.name(),
                    mail.recipients().len()
                );

                let action = step.mailet.service(&mut mail).await?;
                tracing::debug!(
                    "Mailet {} returned action: {:?}",
                    step.mailet.name(),
                    action
                );
            }

            // Check if state changed
            if mail.state != self.state {
                tracing::debug!(
                    "Mail state changed from {:?} to {:?}, exiting processor",
                    self.state,
                    mail.state
                );
                return Ok(mail); // Forward to different processor
            }
        }

        tracing::debug!(
            "Mail {} completed processing in processor {}",
            mail.id(),
            self.name
        );
        Ok(mail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailet::{MailetAction, MailetConfig};
    use crate::matcher::AllMatcher;
    use async_trait::async_trait;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

    struct TestMailet {
        name: String,
    }

    #[async_trait]
    impl Mailet for TestMailet {
        async fn init(&mut self, _config: MailetConfig) -> anyhow::Result<()> {
            Ok(())
        }

        async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
            mail.set_attribute("processed_by", self.name.clone());
            Ok(MailetAction::Continue)
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_processor_chain() {
        let mut processor = Processor::new("test", MailState::Root);

        let mailet1 = Arc::new(TestMailet {
            name: "mailet1".to_string(),
        });
        let mailet2 = Arc::new(TestMailet {
            name: "mailet2".to_string(),
        });

        processor.add_step(ProcessingStep::new(Arc::new(AllMatcher), mailet1));
        processor.add_step(ProcessingStep::new(Arc::new(AllMatcher), mailet2));

        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));
        let mail = Mail::new(
            None,
            vec!["user@example.com".parse().unwrap()],
            message,
            None,
            None,
        );

        let result = processor.process(mail).await.unwrap();
        assert!(result.get_attribute("processed_by").is_some());
    }
}
