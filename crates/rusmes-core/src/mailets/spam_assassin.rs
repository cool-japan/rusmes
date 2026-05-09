//! SpamAssassin integration mailet

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rusmes_proto::{Mail, MailState};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// SpamAssassin configuration
#[derive(Debug, Clone)]
pub struct SpamAssassinConfig {
    pub host: String,
    pub port: u16,
    pub timeout: Duration,
    pub threshold: f64,
}

impl Default for SpamAssassinConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 783,
            timeout: Duration::from_secs(30),
            threshold: 5.0,
        }
    }
}

/// Spam check result from spamd
#[derive(Debug)]
struct SpamResult {
    is_spam: bool,
    score: f64,
    threshold: f64,
    symbols: Vec<String>,
}

/// Convert MIME message to bytes for spamd
///
/// For `MessageBody::Large`, the body is read into memory before serialisation.
async fn message_to_bytes(mail: &Mail) -> Result<Vec<u8>> {
    let message = mail.message();
    let headers = message.headers();
    let body = message.body();

    let mut result = Vec::new();

    // Serialize headers
    for (name, values) in headers.iter() {
        for value in values {
            result.extend_from_slice(name.as_bytes());
            result.extend_from_slice(b": ");
            result.extend_from_slice(value.as_bytes());
            result.extend_from_slice(b"\r\n");
        }
    }

    // Empty line between headers and body
    result.extend_from_slice(b"\r\n");

    // Serialize body
    match body {
        rusmes_proto::MessageBody::Small(bytes) => {
            result.extend_from_slice(bytes);
        }
        rusmes_proto::MessageBody::Large(large) => {
            let bytes = large
                .read_to_bytes()
                .await
                .map_err(|e| anyhow!("Failed to read large message body for spam check: {e}"))?;
            result.extend_from_slice(&bytes);
        }
    }

    Ok(result)
}

/// Check spam using SYMBOLS command (returns spam score and matched rules)
async fn check_spam(message: &[u8], config: &SpamAssassinConfig) -> Result<SpamResult> {
    let addr = format!("{}:{}", config.host, config.port);
    let stream = tokio::time::timeout(config.timeout, TcpStream::connect(&addr))
        .await
        .map_err(|_| anyhow!("Connection timeout to spamd at {}", addr))??;

    let mut stream = stream;

    // Send SYMBOLS request (returns spam score and matched rules)
    let request = format!(
        "SYMBOLS SPAMC/1.2\r\nContent-length: {}\r\n\r\n",
        message.len()
    );

    stream.write_all(request.as_bytes()).await?;
    stream.write_all(message).await?;
    stream.flush().await?;

    // Read response headers
    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader.read_line(&mut response_line).await?;

    // Parse status line: "SPAMD/1.1 0 EX_OK"
    if !response_line.contains("EX_OK") {
        return Err(anyhow!("spamd error: {}", response_line.trim()));
    }

    // Parse headers
    let mut spam_score = 0.0;
    let mut spam_threshold = config.threshold;
    let mut is_spam = false;
    let mut _content_length = 0usize;

    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        if line.trim().is_empty() {
            break; // End of headers
        }

        let line = line.trim();

        if let Some(value) = line.strip_prefix("Spam:") {
            // Format: "Spam: True ; 5.2 / 5.0"
            let parts: Vec<&str> = value.split(';').collect();
            is_spam = parts
                .first()
                .map(|s| s.trim().starts_with("True"))
                .unwrap_or(false);

            // Parse score and threshold
            if parts.len() > 1 {
                if let Some((score_str, threshold_str)) = parts[1].split_once('/') {
                    if let Ok(score) = score_str.trim().parse::<f64>() {
                        spam_score = score;
                    }
                    if let Ok(thresh) = threshold_str.trim().parse::<f64>() {
                        spam_threshold = thresh;
                    }
                }
            }
        } else if let Some(value) = line.strip_prefix("Content-length:") {
            _content_length = value.trim().parse().unwrap_or(0);
        }
    }

    // Read symbols (remaining body)
    let mut symbols_line = String::new();
    reader.read_line(&mut symbols_line).await?;

    let symbols: Vec<String> = if !symbols_line.trim().is_empty() {
        symbols_line
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    };

    Ok(SpamResult {
        is_spam,
        score: spam_score,
        threshold: spam_threshold,
        symbols,
    })
}

/// SpamAssassin spam detection mailet
pub struct SpamAssassinMailet {
    name: String,
    config: SpamAssassinConfig,
    reject_spam: bool,
}

impl SpamAssassinMailet {
    /// Create a new SpamAssassin mailet
    pub fn new() -> Self {
        Self {
            name: "SpamAssassin".to_string(),
            config: SpamAssassinConfig::default(),
            reject_spam: false,
        }
    }
}

impl Default for SpamAssassinMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for SpamAssassinMailet {
    async fn init(&mut self, config: MailetConfig) -> Result<()> {
        // Parse configuration
        if let Some(host) = config.get_param("host") {
            self.config.host = host.to_string();
        }

        if let Some(port_str) = config.get_param("port") {
            self.config.port = port_str.parse()?;
        }

        if let Some(threshold_str) = config.get_param("threshold") {
            self.config.threshold = threshold_str.parse()?;
        }

        if let Some(timeout_str) = config.get_param("timeout") {
            let timeout_secs: u64 = timeout_str.parse()?;
            self.config.timeout = Duration::from_secs(timeout_secs);
        }

        if let Some(reject_str) = config.get_param("reject_spam") {
            self.reject_spam = reject_str.parse::<bool>().unwrap_or(false);
        }

        tracing::info!(
            "Initialized SpamAssassinMailet: host={}:{}, threshold={}, reject_spam={}",
            self.config.host,
            self.config.port,
            self.config.threshold,
            self.reject_spam
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> Result<MailetAction> {
        tracing::debug!("Scanning mail {} for spam", mail.id());

        // Extract message content
        let message_bytes = match message_to_bytes(mail).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to serialize message {}: {}", mail.id(), e);
                mail.set_attribute("spam.check_error", e.to_string());
                return Ok(MailetAction::Continue); // Fail open
            }
        };

        // Check spam score
        let result = match check_spam(&message_bytes, &self.config).await {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("SpamAssassin check error for {}: {}", mail.id(), e);
                mail.set_attribute("spam.check_error", e.to_string());
                return Ok(MailetAction::Continue); // Fail open
            }
        };

        // Store results in mail attributes
        mail.set_attribute("spam.score", result.score);
        mail.set_attribute("spam.threshold", result.threshold);
        mail.set_attribute("spam.is_spam", result.is_spam);

        if !result.symbols.is_empty() {
            mail.set_attribute("spam.symbols", result.symbols.join(","));
        }

        if result.is_spam {
            tracing::warn!(
                "Spam detected in {}: score={:.2}/{:.2}, symbols={:?}",
                mail.id(),
                result.score,
                result.threshold,
                result.symbols
            );

            if self.reject_spam && result.score >= self.config.threshold {
                tracing::info!(
                    "Rejecting spam mail {} (score={:.2})",
                    mail.id(),
                    result.score
                );
                mail.state = MailState::Error;
            }
        } else {
            tracing::info!(
                "Spam check passed for {}: score={:.2}/{:.2}",
                mail.id(),
                result.score,
                result.threshold
            );
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
    async fn test_spam_assassin_mailet_creation() {
        let mailet = SpamAssassinMailet::new();
        assert_eq!(mailet.name(), "SpamAssassin");
        assert!(!mailet.reject_spam);
    }

    #[tokio::test]
    async fn test_spam_assassin_mailet_default() {
        let mailet = SpamAssassinMailet::default();
        assert_eq!(mailet.name(), "SpamAssassin");
    }

    #[tokio::test]
    async fn test_spam_assassin_config_default() {
        let config = SpamAssassinConfig::default();
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 783);
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.threshold, 5.0);
    }

    #[tokio::test]
    async fn test_spam_assassin_init_with_config() {
        let mut mailet = SpamAssassinMailet::new();
        let mut config = MailetConfig::new("SpamAssassin");
        config = config.with_param("host".to_string(), "spam.example.com".to_string());
        config = config.with_param("port".to_string(), "11333".to_string());
        config = config.with_param("threshold".to_string(), "7.5".to_string());
        config = config.with_param("timeout".to_string(), "60".to_string());
        config = config.with_param("reject_spam".to_string(), "true".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert_eq!(mailet.config.host, "spam.example.com");
        assert_eq!(mailet.config.port, 11333);
        assert_eq!(mailet.config.threshold, 7.5);
        assert_eq!(mailet.config.timeout, Duration::from_secs(60));
        assert!(mailet.reject_spam);
    }

    #[tokio::test]
    async fn test_spam_assassin_init_invalid_port() {
        let mut mailet = SpamAssassinMailet::new();
        let mut config = MailetConfig::new("SpamAssassin");
        config = config.with_param("port".to_string(), "not_a_number".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spam_assassin_init_invalid_threshold() {
        let mut mailet = SpamAssassinMailet::new();
        let mut config = MailetConfig::new("SpamAssassin");
        config = config.with_param("threshold".to_string(), "invalid".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spam_assassin_init_invalid_timeout() {
        let mut mailet = SpamAssassinMailet::new();
        let mut config = MailetConfig::new("SpamAssassin");
        config = config.with_param("timeout".to_string(), "invalid".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_message_to_bytes_no_headers() {
        let mail = create_test_mail("sender@example.com", vec!["recipient@test.com"]);
        let result = message_to_bytes(&mail).await;
        assert!(result.is_ok());

        let bytes = result.unwrap();
        let message = String::from_utf8_lossy(&bytes);

        // With no headers, we still have one separator before body
        assert!(message.starts_with("\r\n"));
        assert!(message.contains("Test message"));
    }
}
