//! DKIM signature verification mailet

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hickory_resolver::TokioResolver;
use rusmes_proto::Mail;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// DKIM (DomainKeys Identified Mail) verification
pub struct DkimVerifyMailet {
    name: String,
    reject_on_fail: bool,
}

impl DkimVerifyMailet {
    /// Create a new DKIM verify mailet
    pub fn new() -> Self {
        Self {
            name: "DkimVerify".to_string(),
            reject_on_fail: false,
        }
    }
}

impl Default for DkimVerifyMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for DkimVerifyMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        if let Some(reject_str) = config.get_param("reject_on_fail") {
            self.reject_on_fail = reject_str.parse()?;
        }

        tracing::info!(
            "Initialized DkimVerifyMailet (reject on fail: {})",
            self.reject_on_fail
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        tracing::debug!("Verifying DKIM signature for mail {}", mail.id());

        let result = verify_dkim_signature(mail).await;

        match result {
            Ok(DkimResult::Pass) => {
                mail.set_attribute("dkim.result", "pass");
                mail.set_attribute("dkim.verified", true);
                tracing::info!("DKIM verification passed for mail {}", mail.id());
            }
            Ok(DkimResult::Fail(reason)) => {
                mail.set_attribute("dkim.result", "fail");
                mail.set_attribute("dkim.verified", false);
                tracing::warn!(
                    "DKIM verification failed for mail {}: {}",
                    mail.id(),
                    reason
                );

                if self.reject_on_fail {
                    return Ok(MailetAction::Drop);
                }
            }
            Ok(DkimResult::TempError(reason)) => {
                mail.set_attribute("dkim.result", "temperror");
                mail.set_attribute("dkim.verified", false);
                tracing::warn!(
                    "DKIM verification temp error for mail {}: {}",
                    mail.id(),
                    reason
                );
            }
            Ok(DkimResult::PermError(reason)) => {
                mail.set_attribute("dkim.result", "permerror");
                mail.set_attribute("dkim.verified", false);
                tracing::warn!(
                    "DKIM verification perm error for mail {}: {}",
                    mail.id(),
                    reason
                );
            }
            Ok(DkimResult::Neutral(reason)) => {
                mail.set_attribute("dkim.result", "neutral");
                mail.set_attribute("dkim.verified", false);
                tracing::info!(
                    "DKIM verification neutral for mail {}: {}",
                    mail.id(),
                    reason
                );
            }
            Ok(DkimResult::None) => {
                mail.set_attribute("dkim.result", "none");
                mail.set_attribute("dkim.verified", false);
                tracing::debug!("No DKIM signature found for mail {}", mail.id());
            }
            Err(e) => {
                mail.set_attribute("dkim.result", "temperror");
                mail.set_attribute("dkim.verified", false);
                tracing::error!("DKIM verification error for mail {}: {}", mail.id(), e);
            }
        }

        Ok(MailetAction::Continue)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// DKIM verification result
#[derive(Debug)]
enum DkimResult {
    Pass,
    Fail(&'static str),
    TempError(&'static str),
    PermError(&'static str),
    Neutral(&'static str),
    None,
}

/// DKIM-Signature header parsed
#[derive(Debug)]
struct DkimSignature {
    version: u32,
    algorithm: String,
    domain: String,
    selector: String,
    signed_headers: Vec<String>,
    body_hash: String,
    signature: String,
    canonicalization: (String, String), // (header, body)
}

/// Verify DKIM signature on a mail message
async fn verify_dkim_signature(mail: &Mail) -> anyhow::Result<DkimResult> {
    // 1. Parse DKIM-Signature header
    let dkim_header = match parse_dkim_signature_header(mail) {
        Some(header) => header,
        None => return Ok(DkimResult::None),
    };

    let dkim_sig = match parse_dkim_signature(&dkim_header) {
        Ok(sig) => sig,
        Err(e) => {
            tracing::warn!("Failed to parse DKIM signature: {}", e);
            return Ok(DkimResult::PermError("Invalid signature format"));
        }
    };

    // 2. DNS lookup for public key
    let dns_name = format!("{}._domainkey.{}", dkim_sig.selector, dkim_sig.domain);

    let resolver = match TokioResolver::builder_tokio().map(|b| b.build()) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to create DNS resolver: {}", e);
            return Ok(DkimResult::TempError("DNS resolver error"));
        }
    };

    let txt_records = match resolver.txt_lookup(&dns_name).await {
        Ok(records) => records,
        Err(e) => {
            tracing::warn!("DNS lookup failed for {}: {}", dns_name, e);
            return Ok(DkimResult::TempError("DNS lookup failed"));
        }
    };

    // 3. Parse public key from DNS TXT record
    let public_key_record = match parse_dkim_txt_record(&txt_records) {
        Ok(record) => record,
        Err(e) => {
            tracing::warn!("Failed to parse DKIM TXT record: {}", e);
            return Ok(DkimResult::PermError("Invalid public key record"));
        }
    };

    // 4. Get message data for canonicalization
    let message_data = get_message_raw_data(mail);

    // 5. Canonicalize headers and body
    let canonical_headers =
        match canonicalize_headers(&message_data, &dkim_sig, &dkim_sig.canonicalization.0) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("Header canonicalization failed: {}", e);
                return Ok(DkimResult::PermError("Canonicalization error"));
            }
        };

    let canonical_body = match canonicalize_body(&message_data, &dkim_sig.canonicalization.1) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("Body canonicalization failed: {}", e);
            return Ok(DkimResult::PermError("Canonicalization error"));
        }
    };

    // 6. Verify body hash
    let mut hasher = Sha256::new();
    hasher.update(&canonical_body);
    let body_hash = hasher.finalize();

    let expected_bh = match BASE64.decode(dkim_sig.body_hash.as_bytes()) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("Failed to decode body hash: {}", e);
            return Ok(DkimResult::PermError("Invalid body hash encoding"));
        }
    };

    if body_hash.as_slice() != expected_bh.as_slice() {
        tracing::warn!("Body hash mismatch");
        return Ok(DkimResult::Fail("Body hash mismatch"));
    }

    // 7. Verify signature
    let signature_bytes = match BASE64.decode(dkim_sig.signature.as_bytes()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to decode signature: {}", e);
            return Ok(DkimResult::PermError("Invalid signature encoding"));
        }
    };

    match dkim_sig.algorithm.as_str() {
        "rsa-sha256" => {
            match verify_rsa_signature(
                &public_key_record.public_key,
                &canonical_headers,
                &signature_bytes,
            ) {
                Ok(true) => Ok(DkimResult::Pass),
                Ok(false) => Ok(DkimResult::Fail("RSA signature verification failed")),
                Err(e) => {
                    tracing::warn!("RSA verification error: {}", e);
                    Ok(DkimResult::PermError("RSA verification error"))
                }
            }
        }
        "ed25519-sha256" => {
            match verify_ed25519_signature(
                &public_key_record.public_key,
                &canonical_headers,
                &signature_bytes,
            ) {
                Ok(true) => Ok(DkimResult::Pass),
                Ok(false) => Ok(DkimResult::Fail("Ed25519 signature verification failed")),
                Err(e) => {
                    tracing::warn!("Ed25519 verification error: {}", e);
                    Ok(DkimResult::PermError("Ed25519 verification error"))
                }
            }
        }
        _ => {
            tracing::warn!("Unknown algorithm: {}", dkim_sig.algorithm);
            Ok(DkimResult::Neutral("Unknown algorithm"))
        }
    }
}

/// Extract DKIM-Signature header from mail
fn parse_dkim_signature_header(mail: &Mail) -> Option<String> {
    mail.message()
        .headers()
        .get_first("dkim-signature")
        .map(|s| s.to_string())
}

/// Parse DKIM-Signature header into structured data
fn parse_dkim_signature(header: &str) -> anyhow::Result<DkimSignature> {
    let mut params: HashMap<String, String> = HashMap::new();

    // Unfold header first
    let unfolded = rusmes_proto::message::HeaderMap::unfold_value(header);

    // Parse tag=value pairs
    for part in unfolded.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some(eq_pos) = part.find('=') {
            let key = part[..eq_pos].trim().to_string();
            let value = part[eq_pos + 1..].trim().to_string();
            params.insert(key, value);
        }
    }

    // Extract required fields
    let version = params
        .get("v")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1);

    let algorithm = params
        .get("a")
        .ok_or_else(|| anyhow::anyhow!("Missing algorithm (a=)"))?
        .clone();

    let domain = params
        .get("d")
        .ok_or_else(|| anyhow::anyhow!("Missing domain (d=)"))?
        .clone();

    let selector = params
        .get("s")
        .ok_or_else(|| anyhow::anyhow!("Missing selector (s=)"))?
        .clone();

    let signed_headers = params
        .get("h")
        .ok_or_else(|| anyhow::anyhow!("Missing signed headers (h=)"))?
        .split(':')
        .map(|s| s.trim().to_lowercase())
        .collect();

    let body_hash = params
        .get("bh")
        .ok_or_else(|| anyhow::anyhow!("Missing body hash (bh=)"))?
        .clone();

    let signature = params
        .get("b")
        .ok_or_else(|| anyhow::anyhow!("Missing signature (b=)"))?
        .clone();

    // Parse canonicalization (default: simple/simple)
    let canonicalization = params
        .get("c")
        .map(|c| {
            let parts: Vec<&str> = c.split('/').collect();
            if parts.len() == 2 {
                (parts[0].to_string(), parts[1].to_string())
            } else {
                (parts[0].to_string(), "simple".to_string())
            }
        })
        .unwrap_or_else(|| ("simple".to_string(), "simple".to_string()));

    Ok(DkimSignature {
        version,
        algorithm,
        domain,
        selector,
        signed_headers,
        body_hash,
        signature,
        canonicalization,
    })
}

/// DKIM public key record
struct DkimPublicKey {
    #[allow(dead_code)]
    key_type: String,
    public_key: Vec<u8>,
}

/// Parse DKIM public key from DNS TXT record
fn parse_dkim_txt_record(
    records: &hickory_resolver::lookup::TxtLookup,
) -> anyhow::Result<DkimPublicKey> {
    // Concatenate all TXT record strings
    let mut record_data = String::new();
    for record in records.iter() {
        for data in record.iter() {
            record_data.push_str(&String::from_utf8_lossy(data));
        }
    }

    // Parse tag=value pairs
    let mut params: HashMap<String, String> = HashMap::new();
    for part in record_data.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some(eq_pos) = part.find('=') {
            let key = part[..eq_pos].trim().to_string();
            let value = part[eq_pos + 1..].trim().to_string();
            params.insert(key, value);
        }
    }

    let key_type = params
        .get("k")
        .cloned()
        .unwrap_or_else(|| "rsa".to_string());

    let public_key_b64 = params
        .get("p")
        .ok_or_else(|| anyhow::anyhow!("Missing public key (p=)"))?;

    // Decode base64 public key
    let public_key = BASE64
        .decode(public_key_b64.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to decode public key: {}", e))?;

    Ok(DkimPublicKey {
        key_type,
        public_key,
    })
}

/// Get raw message data for canonicalization
fn get_message_raw_data(mail: &Mail) -> Vec<u8> {
    // Reconstruct headers
    let mut data = Vec::new();

    for (name, values) in mail.message().headers().iter() {
        for value in values {
            data.extend_from_slice(name.as_bytes());
            data.extend_from_slice(b": ");
            data.extend_from_slice(value.as_bytes());
            data.extend_from_slice(b"\r\n");
        }
    }

    // Empty line between headers and body
    data.extend_from_slice(b"\r\n");

    // Body
    if let rusmes_proto::MessageBody::Small(body_bytes) = mail.message().body() {
        data.extend_from_slice(body_bytes);
    }

    data
}

/// Canonicalize headers per RFC 6376
fn canonicalize_headers(
    message_data: &[u8],
    dkim_sig: &DkimSignature,
    method: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut result = Vec::new();

    // Parse headers from message
    let (headers, _) = rusmes_proto::mime::parse_headers(message_data)?;

    // Process each signed header
    for header_name in &dkim_sig.signed_headers {
        if let Some(value) = headers.get(header_name) {
            match method {
                "relaxed" => {
                    // Relaxed canonicalization:
                    // - Convert header name to lowercase
                    // - Unfold header value
                    // - Convert all whitespace to single space
                    // - Remove leading/trailing whitespace from value
                    result.extend_from_slice(header_name.to_lowercase().as_bytes());
                    result.extend_from_slice(b":");

                    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
                    result.extend_from_slice(cleaned.as_bytes());
                    result.extend_from_slice(b"\r\n");
                }
                _ => {
                    // Simple canonicalization (default): headers as-is
                    result.extend_from_slice(header_name.as_bytes());
                    result.extend_from_slice(b": ");
                    result.extend_from_slice(value.as_bytes());
                    result.extend_from_slice(b"\r\n");
                }
            }
        }
    }

    // Add DKIM-Signature header itself (with b= value removed)
    if method == "relaxed" {
        result.extend_from_slice(b"dkim-signature:");
    } else {
        result.extend_from_slice(b"DKIM-Signature: ");
    }

    // Reconstruct DKIM-Signature without the signature value
    let mut sig_parts = Vec::new();
    sig_parts.push(format!("v={}", dkim_sig.version));
    sig_parts.push(format!("a={}", dkim_sig.algorithm));
    sig_parts.push(format!("d={}", dkim_sig.domain));
    sig_parts.push(format!("s={}", dkim_sig.selector));
    sig_parts.push(format!(
        "c={}/{}",
        dkim_sig.canonicalization.0, dkim_sig.canonicalization.1
    ));
    sig_parts.push(format!("h={}", dkim_sig.signed_headers.join(":")));
    sig_parts.push(format!("bh={}", dkim_sig.body_hash));
    sig_parts.push("b=".to_string());

    let sig_value = sig_parts.join("; ");

    if method == "relaxed" {
        let cleaned = sig_value.split_whitespace().collect::<Vec<_>>().join(" ");
        result.extend_from_slice(cleaned.as_bytes());
    } else {
        result.extend_from_slice(sig_value.as_bytes());
    }

    Ok(result)
}

/// Canonicalize body per RFC 6376
fn canonicalize_body(message_data: &[u8], method: &str) -> anyhow::Result<Vec<u8>> {
    // Find body start (after headers)
    let (_, body_offset) = rusmes_proto::mime::parse_headers(message_data)?;

    if body_offset >= message_data.len() {
        return Ok(Vec::new());
    }

    let body = &message_data[body_offset..];

    match method {
        "relaxed" => {
            // Relaxed body canonicalization:
            // - Ignore all whitespace at end of lines
            // - Reduce all sequences of whitespace to single space
            // - Ignore all empty lines at end of body
            let mut result = Vec::new();
            let lines = body.split(|&b| b == b'\n');
            let mut line_vec: Vec<&[u8]> = lines.collect();

            // Remove trailing empty lines
            while let Some(last) = line_vec.last() {
                let trimmed = last
                    .iter()
                    .filter(|&&b| b != b'\r' && b != b' ' && b != b'\t')
                    .count();
                if trimmed == 0 {
                    line_vec.pop();
                } else {
                    break;
                }
            }

            for line in line_vec {
                // Remove trailing whitespace
                let mut end = line.len();
                while end > 0
                    && (line[end - 1] == b' ' || line[end - 1] == b'\t' || line[end - 1] == b'\r')
                {
                    end -= 1;
                }

                // Reduce multiple spaces to single space
                let mut prev_was_space = false;
                for &byte in line.iter().take(end) {
                    if byte == b' ' || byte == b'\t' {
                        if !prev_was_space {
                            result.push(b' ');
                            prev_was_space = true;
                        }
                    } else {
                        result.push(byte);
                        prev_was_space = false;
                    }
                }

                result.extend_from_slice(b"\r\n");
            }

            Ok(result)
        }
        _ => {
            // Simple body canonicalization (default):
            // - Ignore all empty lines at end of body
            let mut result = body.to_vec();

            // Remove trailing CRLF sequences
            while result.len() >= 2 {
                let len = result.len();
                if result[len - 2] == b'\r' && result[len - 1] == b'\n' {
                    result.truncate(len - 2);
                } else if result[len - 1] == b'\n' {
                    result.truncate(len - 1);
                } else {
                    break;
                }
            }

            // Ensure exactly one trailing CRLF
            result.extend_from_slice(b"\r\n");

            Ok(result)
        }
    }
}

/// Verify RSA signature
fn verify_rsa_signature(
    public_key_der: &[u8],
    data: &[u8],
    signature: &[u8],
) -> anyhow::Result<bool> {
    use rsa::pkcs1::DecodeRsaPublicKey;
    use rsa::pkcs8::DecodePublicKey;
    use rsa::RsaPublicKey;

    // Try to decode as PKCS#8 first, then PKCS#1
    let public_key = if let Ok(key) = RsaPublicKey::from_public_key_der(public_key_der) {
        key
    } else {
        // Try PKCS#1
        RsaPublicKey::from_pkcs1_der(public_key_der)
            .map_err(|e| anyhow::anyhow!("Failed to parse RSA public key: {}", e))?
    };

    // Hash the data using SHA-256
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();

    // Verify signature using PKCS#1 v1.5 padding scheme
    // We need to manually verify since we're dealing with raw signatures
    let padding = rsa::Pkcs1v15Sign::new_unprefixed();

    match public_key.verify(padding, &hash, signature) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Verify Ed25519 signature
fn verify_ed25519_signature(
    public_key_bytes: &[u8],
    data: &[u8],
    signature_bytes: &[u8],
) -> anyhow::Result<bool> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    if public_key_bytes.len() != 32 {
        return Err(anyhow::anyhow!("Invalid Ed25519 public key length"));
    }

    if signature_bytes.len() != 64 {
        return Err(anyhow::anyhow!("Invalid Ed25519 signature length"));
    }

    let public_key = VerifyingKey::from_bytes(
        public_key_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to convert public key"))?,
    )
    .map_err(|e| anyhow::anyhow!("Failed to parse Ed25519 public key: {}", e))?;

    let signature = Signature::from_bytes(
        signature_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to convert signature"))?,
    );

    match public_key.verify(data, &signature) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
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

    fn create_test_mail_with_headers(
        sender: &str,
        recipients: Vec<&str>,
        headers: Vec<(&str, &str)>,
    ) -> Mail {
        let sender_addr = MailAddress::from_str(sender).ok();
        let recipient_addrs: Vec<MailAddress> = recipients
            .iter()
            .filter_map(|r| MailAddress::from_str(r).ok())
            .collect();

        let mut header_map = HeaderMap::new();
        for (name, value) in headers {
            header_map.insert(name.to_string(), value.to_string());
        }

        let message = MimeMessage::new(header_map, MessageBody::Small(Bytes::from("Test message")));

        Mail::new(sender_addr, recipient_addrs, message, None, None)
    }

    #[tokio::test]
    async fn test_dkim_verify_mailet_creation() {
        let mailet = DkimVerifyMailet::new();
        assert_eq!(mailet.name(), "DkimVerify");
        assert!(!mailet.reject_on_fail);
    }

    #[tokio::test]
    async fn test_dkim_verify_mailet_default() {
        let mailet = DkimVerifyMailet::default();
        assert_eq!(mailet.name(), "DkimVerify");
    }

    #[tokio::test]
    async fn test_dkim_verify_init_with_config() {
        let mut mailet = DkimVerifyMailet::new();
        let mut config = MailetConfig::new("DkimVerify");
        config = config.with_param("reject_on_fail".to_string(), "true".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert!(mailet.reject_on_fail);
    }

    #[tokio::test]
    async fn test_dkim_verify_init_invalid_config() {
        let mut mailet = DkimVerifyMailet::new();
        let mut config = MailetConfig::new("DkimVerify");
        config = config.with_param("reject_on_fail".to_string(), "not_a_bool".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_dkim_verify_no_signature() {
        let mailet = DkimVerifyMailet::new();
        let mut mail = create_test_mail("sender@example.com", vec!["recipient@test.com"]);

        let action = mailet.service(&mut mail).await.unwrap();
        assert!(matches!(action, MailetAction::Continue));

        let dkim_result = mail.get_attribute("dkim.result").and_then(|v| v.as_str());
        assert_eq!(dkim_result, Some("none"));

        let dkim_verified = mail
            .get_attribute("dkim.verified")
            .and_then(|v| v.as_bool());
        assert_eq!(dkim_verified, Some(false));
    }

    #[test]
    fn test_parse_dkim_signature_header_none() {
        let mail = create_test_mail("sender@example.com", vec!["recipient@test.com"]);
        let header = parse_dkim_signature_header(&mail);
        assert!(header.is_none());
    }

    #[test]
    fn test_parse_dkim_signature_header_exists() {
        let mail = create_test_mail_with_headers(
            "sender@example.com",
            vec!["recipient@test.com"],
            vec![(
                "DKIM-Signature",
                "v=1; a=rsa-sha256; d=example.com; s=selector; h=from:to:subject; bh=test; b=signature",
            )],
        );
        let header = parse_dkim_signature_header(&mail);
        assert!(header.is_some());
    }

    #[test]
    fn test_parse_dkim_signature_simple() {
        let header = "v=1; a=rsa-sha256; d=example.com; s=selector; h=from:to:subject; bh=dGVzdA==; b=c2lnbmF0dXJl";
        let signature = parse_dkim_signature(header).unwrap();

        assert_eq!(signature.version, 1);
        assert_eq!(signature.algorithm, "rsa-sha256");
        assert_eq!(signature.domain, "example.com");
        assert_eq!(signature.selector, "selector");
        assert_eq!(signature.signed_headers, vec!["from", "to", "subject"]);
        assert_eq!(signature.body_hash, "dGVzdA==");
        assert_eq!(signature.signature, "c2lnbmF0dXJl");
        assert_eq!(signature.canonicalization.0, "simple");
        assert_eq!(signature.canonicalization.1, "simple");
    }

    #[test]
    fn test_parse_dkim_signature_with_canonicalization() {
        let header = "v=1; a=rsa-sha256; c=relaxed/relaxed; d=example.com; s=selector; h=from:to; bh=dGVzdA==; b=c2ln";
        let signature = parse_dkim_signature(header).unwrap();

        assert_eq!(signature.canonicalization.0, "relaxed");
        assert_eq!(signature.canonicalization.1, "relaxed");
    }

    #[test]
    fn test_parse_dkim_signature_with_partial_canonicalization() {
        let header =
            "v=1; a=rsa-sha256; c=relaxed; d=example.com; s=selector; h=from; bh=dGVzdA==; b=c2ln";
        let signature = parse_dkim_signature(header).unwrap();

        assert_eq!(signature.canonicalization.0, "relaxed");
        assert_eq!(signature.canonicalization.1, "simple");
    }

    #[test]
    fn test_parse_dkim_signature_missing_algorithm() {
        let header = "v=1; d=example.com; s=selector; h=from; bh=test; b=sig";
        let result = parse_dkim_signature(header);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dkim_signature_missing_domain() {
        let header = "v=1; a=rsa-sha256; s=selector; h=from; bh=test; b=sig";
        let result = parse_dkim_signature(header);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dkim_signature_missing_selector() {
        let header = "v=1; a=rsa-sha256; d=example.com; h=from; bh=test; b=sig";
        let result = parse_dkim_signature(header);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dkim_signature_missing_headers() {
        let header = "v=1; a=rsa-sha256; d=example.com; s=selector; bh=test; b=sig";
        let result = parse_dkim_signature(header);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dkim_signature_missing_body_hash() {
        let header = "v=1; a=rsa-sha256; d=example.com; s=selector; h=from; b=sig";
        let result = parse_dkim_signature(header);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dkim_signature_missing_signature() {
        let header = "v=1; a=rsa-sha256; d=example.com; s=selector; h=from; bh=test";
        let result = parse_dkim_signature(header);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_dkim_signature_default_version() {
        let header = "a=rsa-sha256; d=example.com; s=selector; h=from; bh=dGVzdA==; b=c2ln";
        let signature = parse_dkim_signature(header).unwrap();
        assert_eq!(signature.version, 1);
    }

    #[test]
    fn test_parse_dkim_signature_with_whitespace() {
        let header =
            "v=1 ;  a=rsa-sha256  ; d=example.com ; s=selector ; h=from:to ; bh=dGVzdA== ; b=c2ln";
        let signature = parse_dkim_signature(header).unwrap();

        assert_eq!(signature.algorithm, "rsa-sha256");
        assert_eq!(signature.domain, "example.com");
    }

    #[test]
    fn test_parse_dkim_signature_ed25519() {
        let header =
            "v=1; a=ed25519-sha256; d=example.com; s=selector; h=from; bh=dGVzdA==; b=c2ln";
        let signature = parse_dkim_signature(header).unwrap();

        assert_eq!(signature.algorithm, "ed25519-sha256");
    }

    #[test]
    fn test_get_message_raw_data() {
        let mail = create_test_mail_with_headers(
            "sender@example.com",
            vec!["recipient@test.com"],
            vec![("From", "sender@example.com"), ("To", "recipient@test.com")],
        );

        let raw_data = get_message_raw_data(&mail);
        assert!(!raw_data.is_empty());

        // Should contain headers (HeaderMap stores names in lowercase)
        let data_str = String::from_utf8_lossy(&raw_data);
        assert!(data_str.contains("from:"));
        assert!(data_str.contains("to:"));
    }

    #[test]
    fn test_canonicalize_body_simple() {
        // Email format: headers, then \r\n\r\n, then body
        let message = b"\r\n\r\nLine 1\r\nLine 2\r\n\r\n\r\n";
        let result = canonicalize_body(message, "simple").unwrap();

        // Simple canonicalization removes trailing empty lines
        let result_str = String::from_utf8_lossy(&result);
        assert!(!result_str.ends_with("\r\n\r\n\r\n"));
    }

    #[test]
    fn test_canonicalize_body_relaxed() {
        // Email format: headers, then \r\n\r\n, then body
        let message = b"\r\n\r\nLine with  multiple   spaces\r\nAnother line  \r\n\r\n\r\n";
        let result = canonicalize_body(message, "relaxed").unwrap();

        // Relaxed canonicalization should reduce whitespace
        let result_str = String::from_utf8_lossy(&result);
        assert!(result_str.contains("Line with multiple spaces"));
    }

    #[test]
    fn test_canonicalize_body_empty() {
        let message = b"";
        let result = canonicalize_body(message, "simple").unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_verify_rsa_signature_invalid_key() {
        let bad_key = b"not a valid key";
        let data = b"test data";
        let signature = b"test signature";

        let result = verify_rsa_signature(bad_key, data, signature);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_ed25519_signature_invalid_key_length() {
        let bad_key = b"too short";
        let data = b"test data";
        let signature = &[0u8; 64];

        let result = verify_ed25519_signature(bad_key, data, signature);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_ed25519_signature_invalid_signature_length() {
        let public_key = &[0u8; 32];
        let data = b"test data";
        let bad_signature = b"too short";

        let result = verify_ed25519_signature(public_key, data, bad_signature);
        assert!(result.is_err());
    }
}
