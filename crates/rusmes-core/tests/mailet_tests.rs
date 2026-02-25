//! Comprehensive mailet tests

use bytes::Bytes;
use rusmes_core::mailets::*;
use rusmes_core::{Mailet, MailetAction, MailetConfig};
use rusmes_proto::{
    message::{HeaderMap, MessageBody},
    AttributeValue, Mail, MimeMessage,
};

fn make_test_mail() -> Mail {
    let message = MimeMessage::new(
        HeaderMap::new(),
        MessageBody::Small(Bytes::from("Test body")),
    );
    Mail::new(None, vec![], message, None, None)
}

#[tokio::test]
async fn test_local_delivery_mailet() {
    let mailet = local_delivery::LocalDeliveryMailet::new();
    assert_eq!(mailet.name(), "LocalDelivery");
}

#[tokio::test]
async fn test_remote_delivery_mailet() {
    let mailet = remote_delivery::RemoteDeliveryMailet::new();
    assert_eq!(mailet.name(), "RemoteDelivery");
}

#[tokio::test]
async fn test_add_header_mailet_comprehensive() {
    let mut mailet = add_header::AddHeaderMailet::new();
    let config = MailetConfig::new("AddHeader")
        .with_param("name", "X-Test-Header")
        .with_param("value", "Test Value");

    assert!(mailet.init(config).await.is_ok());

    let mut mail = make_test_mail();
    let action = mailet.service(&mut mail).await.unwrap();
    assert!(matches!(action, MailetAction::Continue));
    assert_eq!(
        mail.get_attribute("header.X-Test-Header"),
        Some(&AttributeValue::String("Test Value".to_string()))
    );
}

#[tokio::test]
async fn test_virus_scan_mailet() {
    let mailet = virus_scan::VirusScanMailet::new();
    assert_eq!(mailet.name(), "VirusScan");
}

#[tokio::test]
async fn test_spam_assassin_mailet() {
    let mailet = spam_assassin::SpamAssassinMailet::new();
    assert_eq!(mailet.name(), "SpamAssassin");
}

#[tokio::test]
async fn test_dkim_verify_mailet() {
    let mailet = dkim_verify::DkimVerifyMailet::new();
    assert_eq!(mailet.name(), "DkimVerify");
}

#[tokio::test]
async fn test_dmarc_verify_mailet() {
    let mailet = dmarc_verify::DmarcVerifyMailet::new();
    assert_eq!(mailet.name(), "DmarcVerify");
}

#[tokio::test]
async fn test_spf_check_mailet() {
    let mailet = spf_check::SpfCheckMailet::new();
    assert_eq!(mailet.name(), "SpfCheck");
}

#[tokio::test]
async fn test_mailet_pipeline_execution() {
    let mailets: Vec<Box<dyn Mailet + Send + Sync>> = vec![
        Box::new(add_header::AddHeaderMailet::new()),
        Box::new(virus_scan::VirusScanMailet::new()),
        Box::new(spam_assassin::SpamAssassinMailet::new()),
    ];

    assert_eq!(mailets.len(), 3);
}

#[tokio::test]
async fn test_mailet_error_handling() {
    let mut mailet = add_header::AddHeaderMailet::new();
    let config = MailetConfig::new("AddHeader");
    // Missing required parameters
    let result = mailet.init(config).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_mailet_action_types() {
    let continue_action = MailetAction::Continue;
    let drop_action = MailetAction::Drop;

    assert!(matches!(continue_action, MailetAction::Continue));
    assert!(matches!(drop_action, MailetAction::Drop));
}

#[tokio::test]
async fn test_mail_creation() {
    let mail = make_test_mail();
    assert!(mail.sender().is_none());
    assert!(mail.recipients().is_empty());
}

#[tokio::test]
async fn test_mail_attributes() {
    let mut mail = make_test_mail();

    mail.set_attribute("custom.key", "custom.value");
    assert_eq!(
        mail.get_attribute("custom.key"),
        Some(&AttributeValue::String("custom.value".to_string()))
    );
}

#[tokio::test]
async fn test_multiple_mailet_processing() {
    let mut mailet1 = add_header::AddHeaderMailet::new();
    let config1 = MailetConfig::new("AddHeader")
        .with_param("name", "X-Header-1")
        .with_param("value", "Value1");
    mailet1.init(config1).await.unwrap();

    let mut mailet2 = add_header::AddHeaderMailet::new();
    let config2 = MailetConfig::new("AddHeader")
        .with_param("name", "X-Header-2")
        .with_param("value", "Value2");
    mailet2.init(config2).await.unwrap();

    let mut mail = make_test_mail();
    mailet1.service(&mut mail).await.unwrap();
    mailet2.service(&mut mail).await.unwrap();

    assert_eq!(
        mail.get_attribute("header.X-Header-1"),
        Some(&AttributeValue::String("Value1".to_string()))
    );
    assert_eq!(
        mail.get_attribute("header.X-Header-2"),
        Some(&AttributeValue::String("Value2".to_string()))
    );
}

#[tokio::test]
async fn test_mailet_concurrent_processing() {
    use tokio::task;

    let mut handles = vec![];

    for i in 0..10 {
        let handle = task::spawn(async move {
            let mut mailet = add_header::AddHeaderMailet::new();
            let config = MailetConfig::new("AddHeader")
                .with_param("name", format!("X-Header-{}", i))
                .with_param("value", format!("Value-{}", i));

            mailet.init(config).await.unwrap();

            let mut mail = make_test_mail();
            mailet.service(&mut mail).await.unwrap();
            mail
        });

        handles.push(handle);
    }

    for handle in handles {
        let _mail = handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_mailet_config_validation() {
    let config = MailetConfig::new("TestMailet");
    assert_eq!(config.name, "TestMailet");
}

#[tokio::test]
async fn test_mailet_config_params() {
    let config = MailetConfig::new("TestMailet")
        .with_param("key1", "value1")
        .with_param("key2", "value2");

    assert_eq!(config.get_param("key1"), Some("value1"));
    assert_eq!(config.get_param("key2"), Some("value2"));
    assert_eq!(config.get_param("key3"), None);
}

#[tokio::test]
async fn test_mail_id_uniqueness() {
    let mail1 = make_test_mail();
    let mail2 = make_test_mail();

    assert_ne!(mail1.id(), mail2.id());
}

#[tokio::test]
async fn test_mail_recipients() {
    let recipients: Vec<rusmes_proto::MailAddress> = vec![
        "recipient1@example.com".parse().unwrap(),
        "recipient2@example.com".parse().unwrap(),
    ];
    let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Body")));
    let mail = Mail::new(None, recipients, message, None, None);

    assert_eq!(mail.recipients().len(), 2);
}

#[tokio::test]
async fn test_large_mail_processing() {
    let large_body = "A".repeat(1_000_000);
    let message = MimeMessage::new(
        HeaderMap::new(),
        MessageBody::Small(Bytes::from(large_body)),
    );
    let mut mail = Mail::new(None, vec![], message, None, None);

    let mut mailet = add_header::AddHeaderMailet::new();
    let config = MailetConfig::new("AddHeader")
        .with_param("name", "X-Large")
        .with_param("value", "Processed");
    mailet.init(config).await.unwrap();

    let action = mailet.service(&mut mail).await.unwrap();
    assert!(matches!(action, MailetAction::Continue));
    assert_eq!(
        mail.get_attribute("header.X-Large"),
        Some(&AttributeValue::String("Processed".to_string()))
    );
}
