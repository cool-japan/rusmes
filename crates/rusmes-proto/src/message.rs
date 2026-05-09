//! MIME message types

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
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

/// A streaming body for messages too large to hold in memory.
///
/// Backed by a pinned async reader behind a Mutex so the body can be
/// cheaply cloned (Arc) while remaining safely pollable by one consumer
/// at a time.
///
/// # One-shot semantics
///
/// `read_to_bytes` reads from the current stream position to EOF.
/// Calling it a second time will return an empty buffer (the reader is at EOF).
/// If you need to re-read the body, construct a new `LargeBody` via
/// [`LargeBody::from_path`].
#[derive(Clone)]
pub struct LargeBody {
    reader: Arc<tokio::sync::Mutex<std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send + Sync>>>>,
    /// Pre-known byte count (e.g. file size). Callers should not read past this.
    size: u64,
    /// Optional SHA-256 digest of the full body content.
    digest: Option<[u8; 32]>,
}

impl std::fmt::Debug for LargeBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LargeBody")
            .field("size", &self.size)
            .finish()
    }
}

impl LargeBody {
    /// Wrap an async reader of known size.
    pub fn from_reader<R: tokio::io::AsyncRead + Send + Sync + 'static>(
        reader: R,
        size: u64,
    ) -> Self {
        Self {
            reader: Arc::new(tokio::sync::Mutex::new(Box::pin(reader))),
            size,
            digest: None,
        }
    }

    /// Open a file as a streaming body.
    pub async fn from_path(path: &std::path::Path) -> crate::error::Result<Self> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| crate::error::MailError::Parse(format!("cannot open large body: {e}")))?;
        let size = file
            .metadata()
            .await
            .map_err(|e| crate::error::MailError::Parse(format!("cannot stat large body: {e}")))?
            .len();
        Ok(Self::from_reader(file, size))
    }

    /// Pre-known byte count.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// SHA-256 digest if available.
    pub fn digest(&self) -> Option<&[u8; 32]> {
        self.digest.as_ref()
    }

    /// Set the digest (used by callers that computed it during streaming).
    pub fn set_digest(&mut self, digest: [u8; 32]) {
        self.digest = Some(digest);
    }

    /// Read the entire body into memory.
    ///
    /// For large messages this is expensive — prefer streaming consumers.
    ///
    /// # One-shot semantics
    ///
    /// This reads from the current stream position to EOF. A second call will
    /// return an empty buffer. Construct a new `LargeBody` to re-read.
    pub async fn read_to_bytes(&self) -> crate::error::Result<Bytes> {
        use tokio::io::AsyncReadExt;
        let mut guard = self.reader.lock().await;
        let mut buf = Vec::new();
        guard
            .as_mut()
            .read_to_end(&mut buf)
            .await
            .map_err(|e| crate::error::MailError::Parse(format!("read error: {e}")))?;
        Ok(Bytes::from(buf))
    }
}

/// Message body - optimized for small and large messages
#[derive(Clone, Debug)]
pub enum MessageBody {
    /// Small message stored in memory (<1MB)
    Small(Bytes),
    /// Large message backed by a streaming async reader.
    Large(LargeBody),
}

impl MessageBody {
    /// Choose `Small` or `Large` based on file size vs `threshold_bytes`.
    ///
    /// If the file fits within `threshold_bytes`, it is read into memory as `Small`.
    /// Otherwise it is opened as a streaming `Large` body.
    pub async fn from_path_with_threshold(
        path: &std::path::Path,
        threshold_bytes: u64,
    ) -> crate::error::Result<Self> {
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| crate::error::MailError::Parse(format!("stat failed: {e}")))?;
        if meta.len() <= threshold_bytes {
            let data = tokio::fs::read(path)
                .await
                .map_err(|e| crate::error::MailError::Parse(format!("read failed: {e}")))?;
            Ok(MessageBody::Small(Bytes::from(data)))
        } else {
            Ok(MessageBody::Large(LargeBody::from_path(path).await?))
        }
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

    /// Extract text content from message body.
    ///
    /// For `Small` bodies this is infallible in the sense that any non-UTF-8
    /// bytes would have caused an earlier parse failure; for `Large` bodies the
    /// reader is consumed once (see [`LargeBody`] one-shot semantics).
    pub async fn extract_text(&self) -> crate::error::Result<String> {
        match &self.body {
            MessageBody::Small(bytes) => String::from_utf8(bytes.to_vec())
                .map_err(|e| crate::error::MailError::Parse(e.to_string())),
            MessageBody::Large(large) => {
                let data = large.read_to_bytes().await?;
                Ok(String::from_utf8_lossy(&data).into_owned())
            }
        }
    }

    /// Get message body size in bytes (body only, no headers).
    ///
    /// Use [`Self::size_with_headers`] when SIZE/quota semantics require the
    /// full on-wire byte count including the serialized header block.
    pub fn body_size(&self) -> usize {
        match &self.body {
            MessageBody::Small(bytes) => bytes.len(),
            MessageBody::Large(large) => large.size() as usize,
        }
    }

    /// Get total message size in bytes (full on-wire form).
    ///
    /// Counts the serialized header block (every header line written as
    /// `Name: Value\r\n`, then a final `\r\n` blank line separating headers
    /// from the body) plus the body byte length.
    ///
    /// This is the value that should be reported for SMTP `SIZE` (RFC 1870),
    /// IMAP `RFC822.SIZE` (RFC 9051), and quota accounting — all of which
    /// describe the message as it appears on the wire, not just its body.
    ///
    /// Note: header values stored in [`HeaderMap`] have already been
    /// unfolded; this calculation reports them in their re-folded canonical
    /// CRLF-terminated single-line form. Callers that re-fold long header
    /// values for transmission may emit slightly more bytes; this helper
    /// gives the canonical lower-bound per-RFC count.
    pub fn size_with_headers(&self) -> usize {
        let mut total = 0usize;
        for (name, values) in self.headers.iter() {
            for value in values {
                // "Name: Value\r\n"
                total = total
                    .saturating_add(name.len())
                    .saturating_add(2) // ": "
                    .saturating_add(value.len())
                    .saturating_add(2); // "\r\n"
            }
        }
        // Final blank line separator between headers and body.
        total = total.saturating_add(2); // "\r\n"
        total.saturating_add(self.body_size())
    }

    /// Get message size in bytes.
    ///
    /// Equivalent to [`Self::size_with_headers`] — preserved for backwards
    /// compatibility with existing callers (storage backends, IMAP
    /// `RFC822.SIZE`, JMAP `Email/get`, quota accounting). For body-only
    /// counts use [`Self::body_size`] explicitly.
    pub fn size(&self) -> usize {
        self.size_with_headers()
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

    /// Parse multipart message into parts.
    ///
    /// For `Large` bodies, the body is read into memory first (acceptable
    /// since multipart messages are rarely single-attachment multi-MB bodies).
    pub async fn parse_multipart(&self) -> crate::error::Result<Vec<crate::mime::MimePart>> {
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

        let bytes = match &self.body {
            MessageBody::Small(bytes) => bytes.clone(),
            MessageBody::Large(large) => large.read_to_bytes().await?,
        };
        crate::mime::split_multipart(&bytes, boundary)
    }

    /// Decode message body according to Content-Transfer-Encoding.
    ///
    /// For `Large` bodies, the body is read into memory before decoding.
    pub async fn decode_body(&self) -> crate::error::Result<Vec<u8>> {
        let encoding = self.content_transfer_encoding();

        let bytes = match &self.body {
            MessageBody::Small(bytes) => bytes.clone(),
            MessageBody::Large(large) => large.read_to_bytes().await?,
        };

        match encoding {
            crate::mime::ContentTransferEncoding::Base64 => crate::mime::decode_base64(&bytes),
            crate::mime::ContentTransferEncoding::QuotedPrintable => {
                crate::mime::decode_quoted_printable(&bytes)
            }
            _ => Ok(bytes.to_vec()),
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
        // body_size returns the body bytes only.
        assert_eq!(msg.body_size(), 13);
        // size() now reports headers + final CRLF + body. With no headers the
        // header block is just the empty-line separator (2 bytes).
        assert_eq!(msg.size(), 15);
        assert_eq!(msg.size(), msg.size_with_headers());
    }

    #[test]
    fn size_with_headers_known_message() {
        // Build a message with a deterministic single-header layout.
        // Expected on-wire form:
        //
        //   subject: Hello\r\n
        //   from: a@b\r\n
        //   \r\n
        //   Hi
        //
        // Bytes:
        //   "subject: Hello\r\n" = 7 + 2 + 5 + 2 = 16
        //   "from: a@b\r\n"      = 4 + 2 + 3 + 2 = 11
        //   "\r\n"               = 2
        //   body "Hi"            = 2
        //   total                = 31
        let mut headers = HeaderMap::new();
        headers.insert("subject", "Hello");
        headers.insert("from", "a@b");
        let body = MessageBody::Small(Bytes::from("Hi"));
        let msg = MimeMessage::new(headers, body);
        assert_eq!(msg.body_size(), 2);
        assert_eq!(msg.size_with_headers(), 31);
        assert_eq!(msg.size(), 31);
    }
}

#[cfg(test)]
mod large_body_tests {
    use super::*;
    use std::env::temp_dir;

    #[tokio::test]
    async fn test_largebody_from_path_roundtrip() {
        let mut path = temp_dir();
        path.push(format!("rusmes_test_large_{}.bin", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"hello streaming world")
            .await
            .unwrap();
        let large = LargeBody::from_path(&path).await.unwrap();
        assert_eq!(large.size(), 21);
        let data = large.read_to_bytes().await.unwrap();
        assert_eq!(&data[..], b"hello streaming world");
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn test_messagebody_threshold_chooses_small() {
        let mut path = temp_dir();
        path.push(format!("rusmes_test_small_{}.bin", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"tiny").await.unwrap();
        let body = MessageBody::from_path_with_threshold(&path, 1024)
            .await
            .unwrap();
        assert!(matches!(body, MessageBody::Small(_)));
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn test_messagebody_threshold_chooses_large() {
        let mut path = temp_dir();
        path.push(format!("rusmes_test_thresh_{}.bin", uuid::Uuid::new_v4()));
        // Write 2 KiB with threshold 1 KiB → Large
        let data = vec![0u8; 2048];
        tokio::fs::write(&path, &data).await.unwrap();
        let body = MessageBody::from_path_with_threshold(&path, 1024)
            .await
            .unwrap();
        assert!(matches!(body, MessageBody::Large(_)));
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn test_largebody_extract_text() {
        let mut path = temp_dir();
        path.push(format!("rusmes_test_text_{}.txt", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"Hello, Large World!")
            .await
            .unwrap();
        let large = LargeBody::from_path(&path).await.unwrap();
        let msg = MimeMessage::new(HeaderMap::new(), MessageBody::Large(large));
        let text = msg.extract_text().await.unwrap();
        assert_eq!(text, "Hello, Large World!");
        let _ = tokio::fs::remove_file(&path).await;
    }
}
