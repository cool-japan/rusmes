//! Processor chain for mailet execution

use crate::mailet::{Mailet, MailetAction, MailetConfig, MailetError, MailetErrorPolicy};
use crate::matcher::Matcher;
use rusmes_proto::{Mail, MailState};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Retry state for a single mailet step when the error policy is `Retry`
struct RetryState {
    attempts: u32,
    max: u32,
    backoff: Duration,
}

/// Invoke a mailet's `service` method, applying an optional per-mailet timeout.
/// Returns `Ok(action)` on success, or an error wrapping `MailetError`.
async fn invoke_with_timeout(
    mailet: &dyn Mailet,
    mail: &mut Mail,
    timeout_ms: Option<u64>,
) -> Result<MailetAction, MailetError> {
    match timeout_ms {
        None => mailet
            .service(mail)
            .await
            .map_err(MailetError::ServiceError),
        Some(ms) => {
            let duration = Duration::from_millis(ms);
            match timeout(duration, mailet.service(mail)).await {
                Ok(Ok(action)) => Ok(action),
                Ok(Err(e)) => Err(MailetError::ServiceError(e)),
                Err(_elapsed) => Err(MailetError::Timeout(duration)),
            }
        }
    }
}

/// Invoke with timeout, retrying according to the error policy.
///
/// Returns `Ok(Some(action))` on success, `Ok(None)` if the policy says to skip,
/// or `Err(MailetError)` if the policy says to abort.
async fn invoke_with_policy(
    mailet: &dyn Mailet,
    mail: &mut Mail,
    config: &MailetConfig,
) -> anyhow::Result<Option<MailetAction>> {
    match &config.error_policy {
        MailetErrorPolicy::Skip => {
            match invoke_with_timeout(mailet, mail, config.timeout_ms).await {
                Ok(action) => Ok(Some(action)),
                Err(e) => {
                    tracing::warn!(
                        "Mailet {} errored (policy=Skip), skipping: {}",
                        mailet.name(),
                        e
                    );
                    Ok(None)
                }
            }
        }
        MailetErrorPolicy::Abort => {
            let action = invoke_with_timeout(mailet, mail, config.timeout_ms)
                .await
                .map_err(|e| anyhow::anyhow!("Mailet {} aborted pipeline: {}", mailet.name(), e))?;
            Ok(Some(action))
        }
        MailetErrorPolicy::Retry { max, backoff } => {
            let mut state = RetryState {
                attempts: 0,
                max: *max,
                backoff: *backoff,
            };
            loop {
                match invoke_with_timeout(mailet, mail, config.timeout_ms).await {
                    Ok(action) => return Ok(Some(action)),
                    Err(e) => {
                        state.attempts += 1;
                        if state.attempts > state.max {
                            return Err(anyhow::anyhow!(
                                "Mailet {} failed after {} retries, aborting: {}",
                                mailet.name(),
                                state.max,
                                e
                            ));
                        }
                        tracing::warn!(
                            "Mailet {} error (attempt {}/{}), retrying in {:?}: {}",
                            mailet.name(),
                            state.attempts,
                            state.max,
                            state.backoff,
                            e
                        );
                        tokio::time::sleep(state.backoff).await;
                    }
                }
            }
        }
    }
}

/// Matcher-Mailet pair with optional per-step configuration
pub struct ProcessingStep {
    pub matcher: Arc<dyn Matcher>,
    pub mailet: Arc<dyn Mailet>,
    /// Per-step mailet configuration (timeout, error policy)
    pub config: MailetConfig,
}

impl ProcessingStep {
    /// Create a new processing step with a default config derived from the mailet name
    pub fn new(matcher: Arc<dyn Matcher>, mailet: Arc<dyn Mailet>) -> Self {
        let name = mailet.name().to_string();
        Self {
            matcher,
            mailet,
            config: MailetConfig::new(name),
        }
    }

    /// Create a new processing step with an explicit configuration
    pub fn new_with_config(
        matcher: Arc<dyn Matcher>,
        mailet: Arc<dyn Mailet>,
        config: MailetConfig,
    ) -> Self {
        Self {
            matcher,
            mailet,
            config,
        }
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

                // Process matched portion with timeout + error policy
                match invoke_with_policy(step.mailet.as_ref(), &mut matched_mail, &step.config)
                    .await?
                {
                    Some(action) => {
                        tracing::debug!(
                            "Mailet {} returned action: {:?}",
                            step.mailet.name(),
                            action
                        );

                        // Handle state changes from matched portion
                        if matched_mail.state != self.state {
                            tracing::debug!(
                                "Matched mail state changed from {:?} to {:?}",
                                self.state,
                                matched_mail.state
                            );
                        }
                    }
                    None => {
                        // Skip policy: continue with merged mail
                        tracing::debug!("Mailet {} skipped (Skip policy)", step.mailet.name());
                    }
                }

                // Continue with unmatched
                mail = unmatched_mail;
            } else {
                // All recipients match
                tracing::trace!(
                    "Matcher {} matched all {} recipients",
                    step.matcher.name(),
                    mail.recipients().len()
                );

                match invoke_with_policy(step.mailet.as_ref(), &mut mail, &step.config).await? {
                    Some(action) => {
                        tracing::debug!(
                            "Mailet {} returned action: {:?}",
                            step.mailet.name(),
                            action
                        );
                    }
                    None => {
                        // Skip policy: do nothing, continue to next step
                        tracing::debug!("Mailet {} skipped (Skip policy)", step.mailet.name());
                        continue;
                    }
                }
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
    use crate::mailet::{MailetAction, MailetConfig, MailetError, MailetErrorPolicy};
    use crate::matcher::AllMatcher;
    use async_trait::async_trait;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};
    use std::time::Duration;

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

    /// A mailet that sleeps for a fixed duration before returning
    struct SlowMailet {
        sleep_ms: u64,
    }

    #[async_trait]
    impl Mailet for SlowMailet {
        async fn init(&mut self, _config: MailetConfig) -> anyhow::Result<()> {
            Ok(())
        }

        async fn service(&self, _mail: &mut Mail) -> anyhow::Result<MailetAction> {
            tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
            Ok(MailetAction::Continue)
        }

        fn name(&self) -> &str {
            "SlowMailet"
        }
    }

    /// A mailet that always fails
    struct FailingMailet {
        name: String,
        call_count: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait]
    impl Mailet for FailingMailet {
        async fn init(&mut self, _config: MailetConfig) -> anyhow::Result<()> {
            Ok(())
        }

        async fn service(&self, _mail: &mut Mail) -> anyhow::Result<MailetAction> {
            self.call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(anyhow::anyhow!("intentional test failure"))
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    /// A mailet that succeeds and records that it ran
    struct MarkerMailet {
        name: String,
        marker: String,
    }

    #[async_trait]
    impl Mailet for MarkerMailet {
        async fn init(&mut self, _config: MailetConfig) -> anyhow::Result<()> {
            Ok(())
        }

        async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
            mail.set_attribute(self.marker.clone(), true);
            Ok(MailetAction::Continue)
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    fn make_test_mail() -> Mail {
        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));
        Mail::new(
            None,
            vec!["user@example.com".parse().unwrap()],
            message,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn mailet_execution_timeout() {
        // Mailet sleeps 200 ms; timeout is 50 ms → should produce Timeout error
        let timeout_ms = 50u64;
        let sleep_ms = 200u64;

        let result = invoke_with_timeout(
            &SlowMailet { sleep_ms },
            &mut make_test_mail(),
            Some(timeout_ms),
        )
        .await;

        assert!(
            matches!(result, Err(MailetError::Timeout(_))),
            "Expected Timeout error, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn mailet_error_policy_skip() {
        // A failing mailet with Skip policy should allow the next mailet to run
        let mut processor = Processor::new("test", MailState::Root);

        let failing = Arc::new(FailingMailet {
            name: "failing".to_string(),
            call_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        });

        let marker = Arc::new(MarkerMailet {
            name: "marker".to_string(),
            marker: "marker_ran".to_string(),
        });

        let skip_config = MailetConfig::new("failing").with_error_policy(MailetErrorPolicy::Skip);

        processor.add_step(ProcessingStep::new_with_config(
            Arc::new(AllMatcher),
            failing,
            skip_config,
        ));
        processor.add_step(ProcessingStep::new(Arc::new(AllMatcher), marker));

        let result = processor.process(make_test_mail()).await.unwrap();
        assert!(
            result.get_attribute("marker_ran").is_some(),
            "Next mailet should have run after Skip"
        );
    }

    #[tokio::test]
    async fn mailet_error_policy_abort() {
        // A failing mailet with Abort policy should propagate the error
        let mut processor = Processor::new("test", MailState::Root);

        let failing = Arc::new(FailingMailet {
            name: "failing".to_string(),
            call_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        });

        let abort_config = MailetConfig::new("failing").with_error_policy(MailetErrorPolicy::Abort);

        processor.add_step(ProcessingStep::new_with_config(
            Arc::new(AllMatcher),
            failing,
            abort_config,
        ));

        let result = processor.process(make_test_mail()).await;
        assert!(result.is_err(), "Abort policy should propagate error");
    }

    #[tokio::test]
    async fn mailet_error_policy_retry_then_abort() {
        // Retry with max:2 → mailet called 1 + 2 = 3 times total, then Abort
        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let failing = Arc::new(FailingMailet {
            name: "failing".to_string(),
            call_count: Arc::clone(&call_count),
        });

        let retry_config =
            MailetConfig::new("failing").with_error_policy(MailetErrorPolicy::Retry {
                max: 2,
                backoff: Duration::from_millis(1), // 1ms backoff for test speed
            });

        let result =
            invoke_with_policy(failing.as_ref(), &mut make_test_mail(), &retry_config).await;

        assert!(result.is_err(), "Should error after exhausting retries");
        // 1 initial + 2 retries = 3 calls
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "Should have been called 3 times (1 initial + 2 retries)"
        );
    }
}
