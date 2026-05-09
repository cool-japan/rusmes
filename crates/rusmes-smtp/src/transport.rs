//! SMTP mail transport client implementation
//!
//! Provides [`SmtpMailTransport`] which implements [`MailTransport`] from
//! `rusmes_core` using a minimal RFC 5321 SMTP client over a plain TCP socket
//! (with optional AUTH LOGIN).
//!
//! When an [`crate::outbound_pool::OutboundPool`] is attached via
//! [`SmtpMailTransport::with_pool`] the transport reuses established
//! connections for consecutive deliveries to the same host instead of opening
//! a new TCP connection per message.

use crate::outbound_pool::{smtp_read_response_raw, smtp_write, OutboundPool, PooledConn};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use rusmes_core::transport::{MailTransport, SmtpEnvelope};
use rusmes_proto::{Mail, MessageBody};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

// ── Internal queue entry ─────────────────────────────────────────────────────

/// An entry held in the in-memory scheduled-send queue.
#[derive(Debug)]
struct QueuedSend {
    id: String,
    envelope: SmtpEnvelope,
    /// Serialised RFC 5322 message bytes (headers + body).
    message_bytes: Vec<u8>,
    deliver_at: DateTime<Utc>,
}

// ── Configuration ────────────────────────────────────────────────────────────

/// Connection parameters for the upstream SMTP relay.
#[derive(Debug, Clone)]
pub struct SmtpRelayConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Per-connection I/O timeout.
    pub timeout: Duration,
}

impl Default for SmtpRelayConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 25,
            username: None,
            password: None,
            timeout: Duration::from_secs(30),
        }
    }
}

// ── SmtpMailTransport ────────────────────────────────────────────────────────

/// SMTP mail transport that delivers messages to a relay host.
///
/// Immediate sends open a TCP connection (or reuse a pooled one), handshake,
/// and deliver within [`MailTransport::send`].  Scheduled sends are queued in
/// memory and a background worker drains them when their `deliver_at` instant
/// is reached.
///
/// ## Connection pooling
///
/// Attach an [`OutboundPool`] via [`SmtpMailTransport::with_pool`] to reuse
/// SMTP connections across deliveries.  Without a pool every send opens and
/// closes a fresh TCP connection.
pub struct SmtpMailTransport {
    config: SmtpRelayConfig,
    queue: Arc<Mutex<VecDeque<QueuedSend>>>,
    notify: Arc<Notify>,
    /// Signals the background worker to stop.
    shutdown: Arc<tokio::sync::watch::Sender<bool>>,
    /// Optional outbound connection pool.  When `None` every delivery opens a
    /// fresh TCP connection (backward-compatible behaviour).
    pool: Option<Arc<OutboundPool>>,
}

impl SmtpMailTransport {
    /// Create a new transport and start the background drain worker.
    ///
    /// The worker shuts down when the returned `SmtpMailTransport` is dropped.
    pub fn new(
        host: String,
        port: u16,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
        let config = SmtpRelayConfig {
            host,
            port,
            username,
            password,
            ..Default::default()
        };

        let queue: Arc<Mutex<VecDeque<QueuedSend>>> = Arc::new(Mutex::new(VecDeque::new()));
        let notify = Arc::new(Notify::new());
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        let shutdown = Arc::new(tx);

        // Spawn background worker that drains due entries.
        let worker_queue = queue.clone();
        let worker_notify = notify.clone();
        let worker_config = config.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Wait for a notification (new entry enqueued) or a timeout.
                    _ = worker_notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = rx.changed() => {
                        // Shutdown requested — exit the loop.
                        break;
                    }
                }

                let now = Utc::now();
                let due_entries: Vec<QueuedSend> = {
                    let mut q = worker_queue.lock().await;
                    let mut due = Vec::new();
                    while let Some(front) = q.front() {
                        if front.deliver_at <= now {
                            if let Some(entry) = q.pop_front() {
                                due.push(entry);
                            }
                        } else {
                            break;
                        }
                    }
                    due
                };

                for entry in due_entries {
                    if let Err(e) =
                        deliver_via_smtp(&entry.envelope, &entry.message_bytes, &worker_config)
                            .await
                    {
                        tracing::error!("Scheduled send {} failed: {}", entry.id, e);
                    } else {
                        tracing::info!("Scheduled send {} delivered", entry.id);
                    }
                }
            }
        });

        Self {
            config,
            queue,
            notify,
            shutdown,
            pool: None,
        }
    }

    /// Attach an outbound connection pool so that consecutive deliveries to the
    /// same relay can reuse established TCP connections.
    ///
    /// Call this immediately after [`SmtpMailTransport::new`] before the
    /// transport is used.
    pub fn with_pool(mut self, pool: Arc<OutboundPool>) -> Self {
        self.pool = Some(pool);
        self
    }
}

impl Drop for SmtpMailTransport {
    fn drop(&mut self) {
        // Signal the background worker to exit gracefully.
        let _ = self.shutdown.send(true);
    }
}

#[async_trait]
impl MailTransport for SmtpMailTransport {
    async fn send(&self, envelope: SmtpEnvelope, mail: &Mail) -> anyhow::Result<String> {
        let msg_bytes = serialize_message(mail).await;
        deliver_via_smtp_pooled(&envelope, &msg_bytes, &self.config, self.pool.as_deref()).await?;
        let id = Uuid::new_v4().to_string();
        Ok(id)
    }

    async fn send_at(
        &self,
        envelope: SmtpEnvelope,
        mail: &Mail,
        at: DateTime<Utc>,
    ) -> anyhow::Result<String> {
        let id = Uuid::new_v4().to_string();
        let threshold = Utc::now() + chrono::Duration::seconds(5);

        if at <= threshold {
            // Near-immediate — deliver right away.
            let msg_bytes = serialize_message(mail).await;
            deliver_via_smtp(&envelope, &msg_bytes, &self.config).await?;
            return Ok(id);
        }

        // Enqueue for later delivery, inserting in sorted order.
        let entry = QueuedSend {
            id: id.clone(),
            envelope,
            message_bytes: serialize_message(mail).await,
            deliver_at: at,
        };

        {
            let mut q = self.queue.lock().await;
            // Insert in deliver_at order (earliest first) for efficient front-pop.
            let pos = q.iter().position(|e| e.deliver_at > at).unwrap_or(q.len());
            q.insert(pos, entry);
        }

        // Wake the background worker so it can compute the next sleep interval.
        self.notify.notify_one();

        Ok(id)
    }

    async fn cancel(&self, submission_id: &str) -> anyhow::Result<bool> {
        let mut q = self.queue.lock().await;
        if let Some(pos) = q.iter().position(|e| e.id == submission_id) {
            q.remove(pos);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

// ── Serialisation ────────────────────────────────────────────────────────────

/// Convert a [`Mail`] object to RFC 5322 wire bytes (headers + blank line + body).
///
/// For `MessageBody::Large`, the bytes are read asynchronously before serialisation.
pub(crate) async fn serialize_message(mail: &Mail) -> Vec<u8> {
    let mut data: Vec<u8> = Vec::new();

    for (name, values) in mail.message().headers().iter() {
        for value in values {
            data.extend_from_slice(name.as_bytes());
            data.extend_from_slice(b": ");
            data.extend_from_slice(value.as_bytes());
            data.extend_from_slice(b"\r\n");
        }
    }

    data.extend_from_slice(b"\r\n");

    match mail.message().body() {
        MessageBody::Small(bytes) => {
            data.extend_from_slice(bytes);
        }
        MessageBody::Large(large) => match large.read_to_bytes().await {
            Ok(bytes) => {
                data.extend_from_slice(&bytes);
            }
            Err(e) => {
                tracing::warn!("Failed to read large message body for SMTP delivery: {e}");
            }
        },
    }

    data
}

// ── Low-level SMTP client ────────────────────────────────────────────────────

/// Deliver pre-serialised `msg_bytes` through an existing pooled connection.
///
/// Expects the connection to already have completed greeting + EHLO (as done
/// by [`OutboundPool::get_or_connect`]).  Sends MAIL FROM / RCPT TO / DATA /
/// message.  Does **not** send QUIT — the caller is responsible for returning
/// the connection to the pool.
async fn run_smtp_transaction(
    conn: &mut PooledConn,
    envelope: &SmtpEnvelope,
    msg_bytes: &[u8],
    config: &SmtpRelayConfig,
) -> anyhow::Result<()> {
    // AUTH LOGIN if credentials provided (only on fresh connections — reused
    // connections have already authenticated).  We send AUTH on every reuse
    // here for simplicity; servers that disallow re-auth will reject it, which
    // is treated as a fatal error and the connection is dropped.
    if let (Some(user), Some(pass)) = (&config.username, &config.password) {
        smtp_write(&mut conn.reader, "AUTH LOGIN\r\n").await?;
        let _ = smtp_read_response_raw(&mut conn.reader).await?;

        smtp_write(&mut conn.reader, &format!("{}\r\n", BASE64.encode(user))).await?;
        let _ = smtp_read_response_raw(&mut conn.reader).await?;

        smtp_write(&mut conn.reader, &format!("{}\r\n", BASE64.encode(pass))).await?;
        let auth_resp = smtp_read_response_raw(&mut conn.reader).await?;
        if !auth_resp.starts_with("235") {
            anyhow::bail!("SMTP AUTH LOGIN failed: {}", auth_resp.trim());
        }
    }

    // MAIL FROM
    smtp_write(
        &mut conn.reader,
        &format!("MAIL FROM:<{}>\r\n", envelope.mail_from),
    )
    .await?;
    let mf_resp = smtp_read_response_raw(&mut conn.reader).await?;
    if !mf_resp.starts_with("250") {
        anyhow::bail!("SMTP MAIL FROM rejected: {}", mf_resp.trim());
    }

    // RCPT TO
    if envelope.rcpt_to.is_empty() {
        anyhow::bail!("No recipients in SmtpEnvelope");
    }
    let mut accepted = 0usize;
    for rcpt in &envelope.rcpt_to {
        smtp_write(&mut conn.reader, &format!("RCPT TO:<{}>\r\n", rcpt)).await?;
        let rcpt_resp = smtp_read_response_raw(&mut conn.reader).await?;
        if rcpt_resp.starts_with("250") || rcpt_resp.starts_with("251") {
            accepted += 1;
        } else {
            tracing::warn!("RCPT TO <{}> rejected: {}", rcpt, rcpt_resp.trim());
        }
    }
    if accepted == 0 {
        anyhow::bail!("All RCPT TO addresses rejected by relay");
    }

    // DATA
    smtp_write(&mut conn.reader, "DATA\r\n").await?;
    let data_resp = smtp_read_response_raw(&mut conn.reader).await?;
    if !data_resp.starts_with("354") {
        anyhow::bail!("SMTP DATA command failed: {}", data_resp.trim());
    }

    // Send message bytes + terminating dot.
    {
        let writer = conn.reader.get_mut();
        writer.write_all(msg_bytes).await?;
        if !msg_bytes.ends_with(b"\r\n") {
            writer.write_all(b"\r\n").await?;
        }
        writer.write_all(b".\r\n").await?;
        writer.flush().await?;
    }

    let send_resp = smtp_read_response_raw(&mut conn.reader).await?;
    if !send_resp.starts_with("250") {
        anyhow::bail!("SMTP message send rejected: {}", send_resp.trim());
    }

    Ok(())
}

/// Deliver pre-serialised `msg_bytes` to the relay described by `config`,
/// optionally reusing a connection from `pool`.
///
/// If `pool` is `Some`, the function tries to get a pooled connection; on
/// success returns it to the pool after delivery.  On delivery error the
/// connection is dropped (not returned).  If `pool` is `None`, a fresh
/// connection is opened and closed (QUIT) after delivery.
async fn deliver_via_smtp_pooled(
    envelope: &SmtpEnvelope,
    msg_bytes: &[u8],
    config: &SmtpRelayConfig,
    pool: Option<&OutboundPool>,
) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.host, config.port);

    if let Some(p) = pool {
        // Pooled path.
        let mut conn = tokio::time::timeout(config.timeout, p.get_or_connect(&addr))
            .await
            .map_err(|_| anyhow::anyhow!("SMTP pool get_or_connect timeout for {}", addr))??;

        match run_smtp_transaction(&mut conn, envelope, msg_bytes, config).await {
            Ok(()) => {
                p.return_conn(conn).await;
                Ok(())
            }
            Err(e) => {
                // Drop conn (not returned) on error.
                Err(e)
            }
        }
    } else {
        // Non-pooled path — classic open / deliver / quit.
        deliver_via_smtp_direct(envelope, msg_bytes, config).await
    }
}

/// Open a fresh TCP connection, run the full SMTP session (greeting + EHLO +
/// transaction + QUIT) and close.
async fn deliver_via_smtp_direct(
    envelope: &SmtpEnvelope,
    msg_bytes: &[u8],
    config: &SmtpRelayConfig,
) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.host, config.port);

    let stream = tokio::time::timeout(config.timeout, TcpStream::connect(&addr))
        .await
        .map_err(|_| anyhow::anyhow!("SMTP connection timeout to {}", addr))??;

    let mut reader = BufReader::new(stream);

    // Read greeting (220 …)
    let greeting = smtp_read_response_raw(&mut reader).await?;
    if !greeting.starts_with("220") {
        anyhow::bail!("Unexpected SMTP greeting: {}", greeting.trim());
    }

    // EHLO
    smtp_write(&mut reader, &format!("EHLO {}\r\n", config.host)).await?;
    let ehlo = smtp_read_response_raw(&mut reader).await?;
    tracing::debug!("SMTP EHLO: {}", ehlo.trim());

    // Wrap in a synthetic PooledConn to reuse run_smtp_transaction.
    let mut conn = PooledConn {
        reader,
        last_used: std::time::SystemTime::now(),
        extensions: crate::outbound_pool::SmtpExtensions::from_ehlo(&ehlo),
        remote_key: addr.clone(),
    };

    run_smtp_transaction(&mut conn, envelope, msg_bytes, config).await?;

    // QUIT
    smtp_write(&mut conn.reader, "QUIT\r\n").await?;
    let _ = smtp_read_response_raw(&mut conn.reader).await;

    Ok(())
}

/// Keep backward compatibility: the old `deliver_via_smtp` name is now an alias.
async fn deliver_via_smtp(
    envelope: &SmtpEnvelope,
    msg_bytes: &[u8],
    config: &SmtpRelayConfig,
) -> anyhow::Result<()> {
    deliver_via_smtp_direct(envelope, msg_bytes, config).await
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, MailAddress, MessageBody, MimeMessage};
    use std::str::FromStr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn make_mail(sender: &str, recipient: &str) -> Mail {
        let mut headers = HeaderMap::new();
        headers.insert("From", sender);
        headers.insert("To", recipient);
        headers.insert("Subject", "Test");
        let body = MessageBody::Small(Bytes::from("Hello, world!"));
        let msg = MimeMessage::new(headers, body);
        Mail::new(
            Some(MailAddress::from_str(sender).expect("addr")),
            vec![MailAddress::from_str(recipient).expect("addr")],
            msg,
            None,
            None,
        )
    }

    /// Spawn a minimal SMTP echo server that returns 250 to everything.
    async fn spawn_mock_smtp(responses: Vec<String>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock smtp");
        let port = listener.local_addr().expect("local addr").port();

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                for resp in responses {
                    socket
                        .write_all(resp.as_bytes())
                        .await
                        .expect("write response");
                }
                // Drain any incoming data so the client doesn't get ECONNRESET.
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
            }
        });

        port
    }

    #[tokio::test]
    async fn test_serialize_message() {
        let mail = make_mail("sender@example.com", "recipient@example.com");
        let bytes = serialize_message(&mail).await;
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("from: sender@example.com"));
        assert!(text.contains("to: recipient@example.com"));
        assert!(text.contains("Hello, world!"));
    }

    #[tokio::test]
    async fn test_smtp_mail_transport_send() {
        // Responses for: greeting, EHLO, MAIL FROM, RCPT TO, DATA, (message), QUIT
        let responses = vec![
            "220 localhost ESMTP\r\n".to_string(),
            "250-localhost\r\n250 PIPELINING\r\n".to_string(),
            "250 OK\r\n".to_string(),
            "250 OK\r\n".to_string(),
            "354 Go ahead\r\n".to_string(),
            "250 Queued\r\n".to_string(),
            "221 Bye\r\n".to_string(),
        ];

        let port = spawn_mock_smtp(responses).await;

        let transport = SmtpMailTransport::new("127.0.0.1".to_string(), port, None, None);

        let envelope = SmtpEnvelope {
            mail_from: "sender@example.com".to_string(),
            rcpt_to: vec!["recipient@example.com".to_string()],
        };

        let mail = make_mail("sender@example.com", "recipient@example.com");
        let result = transport.send(envelope, &mail).await;
        assert!(result.is_ok(), "send should succeed: {:?}", result);
        let id = result.expect("submission id");
        // Should be a valid UUID
        assert_eq!(id.len(), 36);
    }

    #[tokio::test]
    async fn test_smtp_mail_transport_send_at_immediate() {
        let responses = vec![
            "220 localhost ESMTP\r\n".to_string(),
            "250-localhost\r\n250 PIPELINING\r\n".to_string(),
            "250 OK\r\n".to_string(),
            "250 OK\r\n".to_string(),
            "354 Go ahead\r\n".to_string(),
            "250 Queued\r\n".to_string(),
            "221 Bye\r\n".to_string(),
        ];

        let port = spawn_mock_smtp(responses).await;
        let transport = SmtpMailTransport::new("127.0.0.1".to_string(), port, None, None);

        let envelope = SmtpEnvelope {
            mail_from: "sender@example.com".to_string(),
            rcpt_to: vec!["recipient@example.com".to_string()],
        };

        // Send at a time within the 5-second threshold → immediate delivery
        let at = Utc::now() + chrono::Duration::seconds(2);
        let mail = make_mail("sender@example.com", "recipient@example.com");
        let result = transport.send_at(envelope, &mail, at).await;
        assert!(
            result.is_ok(),
            "send_at immediate should succeed: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_smtp_mail_transport_send_at_queued() {
        // Future send - should be queued, not delivered immediately.
        let transport = SmtpMailTransport::new("127.0.0.1".to_string(), 9999, None, None);

        let envelope = SmtpEnvelope {
            mail_from: "sender@example.com".to_string(),
            rcpt_to: vec!["recipient@example.com".to_string()],
        };

        let at = Utc::now() + chrono::Duration::hours(2);
        let mail = make_mail("sender@example.com", "recipient@example.com");
        let id = transport
            .send_at(envelope, &mail, at)
            .await
            .expect("queue entry");

        // Entry should be in the queue
        let q = transport.queue.lock().await;
        assert_eq!(q.len(), 1);
        assert_eq!(q.front().map(|e| e.id.as_str()), Some(id.as_str()));
    }

    #[tokio::test]
    async fn test_smtp_mail_transport_cancel() {
        let transport = SmtpMailTransport::new("127.0.0.1".to_string(), 9999, None, None);

        let envelope = SmtpEnvelope {
            mail_from: "sender@example.com".to_string(),
            rcpt_to: vec!["recipient@example.com".to_string()],
        };

        let at = Utc::now() + chrono::Duration::hours(1);
        let mail = make_mail("sender@example.com", "recipient@example.com");
        let id = transport
            .send_at(envelope, &mail, at)
            .await
            .expect("queue entry");

        // Cancel it
        let canceled = transport.cancel(&id).await.expect("cancel ok");
        assert!(canceled, "cancel should return true for queued entry");

        // Queue should be empty
        let q = transport.queue.lock().await;
        assert!(q.is_empty());

        // Cancel again → returns false
        drop(q);
        let again = transport.cancel(&id).await.expect("cancel again ok");
        assert!(!again, "cancel of already-canceled should return false");
    }
}
