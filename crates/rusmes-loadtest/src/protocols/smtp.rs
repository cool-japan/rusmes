//! SMTP client for load testing

use anyhow::{Context, Result};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// SMTP client for load testing
pub struct SmtpClient;

impl SmtpClient {
    /// Send a message via SMTP
    pub async fn send_message(host: &str, port: u16, message: &str) -> Result<usize> {
        let addr = format!("{}:{}", host, port);

        let connect_timeout = Duration::from_secs(5);
        let stream = timeout(connect_timeout, TcpStream::connect(&addr))
            .await
            .context("Connection timeout")?
            .context("Failed to connect")?;

        let mut stream = stream;
        let mut buffer = vec![0u8; 4096];

        // Read greeting
        let n = timeout(Duration::from_secs(5), stream.read(&mut buffer))
            .await
            .context("Read timeout")?
            .context("Failed to read greeting")?;

        let mut bytes_received = n;

        // EHLO
        stream.write_all(b"EHLO loadtest.example.com\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // MAIL FROM
        stream
            .write_all(b"MAIL FROM:<loadtest@example.com>\r\n")
            .await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // RCPT TO
        stream.write_all(b"RCPT TO:<user@example.com>\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // DATA
        stream.write_all(b"DATA\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // Message
        stream.write_all(message.as_bytes()).await?;
        stream.write_all(b"\r\n.\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // QUIT
        stream.write_all(b"QUIT\r\n").await?;
        let _ = stream.read(&mut buffer).await;

        Ok(bytes_received)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_smtp_client_timeout() {
        // Test connection timeout to non-existent host
        let result = SmtpClient::send_message("192.0.2.1", 25, "test").await;
        assert!(result.is_err());
    }
}
