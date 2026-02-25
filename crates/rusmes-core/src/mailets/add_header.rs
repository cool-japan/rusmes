//! Add header mailet

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::Mail;

/// Adds a header to the message
pub struct AddHeaderMailet {
    name: String,
    header_name: Option<String>,
    header_value: Option<String>,
}

impl AddHeaderMailet {
    /// Create a new add header mailet
    pub fn new() -> Self {
        Self {
            name: "AddHeader".to_string(),
            header_name: None,
            header_value: None,
        }
    }
}

impl Default for AddHeaderMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for AddHeaderMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        self.header_name = config.get_param("name").map(|s| s.to_string());
        self.header_value = config.get_param("value").map(|s| s.to_string());

        if self.header_name.is_none() || self.header_value.is_none() {
            return Err(anyhow::anyhow!(
                "AddHeaderMailet requires 'name' and 'value' parameters"
            ));
        }

        tracing::info!(
            "Initialized AddHeaderMailet: {} = {}",
            self.header_name.as_deref().unwrap_or(""),
            self.header_value.as_deref().unwrap_or("")
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        if let (Some(name), Some(value)) = (&self.header_name, &self.header_value) {
            tracing::debug!("Adding header '{}' to mail {}", name, mail.id());
            mail.set_attribute(format!("header.{}", name), value.clone());
        }
        Ok(MailetAction::Continue)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailet::MailetConfig;
    use bytes::Bytes;
    use rusmes_proto::{
        message::{HeaderMap, MessageBody},
        Mail, MimeMessage,
    };

    fn make_test_mail() -> Mail {
        let message = MimeMessage::new(
            HeaderMap::new(),
            MessageBody::Small(Bytes::from("Test body")),
        );
        Mail::new(None, vec![], message, None, None)
    }

    #[tokio::test]
    async fn test_add_header_mailet_creation() {
        let mailet = AddHeaderMailet::new();
        assert_eq!(mailet.name(), "AddHeader");
    }

    #[tokio::test]
    async fn test_add_header_mailet_init_success() {
        let mut mailet = AddHeaderMailet::new();
        let config = MailetConfig::new("AddHeader")
            .with_param("name", "X-Custom-Header")
            .with_param("value", "CustomValue");

        let result = mailet.init(config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_add_header_mailet_init_missing_name() {
        let mut mailet = AddHeaderMailet::new();
        let config = MailetConfig::new("AddHeader").with_param("value", "CustomValue");

        let result = mailet.init(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_add_header_mailet_init_missing_value() {
        let mut mailet = AddHeaderMailet::new();
        let config = MailetConfig::new("AddHeader").with_param("name", "X-Custom-Header");

        let result = mailet.init(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_add_header_mailet_service() {
        let mut mailet = AddHeaderMailet::new();
        let config = MailetConfig::new("AddHeader")
            .with_param("name", "X-Test")
            .with_param("value", "TestValue");
        mailet.init(config).await.unwrap();

        let mut mail = make_test_mail();
        let action = mailet.service(&mut mail).await.unwrap();
        assert!(matches!(action, MailetAction::Continue));

        assert!(mail.get_attribute("header.X-Test").is_some());
        assert_eq!(
            mail.get_attribute("header.X-Test"),
            Some(&rusmes_proto::AttributeValue::String(
                "TestValue".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn test_add_header_mailet_multiple_headers() {
        let mut mailet1 = AddHeaderMailet::new();
        let config1 = MailetConfig::new("AddHeader")
            .with_param("name", "X-Header-1")
            .with_param("value", "Value1");
        mailet1.init(config1).await.unwrap();

        let mut mailet2 = AddHeaderMailet::new();
        let config2 = MailetConfig::new("AddHeader")
            .with_param("name", "X-Header-2")
            .with_param("value", "Value2");
        mailet2.init(config2).await.unwrap();

        let mut mail = make_test_mail();
        mailet1.service(&mut mail).await.unwrap();
        mailet2.service(&mut mail).await.unwrap();

        assert_eq!(
            mail.get_attribute("header.X-Header-1"),
            Some(&rusmes_proto::AttributeValue::String("Value1".to_string()))
        );
        assert_eq!(
            mail.get_attribute("header.X-Header-2"),
            Some(&rusmes_proto::AttributeValue::String("Value2".to_string()))
        );
    }

    #[tokio::test]
    async fn test_add_header_default() {
        let mailet = AddHeaderMailet::default();
        assert_eq!(mailet.name(), "AddHeader");
    }

    #[tokio::test]
    async fn test_add_header_special_characters() {
        let mut mailet = AddHeaderMailet::new();
        let config = MailetConfig::new("AddHeader")
            .with_param("name", "X-Special")
            .with_param("value", "Value with spaces & symbols!");
        mailet.init(config).await.unwrap();

        let mut mail = make_test_mail();
        mailet.service(&mut mail).await.unwrap();
        assert_eq!(
            mail.get_attribute("header.X-Special"),
            Some(&rusmes_proto::AttributeValue::String(
                "Value with spaces & symbols!".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn test_add_header_empty_value() {
        let mut mailet = AddHeaderMailet::new();
        let config = MailetConfig::new("AddHeader")
            .with_param("name", "X-Empty")
            .with_param("value", "");
        mailet.init(config).await.unwrap();

        let mut mail = make_test_mail();
        mailet.service(&mut mail).await.unwrap();
        assert_eq!(
            mail.get_attribute("header.X-Empty"),
            Some(&rusmes_proto::AttributeValue::String("".to_string()))
        );
    }

    #[tokio::test]
    async fn test_add_header_long_value() {
        let long_value = "a".repeat(1000);
        let mut mailet = AddHeaderMailet::new();
        let config = MailetConfig::new("AddHeader")
            .with_param("name", "X-Long")
            .with_param("value", &long_value);
        mailet.init(config).await.unwrap();

        let mut mail = make_test_mail();
        mailet.service(&mut mail).await.unwrap();
        assert_eq!(
            mail.get_attribute("header.X-Long"),
            Some(&rusmes_proto::AttributeValue::String(long_value.clone()))
        );
    }
}
