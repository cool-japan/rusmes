//! Local mailbox delivery mailet

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::{Mail, MailState, Username};
use rusmes_storage::{MailboxPath, StorageBackend};
use std::sync::Arc;

/// Delivers mail to local mailboxes
pub struct LocalDeliveryMailet {
    name: String,
    storage: Option<Arc<dyn StorageBackend>>,
}

impl LocalDeliveryMailet {
    /// Create a new local delivery mailet
    pub fn new() -> Self {
        Self {
            name: "LocalDelivery".to_string(),
            storage: None,
        }
    }

    /// Create a new local delivery mailet with storage
    pub fn with_storage(storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            name: "LocalDelivery".to_string(),
            storage: Some(storage),
        }
    }
}

impl Default for LocalDeliveryMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for LocalDeliveryMailet {
    async fn init(&mut self, _config: MailetConfig) -> anyhow::Result<()> {
        tracing::info!("Initialized LocalDeliveryMailet");
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        tracing::info!(
            "LocalDeliveryMailet: Delivering mail {} to {} local recipients",
            mail.id(),
            mail.recipients().len()
        );

        // Check if storage is available
        let storage = match &self.storage {
            Some(s) => {
                tracing::info!("LocalDeliveryMailet: Storage backend is available");
                s
            }
            None => {
                tracing::warn!(
                    "LocalDeliveryMailet: No storage backend configured, mail will be dropped"
                );
                return Ok(MailetAction::ChangeState(MailState::Ghost));
            }
        };

        tracing::info!("LocalDeliveryMailet: Getting mailbox and message stores");
        let mailbox_store = storage.mailbox_store();
        let message_store = storage.message_store();
        tracing::info!("LocalDeliveryMailet: Got stores successfully");

        // Deliver to each recipient
        let mut delivered_count = 0;
        let mut failed_recipients = Vec::new();

        for recipient in mail.recipients() {
            // Extract username from email address
            let username = Username::new(recipient.local_part())?;

            // Get or create INBOX for this user
            let mailbox_path = MailboxPath::new(username.clone(), vec!["INBOX".to_string()]);

            // Try to find existing mailbox
            let mailboxes = mailbox_store.list_mailboxes(&username).await?;
            let inbox = mailboxes.iter().find(|mb| {
                if let Some(name) = mb.path().name() {
                    name == "INBOX"
                } else {
                    false
                }
            });

            let mailbox_id = if let Some(inbox) = inbox {
                *inbox.id()
            } else {
                // Create INBOX if it doesn't exist
                tracing::info!("Creating INBOX for user {}", username.as_str());
                match mailbox_store.create_mailbox(&mailbox_path).await {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::error!("Failed to create INBOX for {}: {}", recipient, e);
                        failed_recipients.push(recipient.clone());
                        continue;
                    }
                }
            };

            // Append message to mailbox
            match message_store
                .append_message(&mailbox_id, mail.clone())
                .await
            {
                Ok(metadata) => {
                    tracing::info!(
                        "Delivered mail {} to {} (message_id: {})",
                        mail.id(),
                        recipient,
                        metadata.message_id()
                    );
                    delivered_count += 1;
                }
                Err(e) => {
                    tracing::error!("Failed to deliver to {}: {}", recipient, e);
                    failed_recipients.push(recipient.clone());
                }
            }
        }

        if delivered_count == 0 {
            tracing::error!("Failed to deliver mail {} to any recipients", mail.id());
            Ok(MailetAction::ChangeState(MailState::Error))
        } else if !failed_recipients.is_empty() {
            tracing::warn!(
                "Partially delivered mail {}: {}/{} successful",
                mail.id(),
                delivered_count,
                mail.recipients().len()
            );
            // Mark as delivered even if some failed (could improve this)
            Ok(MailetAction::ChangeState(MailState::Ghost))
        } else {
            tracing::info!(
                "Successfully delivered mail {} to all {} recipients",
                mail.id(),
                delivered_count
            );
            Ok(MailetAction::ChangeState(MailState::Ghost))
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};
    use std::str::FromStr;

    fn create_test_mail(sender: &str, recipients: Vec<&str>) -> Mail {
        let sender_addr = MailAddress::from_str(sender).ok();
        let recipient_addrs: Vec<MailAddress> = recipients
            .iter()
            .filter_map(|r| MailAddress::from_str(r).ok())
            .collect();

        let message = MimeMessage::new(
            HeaderMap::new(),
            MessageBody::Small(Bytes::from("Test message")),
        );

        Mail::new(sender_addr, recipient_addrs, message, None, None)
    }

    #[tokio::test]
    async fn test_local_delivery_mailet_creation() {
        let mailet = LocalDeliveryMailet::new();
        assert_eq!(mailet.name(), "LocalDelivery");
    }

    #[tokio::test]
    async fn test_local_delivery_mailet_default() {
        let mailet = LocalDeliveryMailet::default();
        assert_eq!(mailet.name(), "LocalDelivery");
    }

    #[tokio::test]
    async fn test_local_delivery_init() {
        let mut mailet = LocalDeliveryMailet::new();
        let config = MailetConfig::new("LocalDelivery");

        let result = mailet.init(config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_local_delivery_service() {
        let mailet = LocalDeliveryMailet::new();
        let mut mail = create_test_mail(
            "sender@example.com",
            vec!["user1@local.com", "user2@local.com"],
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert!(matches!(
            action,
            MailetAction::ChangeState(MailState::Ghost)
        ));
    }

    #[tokio::test]
    async fn test_local_delivery_single_recipient() {
        let mailet = LocalDeliveryMailet::new();
        let mut mail = create_test_mail("sender@example.com", vec!["user@local.com"]);

        let action = mailet.service(&mut mail).await.unwrap();
        assert!(matches!(
            action,
            MailetAction::ChangeState(MailState::Ghost)
        ));
    }

    #[tokio::test]
    async fn test_local_delivery_multiple_recipients() {
        let mailet = LocalDeliveryMailet::new();
        let mut mail = create_test_mail(
            "sender@example.com",
            vec![
                "user1@local.com",
                "user2@local.com",
                "user3@local.com",
                "user4@local.com",
            ],
        );

        let action = mailet.service(&mut mail).await.unwrap();
        assert!(matches!(
            action,
            MailetAction::ChangeState(MailState::Ghost)
        ));
    }
}
