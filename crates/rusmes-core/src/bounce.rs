//! Bounce message generation

use bytes::Bytes;
use rusmes_proto::message::{HeaderMap, MessageBody};
use rusmes_proto::{Mail, MailAddress, MimeMessage};

/// Generate a bounce message for delivery failure
pub fn generate_bounce(
    original_mail: &Mail,
    error: &str,
    postmaster: &MailAddress,
) -> anyhow::Result<Mail> {
    let sender = original_mail
        .sender()
        .ok_or_else(|| anyhow::anyhow!("Cannot generate bounce: original mail has no sender"))?;

    // Build bounce message
    let mut headers = HeaderMap::new();
    headers.insert("From", postmaster.as_string());
    headers.insert("To", sender.as_string());
    headers.insert("Subject", "Mail Delivery Failure");
    headers.insert("Auto-Submitted", "auto-replied");

    let body_text = format!(
        r#"This is an automatically generated Delivery Status Notification.

Your message could not be delivered to one or more recipients.

Original Message ID: {}
Error: {}

Recipients:
{}

--- Original Message Headers ---
(truncated)
"#,
        original_mail.message_id(),
        error,
        original_mail
            .recipients()
            .iter()
            .map(|r| format!("  - {}", r))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let message = MimeMessage::new(headers, MessageBody::Small(Bytes::from(body_text)));

    let bounce = Mail::new(
        Some(postmaster.clone()),
        vec![sender.clone()],
        message,
        None,
        None,
    );

    tracing::info!(
        "Generated bounce message for {} (original: {})",
        sender,
        original_mail.id()
    );

    Ok(bounce)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MessageBody, MimeMessage};

    #[test]
    fn test_generate_bounce() {
        let sender: MailAddress = "sender@example.com".parse().unwrap();
        let recipient: MailAddress = "recipient@example.com".parse().unwrap();
        let postmaster: MailAddress = "postmaster@example.com".parse().unwrap();

        let message = MimeMessage::new(
            HeaderMap::new(),
            MessageBody::Small(Bytes::from("Test message")),
        );
        let mail = Mail::new(Some(sender.clone()), vec![recipient], message, None, None);

        let bounce = generate_bounce(&mail, "Connection refused", &postmaster).unwrap();

        assert_eq!(bounce.sender().unwrap(), &postmaster);
        assert_eq!(bounce.recipients().len(), 1);
        assert_eq!(bounce.recipients()[0], sender);
    }
}
