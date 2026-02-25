//! Mail processor router

use crate::processor::Processor;
use rusmes_metrics::MetricsCollector;
use rusmes_proto::{Mail, MailState};
use std::collections::HashMap;
use std::sync::Arc;

/// Routes mail between processors based on state
pub struct MailProcessorRouter {
    processors: HashMap<MailState, Arc<Processor>>,
    error_processor: Option<Arc<Processor>>,
    metrics: Arc<MetricsCollector>,
}

impl MailProcessorRouter {
    /// Create a new mail processor router
    pub fn new(metrics: Arc<MetricsCollector>) -> Self {
        Self {
            processors: HashMap::new(),
            error_processor: None,
            metrics,
        }
    }

    /// Register a processor for a state
    pub fn register_processor(&mut self, state: MailState, processor: Arc<Processor>) {
        self.processors.insert(state, processor);
    }

    /// Set the error processor
    pub fn set_error_processor(&mut self, processor: Arc<Processor>) {
        self.error_processor = Some(processor);
    }

    /// Route and process a mail message
    pub async fn route(&self, mut mail: Mail) -> anyhow::Result<()> {
        let mut processing_depth = 0;
        const MAX_DEPTH: usize = 100; // Prevent infinite loops

        loop {
            if processing_depth > MAX_DEPTH {
                tracing::error!(
                    "Mail processing exceeded max depth for mail {}: {}",
                    mail.id(),
                    processing_depth
                );
                mail.state = MailState::Error;
            }

            // Get processor for current state
            let processor = self.processors.get(&mail.state).ok_or_else(|| {
                anyhow::anyhow!("No processor registered for state: {:?}", mail.state)
            })?;

            tracing::debug!(
                "Routing mail {} to processor {} (state: {:?})",
                mail.id(),
                processor.name(),
                mail.state
            );

            // Process mail
            let original_state = mail.state.clone();
            mail = processor.process(mail).await?;

            // Check for completion
            if mail.state == MailState::Ghost {
                tracing::info!("Mail {} completed processing (Ghost state)", mail.id());
                self.metrics.record_mail_completed(&mail);
                return Ok(()); // Mail consumed
            }

            // Check if state changed
            if mail.state == original_state {
                tracing::debug!(
                    "Mail {} processing complete in state {:?} (no state change)",
                    mail.id(),
                    mail.state
                );

                // No state change means processing complete for this state
                if mail.state == MailState::Error {
                    return Err(anyhow::anyhow!(
                        "Mail {} processing failed in Error state",
                        mail.id()
                    ));
                }
                return Ok(());
            }

            tracing::debug!(
                "Mail {} state changed from {:?} to {:?}",
                mail.id(),
                original_state,
                mail.state
            );

            processing_depth += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailet::{Mailet, MailetAction, MailetConfig};
    use crate::matcher::AllMatcher;
    use crate::processor::ProcessingStep;
    use async_trait::async_trait;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

    struct StateChangeMailet {
        target_state: MailState,
    }

    #[async_trait]
    impl Mailet for StateChangeMailet {
        async fn init(&mut self, _config: MailetConfig) -> anyhow::Result<()> {
            Ok(())
        }

        async fn service(&self, _mail: &mut Mail) -> anyhow::Result<MailetAction> {
            Ok(MailetAction::ChangeState(self.target_state.clone()))
        }

        fn name(&self) -> &str {
            "StateChangeMailet"
        }
    }

    struct DropMailet;

    #[async_trait]
    impl Mailet for DropMailet {
        async fn init(&mut self, _config: MailetConfig) -> anyhow::Result<()> {
            Ok(())
        }

        async fn service(&self, _mail: &mut Mail) -> anyhow::Result<MailetAction> {
            Ok(MailetAction::Drop)
        }

        fn name(&self) -> &str {
            "DropMailet"
        }
    }

    #[tokio::test]
    async fn test_router_state_change() {
        let metrics = Arc::new(MetricsCollector::new());
        let mut router = MailProcessorRouter::new(metrics);

        // Root processor - changes to Transport
        let mut root_processor = Processor::new("root", MailState::Root);
        root_processor.add_step(ProcessingStep::new(
            Arc::new(AllMatcher),
            Arc::new(StateChangeMailet {
                target_state: MailState::Transport,
            }),
        ));

        // Transport processor - drops mail
        let mut transport_processor = Processor::new("transport", MailState::Transport);
        transport_processor.add_step(ProcessingStep::new(
            Arc::new(AllMatcher),
            Arc::new(DropMailet),
        ));

        router.register_processor(MailState::Root, Arc::new(root_processor));
        router.register_processor(MailState::Transport, Arc::new(transport_processor));

        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));
        let mail = Mail::new(
            None,
            vec!["user@example.com".parse().unwrap()],
            message,
            None,
            None,
        );

        router.route(mail).await.unwrap();
    }
}
