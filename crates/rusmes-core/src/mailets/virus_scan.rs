//! Virus scanning mailet (ClamAV integration)

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use rusmes_proto::{Mail, MailState};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};

/// ClamAV connection mode
#[derive(Debug, Clone)]
pub enum ClamAVMode {
    UnixSocket(PathBuf),
    Tcp { host: String, port: u16 },
}

/// ClamAV configuration
#[derive(Debug, Clone)]
pub struct ClamAVConfig {
    pub mode: ClamAVMode,
    pub timeout: Duration,
}

impl Default for ClamAVConfig {
    fn default() -> Self {
        Self {
            mode: ClamAVMode::UnixSocket(PathBuf::from("/var/run/clamav/clamd.sock")),
            timeout: Duration::from_secs(30),
        }
    }
}

/// Scan result from ClamAV
#[derive(Debug)]
pub enum ScanResult {
    Clean,
    Infected { virus_name: String },
    Error { message: String },
}

/// ClamAV virus scanning mailet
pub struct VirusScanMailet {
    name: String,
    config: ClamAVConfig,
    reject_on_virus: bool,
}

impl VirusScanMailet {
    /// Create a new virus scan mailet
    pub fn new() -> Self {
        Self {
            name: "VirusScan".to_string(),
            config: ClamAVConfig::default(),
            reject_on_virus: true,
        }
    }

    /// Convert message to bytes for scanning
    fn message_to_bytes(mail: &Mail) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Serialize headers
        let headers = mail.message().headers();
        for (name, values) in headers.iter() {
            for value in values {
                bytes.extend_from_slice(name.as_bytes());
                bytes.extend_from_slice(b": ");
                bytes.extend_from_slice(value.as_bytes());
                bytes.extend_from_slice(b"\r\n");
            }
        }

        // Empty line between headers and body
        bytes.extend_from_slice(b"\r\n");

        // Add body
        match mail.message().body() {
            rusmes_proto::MessageBody::Small(body_bytes) => {
                bytes.extend_from_slice(body_bytes);
            }
            rusmes_proto::MessageBody::Large(_) => {
                // For large messages, we'll just skip the body
                // This is a limitation of the current implementation
                tracing::warn!("Skipping large message body in virus scan");
            }
        }

        bytes
    }

    /// Connect to ClamAV daemon
    async fn connect_clamd(config: &ClamAVConfig) -> anyhow::Result<ClamAVStream> {
        match &config.mode {
            ClamAVMode::UnixSocket(path) => {
                let stream = UnixStream::connect(path).await?;
                Ok(ClamAVStream::Unix(stream))
            }
            ClamAVMode::Tcp { host, port } => {
                let stream = TcpStream::connect((host.as_str(), *port)).await?;
                Ok(ClamAVStream::Tcp(stream))
            }
        }
    }

    /// Scan message with ClamAV using INSTREAM protocol
    async fn scan_message(message: &[u8], config: &ClamAVConfig) -> anyhow::Result<ScanResult> {
        let mut stream = Self::connect_clamd(config).await?;

        // Send INSTREAM command
        stream.write_all(b"zINSTREAM\0").await?;

        // Send message in chunks
        const CHUNK_SIZE: usize = 2048;
        for chunk in message.chunks(CHUNK_SIZE) {
            // Send chunk size (4 bytes, network order)
            let len = (chunk.len() as u32).to_be_bytes();
            stream.write_all(&len).await?;

            // Send chunk data
            stream.write_all(chunk).await?;
        }

        // Send zero-length chunk to indicate end
        stream.write_all(&[0, 0, 0, 0]).await?;

        // Read response
        let mut response = String::new();
        stream.read_to_string(&mut response).await?;

        // Parse response
        Self::parse_clamd_response(&response)
    }

    /// Parse ClamAV daemon response
    fn parse_clamd_response(response: &str) -> anyhow::Result<ScanResult> {
        let response = response.trim();

        if response.ends_with("OK") {
            return Ok(ScanResult::Clean);
        }

        if response.contains("FOUND") {
            // Format: "stream: Eicar-Test-Signature FOUND"
            let parts: Vec<&str> = response.split_whitespace().collect();
            if parts.len() >= 2 {
                let virus_name = parts[1].to_string();
                return Ok(ScanResult::Infected { virus_name });
            }
        }

        if response.contains("ERROR") {
            return Ok(ScanResult::Error {
                message: response.to_string(),
            });
        }

        anyhow::bail!("Unknown clamd response: {}", response)
    }
}

impl Default for VirusScanMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for VirusScanMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        // Parse connection mode
        if let Some(mode_str) = config.get_param("mode") {
            match mode_str {
                "unix_socket" => {
                    let socket_path = config
                        .get_param("socket_path")
                        .unwrap_or("/var/run/clamav/clamd.sock");
                    self.config.mode = ClamAVMode::UnixSocket(PathBuf::from(socket_path));
                }
                "tcp" => {
                    let host = config.get_param("host").unwrap_or("localhost").to_string();
                    let port: u16 = config
                        .get_param("port")
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(3310);
                    self.config.mode = ClamAVMode::Tcp { host, port };
                }
                _ => {
                    anyhow::bail!("Invalid ClamAV mode: {}", mode_str);
                }
            }
        }

        // Parse timeout
        if let Some(timeout_str) = config.get_param("timeout") {
            if let Ok(timeout_secs) = timeout_str.parse::<u64>() {
                self.config.timeout = Duration::from_secs(timeout_secs);
            }
        }

        // Parse reject_on_virus
        if let Some(reject_str) = config.get_param("reject_on_virus") {
            self.reject_on_virus = reject_str.parse()?;
        }

        tracing::info!(
            "Initialized VirusScanMailet (mode: {:?}, reject on virus: {})",
            self.config.mode,
            self.reject_on_virus
        );
        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        tracing::debug!("Scanning mail {} for viruses", mail.id());

        // Extract message content
        let message_bytes = Self::message_to_bytes(mail);

        // Scan with ClamAV
        let result = match Self::scan_message(&message_bytes, &self.config).await {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("ClamAV scan error for {}: {}", mail.id(), e);
                mail.set_attribute("virus.scan_error", e.to_string());
                // Fail open - don't block mail if ClamAV is unavailable
                return Ok(MailetAction::Continue);
            }
        };

        match result {
            ScanResult::Clean => {
                mail.set_attribute("virus.result", "clean");
                tracing::info!("Virus scan clean for {}", mail.id());
            }
            ScanResult::Infected { virus_name } => {
                mail.set_attribute("virus.result", "infected");
                mail.set_attribute("virus.name", virus_name.clone());
                tracing::warn!("Virus detected in {}: {}", mail.id(), virus_name);

                if self.reject_on_virus {
                    mail.state = MailState::Ghost;
                }
            }
            ScanResult::Error { message } => {
                mail.set_attribute("virus.scan_error", message.clone());
                tracing::error!("ClamAV error for {}: {}", mail.id(), message);
            }
        }

        Ok(MailetAction::Continue)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Wrapper for Unix and TCP streams
enum ClamAVStream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl ClamAVStream {
    async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            ClamAVStream::Unix(stream) => stream.write_all(buf).await,
            ClamAVStream::Tcp(stream) => stream.write_all(buf).await,
        }
    }

    async fn read_to_string(&mut self, buf: &mut String) -> std::io::Result<usize> {
        match self {
            ClamAVStream::Unix(stream) => stream.read_to_string(buf).await,
            ClamAVStream::Tcp(stream) => stream.read_to_string(buf).await,
        }
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
    async fn test_virus_scan_mailet_creation() {
        let mailet = VirusScanMailet::new();
        assert_eq!(mailet.name(), "VirusScan");
        assert!(mailet.reject_on_virus);
    }

    #[tokio::test]
    async fn test_virus_scan_mailet_default() {
        let mailet = VirusScanMailet::default();
        assert_eq!(mailet.name(), "VirusScan");
    }

    #[tokio::test]
    async fn test_clamav_config_default() {
        let config = ClamAVConfig::default();
        assert!(matches!(config.mode, ClamAVMode::UnixSocket(_)));
        assert_eq!(config.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_virus_scan_init_unix_socket() {
        let mut mailet = VirusScanMailet::new();
        let config = MailetConfig::new("VirusScan")
            .with_param("mode".to_string(), "unix_socket".to_string())
            .with_param(
                "socket_path".to_string(),
                "/custom/path/clamd.sock".to_string(),
            );

        let result = mailet.init(config).await;
        assert!(result.is_ok());

        if let ClamAVMode::UnixSocket(path) = &mailet.config.mode {
            assert_eq!(path.to_str().unwrap(), "/custom/path/clamd.sock");
        } else {
            panic!("Expected UnixSocket mode");
        }
    }

    #[tokio::test]
    async fn test_virus_scan_init_tcp() {
        let mut mailet = VirusScanMailet::new();
        let config = MailetConfig::new("VirusScan")
            .with_param("mode".to_string(), "tcp".to_string())
            .with_param("host".to_string(), "clamav.example.com".to_string())
            .with_param("port".to_string(), "3310".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_ok());

        if let ClamAVMode::Tcp { host, port } = &mailet.config.mode {
            assert_eq!(host, "clamav.example.com");
            assert_eq!(*port, 3310);
        } else {
            panic!("Expected TCP mode");
        }
    }

    #[tokio::test]
    async fn test_virus_scan_init_tcp_default_port() {
        let mut mailet = VirusScanMailet::new();
        let config = MailetConfig::new("VirusScan")
            .with_param("mode".to_string(), "tcp".to_string())
            .with_param("host".to_string(), "clamav.example.com".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_ok());

        if let ClamAVMode::Tcp { host, port } = &mailet.config.mode {
            assert_eq!(host, "clamav.example.com");
            assert_eq!(*port, 3310);
        } else {
            panic!("Expected TCP mode");
        }
    }

    #[tokio::test]
    async fn test_virus_scan_init_invalid_mode() {
        let mut mailet = VirusScanMailet::new();
        let config = MailetConfig::new("VirusScan")
            .with_param("mode".to_string(), "invalid_mode".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_virus_scan_init_timeout() {
        let mut mailet = VirusScanMailet::new();
        let config =
            MailetConfig::new("VirusScan").with_param("timeout".to_string(), "60".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert_eq!(mailet.config.timeout, Duration::from_secs(60));
    }

    #[tokio::test]
    async fn test_virus_scan_init_reject_on_virus() {
        let mut mailet = VirusScanMailet::new();
        let config = MailetConfig::new("VirusScan")
            .with_param("reject_on_virus".to_string(), "false".to_string());

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert!(!mailet.reject_on_virus);
    }

    #[test]
    fn test_message_to_bytes_no_headers() {
        let mail = create_test_mail("sender@example.com", vec!["recipient@test.com"]);
        let bytes = VirusScanMailet::message_to_bytes(&mail);
        let message = String::from_utf8_lossy(&bytes);

        // With no headers, we still have one separator before body
        assert!(message.starts_with("\r\n"));
        assert!(message.contains("Test message"));
    }

    #[test]
    fn test_parse_clamd_response_clean() {
        let response = "stream: OK";
        let result = VirusScanMailet::parse_clamd_response(response).unwrap();

        assert!(matches!(result, ScanResult::Clean));
    }

    #[test]
    fn test_parse_clamd_response_infected() {
        let response = "stream: Eicar-Test-Signature FOUND";
        let result = VirusScanMailet::parse_clamd_response(response).unwrap();

        if let ScanResult::Infected { virus_name } = result {
            assert_eq!(virus_name, "Eicar-Test-Signature");
        } else {
            panic!("Expected Infected result");
        }
    }

    #[test]
    fn test_parse_clamd_response_error() {
        let response = "stream: ERROR";
        let result = VirusScanMailet::parse_clamd_response(response).unwrap();

        assert!(matches!(result, ScanResult::Error { .. }));
    }

    #[test]
    fn test_parse_clamd_response_unknown() {
        let response = "unknown response format";
        let result = VirusScanMailet::parse_clamd_response(response);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_clamd_response_with_whitespace() {
        let response = "  stream: OK  \n";
        let result = VirusScanMailet::parse_clamd_response(response).unwrap();

        assert!(matches!(result, ScanResult::Clean));
    }

    #[test]
    fn test_parse_clamd_response_different_virus() {
        let response = "stream: Win.Test.Malware FOUND";
        let result = VirusScanMailet::parse_clamd_response(response).unwrap();

        if let ScanResult::Infected { virus_name } = result {
            assert_eq!(virus_name, "Win.Test.Malware");
        } else {
            panic!("Expected Infected result");
        }
    }
}
