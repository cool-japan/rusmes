//! MIME parsing support for RFC 5322 and RFC 2045
//!
//! This module provides:
//! - Header folding/unfolding per RFC 5322
//! - Content-Transfer-Encoding decoding (base64, quoted-printable)
//! - MIME multipart parsing
//! - Content-Type header parsing

use crate::error::{MailError, Result};
use std::collections::HashMap;

/// Content transfer encoding types per RFC 2045
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentTransferEncoding {
    /// 7-bit ASCII
    SevenBit,
    /// 8-bit
    EightBit,
    /// Binary
    Binary,
    /// Quoted-printable
    QuotedPrintable,
    /// Base64
    Base64,
}

impl ContentTransferEncoding {
    /// Parse encoding from string
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "quoted-printable" => Self::QuotedPrintable,
            "base64" => Self::Base64,
            "8bit" => Self::EightBit,
            "binary" => Self::Binary,
            _ => Self::SevenBit,
        }
    }
}

/// Content-Type header parsed per RFC 2045
#[derive(Debug, Clone)]
pub struct ContentType {
    /// Main type (e.g., "text", "multipart")
    pub main_type: String,
    /// Sub type (e.g., "plain", "mixed")
    pub sub_type: String,
    /// Parameters (e.g., charset, boundary)
    pub parameters: HashMap<String, String>,
}

impl ContentType {
    /// Parse Content-Type header value
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();

        // Find the semicolon that separates type from parameters
        let (type_part, params_part) = if let Some(pos) = value.find(';') {
            (&value[..pos], &value[pos + 1..])
        } else {
            (value, "")
        };

        // Parse main/sub type
        let (main_type, sub_type) = if let Some(pos) = type_part.find('/') {
            let main = type_part[..pos].trim().to_lowercase();
            let sub = type_part[pos + 1..].trim().to_lowercase();
            (main, sub)
        } else {
            return Err(MailError::Parse(format!(
                "Invalid Content-Type format: {}",
                value
            )));
        };

        // Parse parameters
        let mut parameters = HashMap::new();
        for param in params_part.split(';') {
            let param = param.trim();
            if param.is_empty() {
                continue;
            }

            if let Some(pos) = param.find('=') {
                let key = param[..pos].trim().to_lowercase();
                let mut val = param[pos + 1..].trim();

                // Remove quotes if present
                if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
                    val = &val[1..val.len() - 1];
                }

                parameters.insert(key, val.to_string());
            }
        }

        Ok(ContentType {
            main_type,
            sub_type,
            parameters,
        })
    }

    /// Get boundary parameter for multipart messages
    pub fn boundary(&self) -> Option<&str> {
        self.parameters.get("boundary").map(|s| s.as_str())
    }

    /// Get charset parameter
    pub fn charset(&self) -> Option<&str> {
        self.parameters.get("charset").map(|s| s.as_str())
    }

    /// Check if this is a multipart type
    pub fn is_multipart(&self) -> bool {
        self.main_type == "multipart"
    }
}

/// A single part in a MIME multipart message
#[derive(Debug, Clone)]
pub struct MimePart {
    /// Headers for this part
    pub headers: HashMap<String, String>,
    /// Body content (raw bytes)
    pub body: Vec<u8>,
}

impl MimePart {
    /// Get Content-Type for this part
    pub fn content_type(&self) -> Result<Option<ContentType>> {
        if let Some(ct) = self.headers.get("content-type") {
            Ok(Some(ContentType::parse(ct)?))
        } else {
            Ok(None)
        }
    }

    /// Get Content-Transfer-Encoding for this part
    pub fn content_transfer_encoding(&self) -> ContentTransferEncoding {
        if let Some(cte) = self.headers.get("content-transfer-encoding") {
            ContentTransferEncoding::parse(cte.trim())
        } else {
            ContentTransferEncoding::SevenBit
        }
    }

    /// Decode the body according to Content-Transfer-Encoding
    pub fn decode_body(&self) -> Result<Vec<u8>> {
        let encoding = self.content_transfer_encoding();

        match encoding {
            ContentTransferEncoding::Base64 => decode_base64(&self.body),
            ContentTransferEncoding::QuotedPrintable => decode_quoted_printable(&self.body),
            _ => Ok(self.body.clone()),
        }
    }
}

/// Unfold headers per RFC 5322 section 2.2.3
///
/// Headers can be folded by inserting CRLF before whitespace.
/// This function removes the folding by replacing CRLF+whitespace with a single space.
pub fn unfold_header(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut prev_was_cr = false;
    let mut prev_was_lf = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                prev_was_cr = true;
                prev_was_lf = false;
            }
            '\n' => {
                if prev_was_cr {
                    prev_was_lf = true;
                    prev_was_cr = false;
                } else {
                    // LF without CR
                    prev_was_lf = true;
                }
            }
            ' ' | '\t' => {
                // If previous was CRLF or LF, this is a fold point
                if prev_was_lf {
                    // Skip all following whitespace and replace with single space
                    while let Some(&next_ch) = chars.peek() {
                        if next_ch == ' ' || next_ch == '\t' {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    result.push(' ');
                } else {
                    result.push(ch);
                }
                prev_was_cr = false;
                prev_was_lf = false;
            }
            _ => {
                prev_was_cr = false;
                prev_was_lf = false;
                result.push(ch);
            }
        }
    }

    result
}

/// Fold a header value per RFC 5322
///
/// Headers should not exceed 78 characters per line.
/// Folding is done by inserting CRLF before whitespace.
pub fn fold_header(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }

    let mut result = String::with_capacity(value.len() + value.len() / max_len * 3);
    let mut line_len = 0;
    let mut last_space = 0;
    let mut pending = String::new();

    for ch in value.chars() {
        pending.push(ch);
        line_len += 1;

        if ch == ' ' || ch == '\t' {
            last_space = pending.len();
        }

        if line_len >= max_len && last_space > 0 {
            // Fold at the last space
            result.push_str(&pending[..last_space]);
            result.push_str("\r\n ");
            pending = pending[last_space..].trim_start().to_string();
            line_len = pending.len();
            last_space = 0;
        }
    }

    result.push_str(&pending);
    result
}

/// Parse headers from raw message data
///
/// Handles header folding per RFC 5322.
/// Returns a map of header names (lowercase) to values.
pub fn parse_headers(data: &[u8]) -> Result<(HashMap<String, String>, usize)> {
    let mut headers = HashMap::new();
    let mut pos = 0;
    let mut current_header: Option<(String, String)> = None;

    let mut line_start = 0;
    let data_len = data.len();

    while pos < data_len {
        // Find line ending
        let line_end = if pos + 1 < data_len && data[pos] == b'\r' && data[pos + 1] == b'\n' {
            pos += 2;
            pos - 2
        } else if pos < data_len && data[pos] == b'\n' {
            pos += 1;
            pos - 1
        } else {
            pos += 1;
            continue;
        };

        let line = &data[line_start..line_end];

        // Empty line signals end of headers
        if line.is_empty() {
            if let Some((name, value)) = current_header.take() {
                headers.insert(name, unfold_header(&value));
            }
            break;
        }

        // Check if this is a continuation line (starts with whitespace)
        if !line.is_empty() && (line[0] == b' ' || line[0] == b'\t') {
            if let Some((_, ref mut value)) = current_header {
                let line_str = String::from_utf8_lossy(line);
                value.push_str(&line_str);
            }
        } else {
            // New header line
            if let Some((name, value)) = current_header.take() {
                headers.insert(name, unfold_header(&value));
            }

            // Parse header name and value
            if let Some(colon_pos) = line.iter().position(|&b| b == b':') {
                let name = String::from_utf8_lossy(&line[..colon_pos])
                    .trim()
                    .to_lowercase();
                let value = String::from_utf8_lossy(&line[colon_pos + 1..]).to_string();
                current_header = Some((name, value));
            }
        }

        line_start = pos;
    }

    // Don't forget the last header
    if let Some((name, value)) = current_header {
        headers.insert(name, unfold_header(&value));
    }

    Ok((headers, pos))
}

/// Split a multipart MIME message into its parts
pub fn split_multipart(body: &[u8], boundary: &str) -> Result<Vec<MimePart>> {
    let mut parts = Vec::new();

    // Construct boundary markers
    let start_boundary = format!("--{}", boundary);
    let end_boundary = format!("--{}--", boundary);

    let start_marker = start_boundary.as_bytes();
    let end_marker = end_boundary.as_bytes();

    let mut pos = 0;
    let body_len = body.len();

    // Find first boundary
    while pos < body_len {
        if body[pos..].starts_with(start_marker) {
            pos += start_marker.len();
            // Skip to end of line
            while pos < body_len && body[pos] != b'\n' {
                pos += 1;
            }
            if pos < body_len {
                pos += 1; // Skip the \n
            }
            break;
        }
        pos += 1;
    }

    // Parse each part
    loop {
        if pos >= body_len {
            break;
        }

        let part_start = pos;

        // Find next boundary
        let mut next_boundary_pos = None;
        let mut is_end = false;

        let mut search_pos = pos;
        while search_pos < body_len {
            if body[search_pos..].starts_with(end_marker) {
                next_boundary_pos = Some(search_pos);
                is_end = true;
                break;
            } else if body[search_pos..].starts_with(start_marker) {
                next_boundary_pos = Some(search_pos);
                break;
            }
            search_pos += 1;
        }

        if let Some(boundary_pos) = next_boundary_pos {
            // Extract part data (excluding the boundary)
            let part_data = &body[part_start..boundary_pos];

            // Parse headers and body for this part
            let (part_headers, headers_end) = parse_headers(part_data)?;
            let part_body = if headers_end < part_data.len() {
                part_data[headers_end..].to_vec()
            } else {
                Vec::new()
            };

            // Trim trailing CRLF from body
            let part_body = trim_trailing_crlf(&part_body);

            parts.push(MimePart {
                headers: part_headers,
                body: part_body,
            });

            if is_end {
                break;
            }

            // Move past boundary
            pos = boundary_pos + start_marker.len();
            while pos < body_len && body[pos] != b'\n' {
                pos += 1;
            }
            if pos < body_len {
                pos += 1;
            }
        } else {
            break;
        }
    }

    Ok(parts)
}

/// Trim trailing CRLF from a byte slice
fn trim_trailing_crlf(data: &[u8]) -> Vec<u8> {
    let mut end = data.len();

    while end > 0 {
        if end >= 2 && data[end - 2] == b'\r' && data[end - 1] == b'\n' {
            end -= 2;
        } else if end >= 1 && data[end - 1] == b'\n' {
            end -= 1;
        } else {
            break;
        }
    }

    data[..end].to_vec()
}

/// Decode Base64 content per RFC 2045
pub fn decode_base64(data: &[u8]) -> Result<Vec<u8>> {
    // Filter out whitespace and newlines as per RFC 2045
    let filtered: Vec<u8> = data
        .iter()
        .copied()
        .filter(|&b| !matches!(b, b'\r' | b'\n' | b' ' | b'\t'))
        .collect();

    // Simple base64 decoding implementation
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut decode_table = [255u8; 256];
    for (i, &ch) in alphabet.iter().enumerate() {
        decode_table[ch as usize] = i as u8;
    }

    let mut result = Vec::with_capacity(filtered.len() * 3 / 4);
    let mut i = 0;

    while i + 4 <= filtered.len() {
        let b0 = filtered[i];
        let b1 = filtered[i + 1];
        let b2 = filtered[i + 2];
        let b3 = filtered[i + 3];

        let v0 = decode_table[b0 as usize];
        let v1 = decode_table[b1 as usize];
        let v2 = if b2 == b'=' {
            0
        } else {
            decode_table[b2 as usize]
        };
        let v3 = if b3 == b'=' {
            0
        } else {
            decode_table[b3 as usize]
        };

        if v0 == 255 || v1 == 255 {
            return Err(MailError::Parse("Invalid base64 character".to_string()));
        }

        result.push((v0 << 2) | (v1 >> 4));

        if b2 != b'=' {
            result.push((v1 << 4) | (v2 >> 2));
        }

        if b3 != b'=' {
            result.push((v2 << 6) | v3);
        }

        i += 4;
    }

    Ok(result)
}

/// Decode quoted-printable content per RFC 2045
pub fn decode_quoted_printable(data: &[u8]) -> Result<Vec<u8>> {
    let mut result = Vec::with_capacity(data.len());
    let mut i = 0;

    while i < data.len() {
        if data[i] == b'=' {
            if i + 2 < data.len() {
                let c1 = data[i + 1];
                let c2 = data[i + 2];

                // Soft line break (=CRLF or =LF)
                if c1 == b'\r' && i + 3 < data.len() && data[i + 2] == b'\n' {
                    i += 3;
                    continue;
                } else if c1 == b'\n' {
                    i += 2;
                    continue;
                }

                // Hex encoded character
                if let (Some(h1), Some(h2)) = (hex_value(c1), hex_value(c2)) {
                    result.push((h1 << 4) | h2);
                    i += 3;
                    continue;
                }
            }

            // If we get here, it's a malformed sequence - pass through the '='
            result.push(b'=');
            i += 1;
        } else {
            result.push(data[i]);
            i += 1;
        }
    }

    Ok(result)
}

/// Convert a hex digit to its numeric value
fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Encode data as Base64
pub fn encode_base64(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;

    while i + 3 <= data.len() {
        let b0 = data[i];
        let b1 = data[i + 1];
        let b2 = data[i + 2];

        result.push(ALPHABET[(b0 >> 2) as usize] as char);
        result.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        result.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        result.push(ALPHABET[(b2 & 0x3f) as usize] as char);

        i += 3;
    }

    // Handle remaining bytes
    match data.len() - i {
        1 => {
            let b0 = data[i];
            result.push(ALPHABET[(b0 >> 2) as usize] as char);
            result.push(ALPHABET[((b0 & 0x03) << 4) as usize] as char);
            result.push_str("==");
        }
        2 => {
            let b0 = data[i];
            let b1 = data[i + 1];
            result.push(ALPHABET[(b0 >> 2) as usize] as char);
            result.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            result.push(ALPHABET[((b1 & 0x0f) << 2) as usize] as char);
            result.push('=');
        }
        _ => {}
    }

    result
}

/// Encode data as quoted-printable
pub fn encode_quoted_printable(data: &[u8]) -> String {
    let mut result = String::with_capacity(data.len());
    let mut line_len = 0;

    for &byte in data {
        // Characters that must be encoded
        let needs_encoding = !(33..=126).contains(&byte) || byte == b'=';

        if needs_encoding {
            let encoded = format!("={:02X}", byte);

            // Check if we need a soft line break
            if line_len + encoded.len() > 76 {
                result.push_str("=\r\n");
                line_len = 0;
            }

            result.push_str(&encoded);
            line_len += encoded.len();
        } else {
            // Check if we need a soft line break
            if line_len >= 76 {
                result.push_str("=\r\n");
                line_len = 0;
            }

            result.push(byte as char);
            line_len += 1;
        }
    }

    result
}
