//! IMAP client for load testing

use anyhow::{Context, Result};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// IMAP client for load testing
pub struct ImapClient;

impl ImapClient {
    /// Fetch messages via IMAP
    pub async fn fetch_messages(host: &str, port: u16) -> Result<usize> {
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

        // LOGIN
        stream
            .write_all(b"A001 LOGIN testuser testpass\r\n")
            .await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // SELECT INBOX
        stream.write_all(b"A002 SELECT INBOX\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // FETCH
        stream.write_all(b"A003 FETCH 1:10 (FLAGS)\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // LOGOUT
        stream.write_all(b"A004 LOGOUT\r\n").await?;
        let _ = stream.read(&mut buffer).await;

        Ok(bytes_received)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_imap_client_timeout() {
        // Test connection timeout to non-existent host
        let result = ImapClient::fetch_messages("192.0.2.1", 143).await;
        assert!(result.is_err());
    }
}
