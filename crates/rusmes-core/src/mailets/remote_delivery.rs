//! Remote SMTP delivery mailet with relay support

use crate::mailet::{Mailet, MailetAction, MailetConfig};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusmes_proto::{Mail, MailState};
use rustls::pki_types::ServerName;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// Relay configuration
#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub use_tls: bool,
    pub timeout: Duration,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 25,
            username: None,
            password: None,
            use_tls: false,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Build a rustls client config that accepts all certificates (for testing/internal use).
/// Production use should validate certificates properly.
fn build_tls_config() -> Arc<rustls::ClientConfig> {
    // Use a no-verifier config for relay connections (mirrors previous native-tls behavior
    // of `danger_accept_invalid_certs(true)`).  The connector is intentionally permissive
    // because SMTP relays often use self-signed certificates.
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();
    Arc::new(config)
}

/// A certificate verifier that accepts all certificates (mirrors `danger_accept_invalid_certs`).
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Delivers mail to remote SMTP servers via relay
pub struct RemoteDeliveryMailet {
    name: String,
    relay_config: Option<RelayConfig>,
}

impl RemoteDeliveryMailet {
    /// Create a new remote delivery mailet
    pub fn new() -> Self {
        Self {
            name: "RemoteDelivery".to_string(),
            relay_config: None,
        }
    }

    /// Serialize mail message to bytes
    fn serialize_message(mail: &Mail) -> Vec<u8> {
        let mut data = Vec::new();

        // Add headers
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

        // Add body
        match mail.message().body() {
            rusmes_proto::MessageBody::Small(body_bytes) => {
                data.extend_from_slice(body_bytes);
            }
            rusmes_proto::MessageBody::Large(_) => {
                tracing::warn!("Large message body not fully supported in relay");
            }
        }

        data
    }

    /// Send mail via SMTP relay
    async fn relay_via_smtp(mail: &Mail, relay: &RelayConfig) -> anyhow::Result<()> {
        let addr = format!("{}:{}", relay.host, relay.port);

        tracing::info!("Connecting to SMTP relay: {}", addr);

        let stream = match tokio::time::timeout(relay.timeout, TcpStream::connect(&addr)).await {
            Ok(Ok(s)) => {
                tracing::info!("TCP connection successful!");
                s
            }
            Ok(Err(e)) => {
                tracing::error!("TCP connection failed: {}", e);
                return Err(e.into());
            }
            Err(_) => {
                tracing::error!("TCP connection timed out after {:?}", relay.timeout);
                return Err(anyhow::anyhow!("Connection timeout"));
            }
        };

        tracing::info!("TCP connection established, setting up stream");

        let tls_config = build_tls_config();
        let connector = TlsConnector::from(tls_config);

        let mut stream: Box<dyn AsyncStream> = if relay.use_tls {
            // Use TLS from the start (SMTPS on port 465)
            let server_name = ServerName::try_from(relay.host.as_str())
                .map_err(|e| anyhow::anyhow!("Invalid server name '{}': {}", relay.host, e))?
                .to_owned();
            let tls_stream = connector.connect(server_name, stream).await?;
            Box::new(TlsStream(BufReader::new(tls_stream)))
        } else {
            Box::new(PlainStream(BufReader::new(stream)))
        };

        tracing::info!("Stream ready, reading greeting");

        // Read greeting
        let greeting = read_response(&mut stream).await?;
        tracing::info!("SMTP greeting received: {}", greeting.trim());

        // Send EHLO
        send_command(&mut stream, &format!("EHLO {}\r\n", relay.host)).await?;
        let ehlo_response = read_response(&mut stream).await?;
        tracing::debug!("EHLO response: {}", ehlo_response);

        // Upgrade to TLS via STARTTLS if not already using TLS
        if !relay.use_tls && ehlo_response.contains("STARTTLS") {
            send_command(&mut stream, "STARTTLS\r\n").await?;
            let _ = read_response(&mut stream).await?;

            // Upgrade connection
            let plain = match stream.into_plain() {
                Some(s) => s,
                None => anyhow::bail!("Cannot upgrade TLS connection"),
            };

            let server_name = ServerName::try_from(relay.host.as_str())
                .map_err(|e| anyhow::anyhow!("Invalid server name '{}': {}", relay.host, e))?
                .to_owned();
            let tls_config2 = build_tls_config();
            let connector2 = TlsConnector::from(tls_config2);
            let tls_stream = connector2.connect(server_name, plain).await?;
            stream = Box::new(TlsStream(BufReader::new(tls_stream)));

            // Send EHLO again after STARTTLS
            send_command(&mut stream, &format!("EHLO {}\r\n", relay.host)).await?;
            let _ = read_response(&mut stream).await?;
        }

        // Authenticate if credentials provided
        if let (Some(username), Some(password)) = (&relay.username, &relay.password) {
            send_command(&mut stream, "AUTH LOGIN\r\n").await?;
            let _ = read_response(&mut stream).await?;

            // Send base64-encoded username
            let username_b64 = BASE64.encode(username);
            send_command(&mut stream, &format!("{}\r\n", username_b64)).await?;
            let _ = read_response(&mut stream).await?;

            // Send base64-encoded password
            let password_b64 = BASE64.encode(password);
            send_command(&mut stream, &format!("{}\r\n", password_b64)).await?;
            let auth_response = read_response(&mut stream).await?;

            if !auth_response.starts_with("235") {
                anyhow::bail!("Authentication failed: {}", auth_response);
            }
            tracing::info!("SMTP authentication successful");
        }

        // Get sender
        let sender = mail
            .sender()
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "postmaster@localhost".to_string());

        // Send MAIL FROM
        send_command(&mut stream, &format!("MAIL FROM:<{}>\r\n", sender)).await?;
        let mail_response = read_response(&mut stream).await?;
        if !mail_response.starts_with("250") {
            anyhow::bail!("MAIL FROM failed: {}", mail_response);
        }

        // Send RCPT TO for each recipient
        let mut success_count = 0;
        for recipient in mail.recipients() {
            send_command(&mut stream, &format!("RCPT TO:<{}>\r\n", recipient)).await?;
            let rcpt_response = read_response(&mut stream).await?;

            if rcpt_response.starts_with("250") || rcpt_response.starts_with("251") {
                success_count += 1;
            } else {
                tracing::warn!("RCPT TO failed for {}: {}", recipient, rcpt_response);
            }
        }

        if success_count == 0 {
            anyhow::bail!("All recipients rejected");
        }

        // Send DATA
        send_command(&mut stream, "DATA\r\n").await?;
        let data_response = read_response(&mut stream).await?;
        if !data_response.starts_with("354") {
            anyhow::bail!("DATA command failed: {}", data_response);
        }

        // Send message content
        let message_data = Self::serialize_message(mail);
        stream.write_all(&message_data).await?;

        // End message with CRLF.CRLF
        if !message_data.ends_with(b"\r\n") {
            stream.write_all(b"\r\n").await?;
        }
        stream.write_all(b".\r\n").await?;
        stream.flush().await?;

        let send_response = read_response(&mut stream).await?;
        if !send_response.starts_with("250") {
            anyhow::bail!("Message send failed: {}", send_response);
        }

        tracing::info!(
            "Mail {} delivered via relay to {} recipients",
            mail.id(),
            success_count
        );

        // Send QUIT
        send_command(&mut stream, "QUIT\r\n").await?;
        let _ = read_response(&mut stream).await;

        Ok(())
    }
}

impl Default for RemoteDeliveryMailet {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Mailet for RemoteDeliveryMailet {
    async fn init(&mut self, config: MailetConfig) -> anyhow::Result<()> {
        // Read relay configuration from mailet config
        let relay_config = if let Some(host) = config.get_param("relay_host") {
            let port = config
                .get_param("relay_port")
                .and_then(|p| p.parse().ok())
                .unwrap_or(587);

            let username = config.get_param("relay_username").map(String::from);
            let password = config.get_param("relay_password").map(String::from);

            let use_tls = config
                .get_param("relay_use_tls")
                .and_then(|v| v.parse().ok())
                .unwrap_or(true);

            let timeout_secs = config
                .get_param("relay_timeout")
                .and_then(|t| t.parse().ok())
                .unwrap_or(30);

            Some(RelayConfig {
                host: host.to_string(),
                port,
                username,
                password,
                use_tls,
                timeout: Duration::from_secs(timeout_secs),
            })
        } else {
            None
        };

        self.relay_config = relay_config.clone();

        if let Some(relay) = &relay_config {
            tracing::info!(
                "Initialized RemoteDeliveryMailet with relay: {}:{} (TLS: {})",
                relay.host,
                relay.port,
                relay.use_tls
            );
        } else {
            tracing::info!("Initialized RemoteDeliveryMailet (no relay configured)");
        }

        Ok(())
    }

    async fn service(&self, mail: &mut Mail) -> anyhow::Result<MailetAction> {
        if mail.recipients().is_empty() {
            tracing::warn!("Mail {} has no recipients, dropping", mail.id());
            return Ok(MailetAction::ChangeState(MailState::Ghost));
        }

        // If relay is configured, use it
        if let Some(relay) = &self.relay_config {
            match Self::relay_via_smtp(mail, relay).await {
                Ok(_) => {
                    tracing::info!("Mail {} successfully relayed", mail.id());
                    Ok(MailetAction::ChangeState(MailState::Ghost))
                }
                Err(e) => {
                    tracing::error!("Failed to relay mail {}: {}", mail.id(), e);
                    mail.set_attribute("delivery.error", e.to_string());
                    Ok(MailetAction::ChangeState(MailState::Error))
                }
            }
        } else {
            tracing::warn!("No relay configured, dropping mail {}", mail.id());
            Ok(MailetAction::ChangeState(MailState::Ghost))
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// Helper trait for async stream operations
#[async_trait::async_trait]
trait AsyncStream: Send + Sync {
    async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()>;
    async fn flush(&mut self) -> std::io::Result<()>;
    async fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize>;
    fn into_plain(self: Box<Self>) -> Option<TcpStream>;
}

struct PlainStream(BufReader<TcpStream>);

#[async_trait::async_trait]
impl AsyncStream for PlainStream {
    async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.0.get_mut().write_all(buf).await
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        self.0.get_mut().flush().await
    }

    async fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        self.0.read_line(buf).await
    }

    fn into_plain(self: Box<Self>) -> Option<TcpStream> {
        Some(self.0.into_inner())
    }
}

struct TlsStream(BufReader<tokio_rustls::client::TlsStream<TcpStream>>);

#[async_trait::async_trait]
impl AsyncStream for TlsStream {
    async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.0.get_mut().write_all(buf).await
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        self.0.get_mut().flush().await
    }

    async fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        self.0.read_line(buf).await
    }

    fn into_plain(self: Box<Self>) -> Option<TcpStream> {
        // Cannot retrieve plain TcpStream from TLS stream after handshake
        None
    }
}

async fn send_command(stream: &mut Box<dyn AsyncStream>, cmd: &str) -> std::io::Result<()> {
    tracing::trace!("SMTP >>> {}", cmd.trim());
    stream.write_all(cmd.as_bytes()).await?;
    stream.flush().await
}

async fn read_response(stream: &mut Box<dyn AsyncStream>) -> std::io::Result<String> {
    let mut response = String::new();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).await?;

        tracing::trace!("SMTP <<< {}", line.trim());

        let is_last = line.len() >= 4 && &line[3..4] == " ";
        response.push_str(&line);

        if is_last {
            break;
        }
    }
    Ok(response)
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
    async fn test_remote_delivery_mailet_creation() {
        let mailet = RemoteDeliveryMailet::new();
        assert_eq!(mailet.name(), "RemoteDelivery");
        assert!(mailet.relay_config.is_none());
    }

    #[tokio::test]
    async fn test_remote_delivery_init_with_relay() {
        let mut mailet = RemoteDeliveryMailet::new();
        let config = MailetConfig::new("RemoteDelivery")
            .with_param("relay_host", "smtp.example.com")
            .with_param("relay_port", "587")
            .with_param("relay_username", "user@example.com")
            .with_param("relay_password", "password")
            .with_param("relay_use_tls", "true");

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert!(mailet.relay_config.is_some());

        let relay = mailet
            .relay_config
            .expect("relay config should be set after init");
        assert_eq!(relay.host, "smtp.example.com");
        assert_eq!(relay.port, 587);
        assert_eq!(relay.username, Some("user@example.com".to_string()));
        assert!(relay.use_tls);
    }

    #[tokio::test]
    async fn test_remote_delivery_init_without_relay() {
        let mut mailet = RemoteDeliveryMailet::new();
        let config = MailetConfig::new("RemoteDelivery");

        let result = mailet.init(config).await;
        assert!(result.is_ok());
        assert!(mailet.relay_config.is_none());
    }

    #[tokio::test]
    async fn test_serialize_message() {
        let mut headers = HeaderMap::new();
        headers.insert("From", "sender@example.com");
        headers.insert("To", "recipient@example.com");
        headers.insert("Subject", "Test");

        let message = MimeMessage::new(headers, MessageBody::Small(Bytes::from("Test body")));

        let mail = Mail::new(
            Some(MailAddress::from_str("sender@example.com").expect("valid address")),
            vec![MailAddress::from_str("recipient@example.com").expect("valid address")],
            message,
            None,
            None,
        );

        let data = RemoteDeliveryMailet::serialize_message(&mail);
        let text = String::from_utf8_lossy(&data);

        assert!(text.contains("from: sender@example.com"));
        assert!(text.contains("to: recipient@example.com"));
        assert!(text.contains("subject: Test"));
        assert!(text.contains("Test body"));
    }

    #[tokio::test]
    async fn test_remote_delivery_no_recipients() {
        let mailet = RemoteDeliveryMailet::new();
        let mut mail = create_test_mail("sender@local.com", vec![]);

        let action = mailet
            .service(&mut mail)
            .await
            .expect("service should not error");
        assert!(matches!(
            action,
            MailetAction::ChangeState(MailState::Ghost)
        ));
    }
}
