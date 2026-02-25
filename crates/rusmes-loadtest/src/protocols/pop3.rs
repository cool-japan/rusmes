//! POP3 client for load testing

use anyhow::{Context, Result};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// POP3 client for load testing
pub struct Pop3Client;

impl Pop3Client {
    /// Retrieve messages via POP3
    pub async fn retrieve_messages(host: &str, port: u16) -> Result<usize> {
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

        // USER
        stream.write_all(b"USER testuser\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // PASS
        stream.write_all(b"PASS testpass\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // STAT (get mailbox statistics)
        stream.write_all(b"STAT\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // LIST (list messages)
        stream.write_all(b"LIST\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // RETR 1 (retrieve first message)
        stream.write_all(b"RETR 1\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // QUIT
        stream.write_all(b"QUIT\r\n").await?;
        let _ = stream.read(&mut buffer).await;

        Ok(bytes_received)
    }

    /// Delete messages via POP3
    pub async fn delete_messages(host: &str, port: u16) -> Result<usize> {
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

        // USER
        stream.write_all(b"USER testuser\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // PASS
        stream.write_all(b"PASS testpass\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // DELE 1 (mark for deletion)
        stream.write_all(b"DELE 1\r\n").await?;
        let n = stream.read(&mut buffer).await?;
        bytes_received += n;

        // QUIT (commit deletions)
        stream.write_all(b"QUIT\r\n").await?;
        let _ = stream.read(&mut buffer).await;

        Ok(bytes_received)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pop3_client_timeout() {
        // Test connection timeout to non-existent host
        let result = Pop3Client::retrieve_messages("192.0.2.1", 110).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pop3_delete_timeout() {
        // Test connection timeout to non-existent host
        let result = Pop3Client::delete_messages("192.0.2.1", 110).await;
        assert!(result.is_err());
    }
}
