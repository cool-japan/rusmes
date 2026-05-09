//! Mail envelope and state machine

use crate::address::MailAddress;
use crate::message::{MessageId, MimeMessage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;

/// Unique identifier for a mail item (distinct from MessageId)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MailId(Uuid);

impl MailId {
    /// Create a new unique mail ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the UUID value
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for MailId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MailId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Mail processing state
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MailState {
    /// Entry point for incoming mail
    Root,
    /// Outbound delivery to remote servers
    Transport,
    /// Local mailbox delivery
    LocalDelivery,
    /// Error handling
    Error,
    /// Deleted/dropped mail
    Ghost,
    /// Custom user-defined state
    Custom(String),
}

impl std::fmt::Display for MailState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MailState::Root => write!(f, "root"),
            MailState::Transport => write!(f, "transport"),
            MailState::LocalDelivery => write!(f, "local-delivery"),
            MailState::Error => write!(f, "error"),
            MailState::Ghost => write!(f, "ghost"),
            MailState::Custom(s) => write!(f, "{}", s),
        }
    }
}

/// Attribute value type
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttributeValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Bytes(Vec<u8>),
}

impl From<String> for AttributeValue {
    fn from(s: String) -> Self {
        AttributeValue::String(s)
    }
}

impl From<&str> for AttributeValue {
    fn from(s: &str) -> Self {
        AttributeValue::String(s.to_string())
    }
}

impl From<i64> for AttributeValue {
    fn from(i: i64) -> Self {
        AttributeValue::Integer(i)
    }
}

impl From<f64> for AttributeValue {
    fn from(f: f64) -> Self {
        AttributeValue::Float(f)
    }
}

impl From<bool> for AttributeValue {
    fn from(b: bool) -> Self {
        AttributeValue::Boolean(b)
    }
}

impl From<Vec<u8>> for AttributeValue {
    fn from(v: Vec<u8>) -> Self {
        AttributeValue::Bytes(v)
    }
}

impl AttributeValue {
    /// Try to get the value as a string reference
    pub fn as_str(&self) -> Option<&str> {
        match self {
            AttributeValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Try to get the value as an integer
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            AttributeValue::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to get the value as a float
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            AttributeValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Try to get the value as a boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            AttributeValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get the value as bytes
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            AttributeValue::Bytes(b) => Some(b.as_slice()),
            _ => None,
        }
    }
}

/// Mail envelope and message wrapper
#[derive(Debug, Clone)]
pub struct Mail {
    /// Unique mail identifier
    id: MailId,
    /// Current processing state
    pub state: MailState,
    /// Envelope sender (can be None for bounce messages)
    sender: Option<MailAddress>,
    /// Envelope recipients
    recipients: Vec<MailAddress>,
    /// The actual message
    message: Arc<MimeMessage>,
    /// Custom attributes for mailet communication
    attributes: HashMap<String, AttributeValue>,
    /// Remote client IP address
    remote_addr: Option<IpAddr>,
    /// Remote client hostname
    remote_host: Option<String>,
    /// Message-ID from headers (different from MailId)
    message_id: MessageId,
}

impl Mail {
    /// Create a new mail item
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sender: Option<MailAddress>,
        recipients: Vec<MailAddress>,
        message: MimeMessage,
        remote_addr: Option<IpAddr>,
        remote_host: Option<String>,
    ) -> Self {
        Self {
            id: MailId::new(),
            state: MailState::Root,
            sender,
            recipients,
            message: Arc::new(message),
            attributes: HashMap::new(),
            remote_addr,
            remote_host,
            message_id: MessageId::new(),
        }
    }

    /// Get the mail ID
    pub fn id(&self) -> &MailId {
        &self.id
    }

    /// Get the message ID
    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }

    /// Create a new mail item with a specific message ID (for deserialization)
    #[allow(clippy::too_many_arguments)]
    pub fn with_message_id(
        sender: Option<MailAddress>,
        recipients: Vec<MailAddress>,
        message: MimeMessage,
        remote_addr: Option<IpAddr>,
        remote_host: Option<String>,
        message_id: MessageId,
    ) -> Self {
        Self {
            id: MailId::new(),
            state: MailState::Root,
            sender,
            recipients,
            message: Arc::new(message),
            attributes: HashMap::new(),
            remote_addr,
            remote_host,
            message_id,
        }
    }

    /// Get the sender
    pub fn sender(&self) -> Option<&MailAddress> {
        self.sender.as_ref()
    }

    /// Set the sender
    pub fn set_sender(&mut self, sender: Option<MailAddress>) {
        self.sender = sender;
    }

    /// Get recipients
    pub fn recipients(&self) -> &[MailAddress] {
        &self.recipients
    }

    /// Get mutable recipients
    pub fn recipients_mut(&mut self) -> &mut Vec<MailAddress> {
        &mut self.recipients
    }

    /// Set recipients
    pub fn set_recipients(&mut self, recipients: Vec<MailAddress>) {
        self.recipients = recipients;
    }

    /// Get the message
    pub fn message(&self) -> &Arc<MimeMessage> {
        &self.message
    }

    /// Set the message
    pub fn set_message(&mut self, message: Arc<MimeMessage>) {
        self.message = message;
    }

    /// Get remote address
    pub fn remote_addr(&self) -> Option<&IpAddr> {
        self.remote_addr.as_ref()
    }

    /// Get remote hostname
    pub fn remote_host(&self) -> Option<&str> {
        self.remote_host.as_deref()
    }

    /// Get an attribute
    pub fn get_attribute(&self, key: &str) -> Option<&AttributeValue> {
        self.attributes.get(key)
    }

    /// Set an attribute
    pub fn set_attribute(&mut self, key: impl Into<String>, value: impl Into<AttributeValue>) {
        self.attributes.insert(key.into(), value.into());
    }

    /// Remove an attribute
    pub fn remove_attribute(&mut self, key: &str) -> Option<AttributeValue> {
        self.attributes.remove(key)
    }

    /// Split mail into matched and unmatched portions
    pub fn split(mut self, matched_recipients: Vec<MailAddress>) -> (Self, Self) {
        let unmatched: Vec<MailAddress> = self
            .recipients
            .iter()
            .filter(|r| !matched_recipients.contains(r))
            .cloned()
            .collect();

        let mut matched_mail = self.clone();
        matched_mail.id = MailId::new(); // New ID for split mail
        matched_mail.recipients = matched_recipients;

        self.recipients = unmatched;

        (matched_mail, self)
    }

    /// Get total message size in bytes (full on-wire form: headers + body).
    ///
    /// Equivalent to [`MimeMessage::size_with_headers`]. This is what callers
    /// reporting SMTP `SIZE` (RFC 1870), IMAP `RFC822.SIZE` (RFC 9051), and
    /// quota usage want.
    pub fn size(&self) -> usize {
        self.message.size_with_headers()
    }

    /// Get the body byte length only (no headers).
    pub fn body_size(&self) -> usize {
        self.message.body_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{HeaderMap, MessageBody};
    use bytes::Bytes;

    #[test]
    fn test_mail_creation() {
        let sender = "sender@example.com".parse().unwrap();
        let recipients = vec!["rcpt@example.com".parse().unwrap()];
        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));

        let mail = Mail::new(Some(sender), recipients, message, None, None);
        assert_eq!(mail.state, MailState::Root);
        assert!(mail.sender().is_some());
        assert_eq!(mail.recipients().len(), 1);
    }

    #[test]
    fn test_mail_attributes() {
        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));
        let mut mail = Mail::new(None, vec![], message, None, None);

        mail.set_attribute("test_key", "test_value");
        assert!(mail.get_attribute("test_key").is_some());

        mail.remove_attribute("test_key");
        assert!(mail.get_attribute("test_key").is_none());
    }

    #[test]
    fn test_mail_split() {
        let recipients = vec![
            "user1@example.com".parse().unwrap(),
            "user2@example.com".parse().unwrap(),
            "user3@example.com".parse().unwrap(),
        ];
        let message = MimeMessage::new(HeaderMap::new(), MessageBody::Small(Bytes::from("Test")));

        let mail = Mail::new(None, recipients, message, None, None);
        let matched = vec!["user1@example.com".parse().unwrap()];

        let (matched_mail, unmatched_mail) = mail.split(matched);
        assert_eq!(matched_mail.recipients().len(), 1);
        assert_eq!(unmatched_mail.recipients().len(), 2);
    }
}
