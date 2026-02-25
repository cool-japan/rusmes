//! MIME message types

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncRead;
use uuid::Uuid;

/// Unique identifier for a message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(Uuid);

impl MessageId {
    /// Create a new unique message ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a MessageId from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the UUID value
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// MIME message structure
#[derive(Debug, Clone)]
pub struct MimeMessage {
    headers: HeaderMap,
    body: MessageBody,
}

impl MimeMessage {
    /// Create a new MIME message
    pub fn new(headers: HeaderMap, body: MessageBody) -> Self {
        Self { headers, body }
    }

    /// Get message headers
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get mutable message headers
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Get message body
    pub fn body(&self) -> &MessageBody {
        &self.body
    }

    /// Extract text content from message (simplified for now)
    pub fn extract_text(&self) -> crate::error::Result<String> {
        match &self.body {
            MessageBody::Small(bytes) => String::from_utf8(bytes.to_vec())
                .map_err(|e| crate::error::MailError::Parse(e.to_string())),
            MessageBody::Large(_) => {
                // For large messages, we'd need to stream and decode
                Ok(String::new())
            }
        }
    }

    /// Get message size in bytes
    pub fn size(&self) -> usize {
        match &self.body {
            MessageBody::Small(bytes) => bytes.len(),
            MessageBody::Large(_) => 0, // Would need to track separately
        }
    }

    /// Parse MIME message from raw bytes
    pub fn parse_from_bytes(data: &[u8]) -> crate::error::Result<Self> {
        let (headers_map, body_offset) = crate::mime::parse_headers(data)?;

        // Convert to HeaderMap
        let mut header_map = HeaderMap::new();
        for (name, value) in headers_map {
            header_map.insert(name, value);
        }

        // Extract body
        let body_bytes = if body_offset < data.len() {
            Bytes::copy_from_slice(&data[body_offset..])
        } else {
            Bytes::new()
        };

        let body = MessageBody::Small(body_bytes);

        Ok(MimeMessage::new(header_map, body))
    }

    /// Get Content-Type header parsed
    pub fn content_type(&self) -> crate::error::Result<Option<crate::mime::ContentType>> {
        if let Some(ct) = self.headers.get_first("content-type") {
            Ok(Some(crate::mime::ContentType::parse(ct)?))
        } else {
            Ok(None)
        }
    }

    /// Get Content-Transfer-Encoding
    pub fn content_transfer_encoding(&self) -> crate::mime::ContentTransferEncoding {
        if let Some(cte) = self.headers.get_first("content-transfer-encoding") {
            crate::mime::ContentTransferEncoding::parse(cte.trim())
        } else {
            crate::mime::ContentTransferEncoding::SevenBit
        }
    }

    /// Parse multipart message into parts
    pub fn parse_multipart(&self) -> crate::error::Result<Vec<crate::mime::MimePart>> {
        let content_type = self
            .content_type()?
            .ok_or_else(|| crate::error::MailError::Parse("No Content-Type header".to_string()))?;

        if !content_type.is_multipart() {
            return Err(crate::error::MailError::Parse(
                "Not a multipart message".to_string(),
            ));
        }

        let boundary = content_type.boundary().ok_or_else(|| {
            crate::error::MailError::Parse("No boundary in multipart".to_string())
        })?;

        match &self.body {
            MessageBody::Small(bytes) => crate::mime::split_multipart(bytes, boundary),
            MessageBody::Large(_) => Err(crate::error::MailError::Parse(
                "Cannot parse multipart from large message stream".to_string(),
            )),
        }
    }

    /// Decode message body according to Content-Transfer-Encoding
    pub fn decode_body(&self) -> crate::error::Result<Vec<u8>> {
        let encoding = self.content_transfer_encoding();

        match &self.body {
            MessageBody::Small(bytes) => match encoding {
                crate::mime::ContentTransferEncoding::Base64 => crate::mime::decode_base64(bytes),
                crate::mime::ContentTransferEncoding::QuotedPrintable => {
                    crate::mime::decode_quoted_printable(bytes)
                }
                _ => Ok(bytes.to_vec()),
            },
            MessageBody::Large(_) => Err(crate::error::MailError::Parse(
                "Cannot decode large message stream".to_string(),
            )),
        }
    }
}

/// Message body - optimized for small and large messages
#[derive(Clone)]
pub enum MessageBody {
    /// Small message stored in memory (<1MB)
    Small(Bytes),
    /// Large message reference (streaming support)
    Large(Arc<dyn AsyncRead + Send + Sync>),
}

impl std::fmt::Debug for MessageBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageBody::Small(bytes) => f.debug_tuple("Small").field(&bytes.len()).finish(),
            MessageBody::Large(_) => f.debug_tuple("Large").field(&"<stream>").finish(),
        }
    }
}

/// Message headers as a map
#[derive(Debug, Clone, Default)]
pub struct HeaderMap {
    headers: HashMap<String, Vec<String>>,
}

impl HeaderMap {
    /// Create a new empty header map
    pub fn new() -> Self {
        Self {
            headers: HashMap::new(),
        }
    }

    /// Insert a header value
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into().to_lowercase();
        let value = value.into();
        self.headers.entry(name).or_default().push(value);
    }

    /// Get all values for a header
    pub fn get(&self, name: &str) -> Option<&[String]> {
        self.headers.get(&name.to_lowercase()).map(|v| v.as_slice())
    }

    /// Get the first value for a header
    pub fn get_first(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(|v| v.first().map(|s| s.as_str()))
    }

    /// Remove a header
    pub fn remove(&mut self, name: &str) -> Option<Vec<String>> {
        self.headers.remove(&name.to_lowercase())
    }

    /// Check if a header exists
    pub fn contains(&self, name: &str) -> bool {
        self.headers.contains_key(&name.to_lowercase())
    }

    /// Iterate over all headers
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.headers.iter()
    }

    /// Parse headers from raw bytes with folding support
    pub fn parse_from_bytes(data: &[u8]) -> crate::error::Result<(Self, usize)> {
        let (headers_map, offset) = crate::mime::parse_headers(data)?;

        let mut header_map = HeaderMap::new();
        for (name, value) in headers_map {
            header_map.insert(name, value);
        }

        Ok((header_map, offset))
    }

    /// Fold a header value for proper line length
    pub fn fold_value(value: &str) -> String {
        crate::mime::fold_header(value, 78)
    }

    /// Unfold a header value
    pub fn unfold_value(value: &str) -> String {
        crate::mime::unfold_header(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_id_unique() {
        let id1 = MessageId::new();
        let id2 = MessageId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_header_map_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "text/plain");
        headers.insert("content-type", "text/html");

        let values = headers.get("CONTENT-TYPE").unwrap();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&"text/plain".to_string()));
        assert!(values.contains(&"text/html".to_string()));
    }

    #[test]
    fn test_small_message_body() {
        let body = MessageBody::Small(Bytes::from("Hello, World!"));
        let headers = HeaderMap::new();
        let msg = MimeMessage::new(headers, body);
        assert_eq!(msg.size(), 13);
    }
}
