//! Outbound SMTP connection pool
//!
//! [`OutboundPool`] maintains a bounded set of reusable TCP connections to
//! remote SMTP servers.  Reusing an established connection avoids the per-
//! message cost of TCP + (optionally) TLS handshake and SMTP greeting/EHLO
//! round-trips.
//!
//! ## Design
//!
//! Each remote address (host:port string key) has its own
//! [`tokio::sync::Mutex`]-guarded [`std::collections::VecDeque`] of idle
//! [`PooledConn`]s.  `get_or_connect` pops from the front; `return_conn`
//! pushes to the back.  Before returning a connection to the pool the caller
//! MUST send `RSET\r\n` and wait for a `250` response; if that fails the
//! connection is dropped rather than returned.
//!
//! A background *idle reaper* task wakes every `idle_timeout / 2` and removes
//! connections whose `last_used` timestamp exceeds `idle_timeout`.
//!
//! ## Caps
//!
//! * **per-remote cap** — at most `per_remote_cap` idle connections are kept
//!   for any single remote address.  Connections beyond the cap are dropped on
//!   return.
//! * **global cap** — the sum of all idle connections across all remotes must
//!   not exceed `global_cap`.  Connections are dropped on return when the
//!   global counter would exceed the cap.

use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

// ── SMTP extensions advertised by the remote server ──────────────────────────

/// Extensions advertised by the remote SMTP server in its EHLO response.
///
/// These are captured once during connection establishment and re-used for
/// every subsequent message sent over a pooled connection, avoiding a
/// redundant EHLO round-trip.
#[derive(Debug, Clone, Default)]
pub struct SmtpExtensions {
    /// Maximum message size (from `250-SIZE <n>`), or `None` if not advertised.
    pub max_size: Option<usize>,
    /// Whether the remote advertises `PIPELINING`.
    pub pipelining: bool,
    /// Whether the remote advertises `8BITMIME`.
    pub eight_bit_mime: bool,
    /// Whether the remote advertises `STARTTLS`.
    pub starttls: bool,
}

impl SmtpExtensions {
    /// Parse the EHLO multi-line response text into an `SmtpExtensions` value.
    pub fn from_ehlo(ehlo_text: &str) -> Self {
        let mut ext = SmtpExtensions::default();
        for line in ehlo_text.lines() {
            // Lines look like "250-PIPELINING" or "250 SIZE 10240000"
            let keyword = line
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .trim_start_matches(['-', ' '])
                .to_ascii_uppercase();

            if keyword.starts_with("SIZE") {
                let parts: Vec<&str> = keyword.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    ext.max_size = parts[1].trim().parse().ok();
                }
            } else if keyword == "PIPELINING" {
                ext.pipelining = true;
            } else if keyword == "8BITMIME" {
                ext.eight_bit_mime = true;
            } else if keyword == "STARTTLS" {
                ext.starttls = true;
            }
        }
        ext
    }
}

// ── Pooled connection ─────────────────────────────────────────────────────────

/// A single idle SMTP connection held in the pool.
pub struct PooledConn {
    /// The underlying TCP stream, wrapped in a line-oriented buffer.
    pub reader: BufReader<TcpStream>,
    /// Wall-clock time of last use (used by the idle reaper).
    pub last_used: SystemTime,
    /// Extensions advertised by the remote server in the initial EHLO.
    pub extensions: SmtpExtensions,
    /// The canonical "host:port" key under which this connection is pooled.
    pub remote_key: String,
}

impl PooledConn {
    /// Extract the underlying [`TcpStream`] reference (read-only; write via
    /// `reader.get_mut()`).
    pub fn stream_mut(&mut self) -> &mut TcpStream {
        self.reader.get_mut()
    }
}

// ── Pool configuration ────────────────────────────────────────────────────────

/// Configuration snapshot used when constructing an [`OutboundPool`].
///
/// Sourced from [`rusmes_config::SmtpOutboundConfig`]; duplicated here to
/// avoid a compile-time dependency on `rusmes-config` inside `rusmes-smtp`.
#[derive(Debug, Clone)]
pub struct OutboundPoolConfig {
    /// Maximum connections kept idle for a single remote address.
    pub per_remote_cap: usize,
    /// Total connections kept idle across all remote addresses.
    pub global_cap: usize,
    /// Duration after which an idle connection is reaped.
    pub idle_timeout: Duration,
}

impl Default for OutboundPoolConfig {
    fn default() -> Self {
        Self {
            per_remote_cap: 8,
            global_cap: 256,
            idle_timeout: Duration::from_secs(30),
        }
    }
}

// ── OutboundPool ──────────────────────────────────────────────────────────────

/// Bounded pool of idle outbound SMTP connections.
///
/// # Thread safety
///
/// `OutboundPool` is `Send + Sync`; share it via `Arc<OutboundPool>`.
pub struct OutboundPool {
    /// Keyed by "host:port" string.
    conns: DashMap<String, Mutex<VecDeque<PooledConn>>>,
    config: OutboundPoolConfig,
    /// Running total of idle connections across all remotes.
    total_idle: Arc<AtomicUsize>,
}

impl OutboundPool {
    /// Create a new pool and spawn the background idle-reaper task.
    ///
    /// The reaper stops automatically when `shutdown_rx` yields `true`.
    pub fn new(
        config: OutboundPoolConfig,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Arc<Self> {
        let pool = Arc::new(Self {
            conns: DashMap::new(),
            config: config.clone(),
            total_idle: Arc::new(AtomicUsize::new(0)),
        });

        // Spawn the background reaper.
        let reaper_pool = pool.clone();
        let reap_interval = config.idle_timeout / 2;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(reap_interval) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
                reaper_pool.reap_idle().await;
            }
        });

        pool
    }

    /// Obtain a connection to `remote_key` (format: `"host:port"`).
    ///
    /// If a pooled idle connection is available, returns it immediately.
    /// Otherwise opens a new TCP connection, reads the `220` greeting, sends
    /// `EHLO localhost`, reads the response, and wraps everything in a
    /// [`PooledConn`].
    pub async fn get_or_connect(&self, remote_key: &str) -> anyhow::Result<PooledConn> {
        // Attempt to pop an idle connection.
        if let Some(bucket) = self.conns.get(remote_key) {
            let mut deque = bucket.lock().await;
            if let Some(conn) = deque.pop_front() {
                self.total_idle.fetch_sub(1, Ordering::Relaxed);
                return Ok(conn);
            }
        }

        // No idle connection — open a fresh one.
        self.open_fresh(remote_key).await
    }

    /// Return a connection to the pool after use.
    ///
    /// Sends `RSET\r\n` and waits for a `250` response.  On any I/O or
    /// protocol error the connection is silently dropped.  If the pool would
    /// exceed its caps the connection is also dropped.
    pub async fn return_conn(&self, mut conn: PooledConn) {
        // Send RSET to clear server-side transaction state.
        if let Err(e) = rset_connection(&mut conn).await {
            tracing::debug!(
                remote = conn.remote_key.as_str(),
                "dropping connection after failed RSET: {}",
                e
            );
            return;
        }

        // Enforce global cap before taking per-remote lock.
        if self.total_idle.load(Ordering::Relaxed) >= self.config.global_cap {
            tracing::debug!(
                remote = conn.remote_key.as_str(),
                "global pool cap reached, dropping connection"
            );
            return;
        }

        let remote_key = conn.remote_key.clone();

        // Insert the bucket lazily.
        let bucket = self
            .conns
            .entry(remote_key.clone())
            .or_insert_with(|| Mutex::new(VecDeque::new()));

        let mut deque = bucket.lock().await;

        // Enforce per-remote cap.
        if deque.len() >= self.config.per_remote_cap {
            tracing::debug!(
                remote = remote_key.as_str(),
                "per-remote cap reached, dropping connection"
            );
            return;
        }

        conn.last_used = SystemTime::now();
        deque.push_back(conn);
        self.total_idle.fetch_add(1, Ordering::Relaxed);
    }

    /// Count currently idle connections (for testing / metrics).
    pub fn idle_count(&self) -> usize {
        self.total_idle.load(Ordering::Relaxed)
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    /// Open a fresh TCP connection, perform the SMTP handshake (220 greeting +
    /// EHLO) and return the ready-to-use connection.
    async fn open_fresh(&self, remote_key: &str) -> anyhow::Result<PooledConn> {
        let stream = TcpStream::connect(remote_key)
            .await
            .map_err(|e| anyhow::anyhow!("SMTP outbound connect to {}: {}", remote_key, e))?;

        let mut reader = BufReader::new(stream);

        // Read 220 greeting.
        let greeting = smtp_read_response_raw(&mut reader).await?;
        if !greeting.starts_with("220") {
            anyhow::bail!(
                "unexpected SMTP greeting from {}: {}",
                remote_key,
                greeting.trim()
            );
        }

        // Send EHLO.
        smtp_write(&mut reader, "EHLO localhost\r\n").await?;
        let ehlo_text = smtp_read_response_raw(&mut reader).await?;
        if !ehlo_text.starts_with("250") {
            anyhow::bail!("EHLO rejected by {}: {}", remote_key, ehlo_text.trim());
        }

        let extensions = SmtpExtensions::from_ehlo(&ehlo_text);

        Ok(PooledConn {
            reader,
            last_used: SystemTime::now(),
            extensions,
            remote_key: remote_key.to_string(),
        })
    }

    /// Iterate all buckets and drop connections idle longer than
    /// `config.idle_timeout`.
    async fn reap_idle(&self) {
        let now = SystemTime::now();
        let mut total_reaped = 0usize;

        for bucket_ref in self.conns.iter() {
            let mut deque = bucket_ref.value().lock().await;
            let before = deque.len();
            deque.retain(|conn| {
                match conn.last_used.elapsed() {
                    Ok(elapsed) => elapsed <= self.config.idle_timeout,
                    // If the system clock went backward, keep the connection.
                    Err(_) => true,
                }
            });
            let reaped = before - deque.len();
            total_reaped += reaped;
        }

        if total_reaped > 0 {
            self.total_idle.fetch_sub(total_reaped, Ordering::Relaxed);
            tracing::debug!(
                "outbound pool idle reaper: closed {} connections",
                total_reaped
            );
        }

        let _ = now; // suppress unused warning
    }
}

// ── Low-level SMTP helpers ────────────────────────────────────────────────────

/// Write a command to the underlying `TcpStream` inside a `BufReader`.
pub(crate) async fn smtp_write(
    reader: &mut BufReader<TcpStream>,
    cmd: &str,
) -> std::io::Result<()> {
    let stream = reader.get_mut();
    stream.write_all(cmd.as_bytes()).await?;
    stream.flush().await
}

/// Read a (possibly multi-line) SMTP response from `reader`.
///
/// Returns the entire response as a single `String` (lines joined with `\n`).
pub(crate) async fn smtp_read_response_raw(
    reader: &mut BufReader<TcpStream>,
) -> std::io::Result<String> {
    let mut full = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        let is_last = line.len() >= 4 && line.as_bytes().get(3) == Some(&b' ');
        full.push_str(&line);
        if is_last {
            break;
        }
    }
    Ok(full)
}

/// Send `RSET\r\n` on `conn` and read the response.  Returns `Ok(())` on a
/// `250` response; `Err` for any I/O error or unexpected response code.
async fn rset_connection(conn: &mut PooledConn) -> anyhow::Result<()> {
    smtp_write(&mut conn.reader, "RSET\r\n").await?;
    let rset_resp = smtp_read_response_raw(&mut conn.reader).await?;
    if !rset_resp.starts_with("250") {
        anyhow::bail!("RSET rejected: {}", rset_resp.trim());
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    // ── Minimal fake SMTP server ──────────────────────────────────────────

    /// Describes the behaviour the fake server should exhibit.
    #[derive(Debug, Clone)]
    struct FakeServerBehaviour {
        /// How many connections to accept.
        accept_count: usize,
        /// Canned multi-line EHLO response (without trailing CRLF).
        ehlo_response: String,
        /// Whether to accept RSET.
        accept_rset: bool,
        /// Whether to accept MAIL FROM.
        accept_mail: bool,
        /// Whether to accept RCPT TO.
        accept_rcpt: bool,
        /// Whether to accept DATA.
        accept_data: bool,
    }

    impl Default for FakeServerBehaviour {
        fn default() -> Self {
            Self {
                accept_count: 1,
                ehlo_response: "250-localhost\r\n250 PIPELINING\r\n".to_string(),
                accept_rset: true,
                accept_mail: true,
                accept_rcpt: true,
                accept_data: true,
            }
        }
    }

    /// Returns (port, connect_count_receiver).
    ///
    /// The channel yields the cumulative number of accepted connections; read
    /// after your operations to assert exactly how many times a new TCP
    /// connection was made.
    async fn spawn_fake_smtp(
        behaviour: FakeServerBehaviour,
    ) -> (u16, tokio::sync::watch::Receiver<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake smtp");
        let port = listener.local_addr().expect("local addr").port();
        let (tx, rx) = tokio::sync::watch::channel(0usize);

        tokio::spawn(async move {
            let mut count = 0usize;
            while count < behaviour.accept_count {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                count += 1;
                let _ = tx.send(count);
                let beh = behaviour.clone();

                tokio::spawn(async move {
                    // Greeting
                    socket.write_all(b"220 localhost ESMTP\r\n").await.ok();

                    // Read lines and respond until connection is closed or QUIT received.
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        let raw = String::from_utf8_lossy(&buf[..n]);
                        let cmd = raw.trim().to_ascii_uppercase();

                        if cmd.starts_with("EHLO") || cmd.starts_with("HELO") {
                            socket.write_all(beh.ehlo_response.as_bytes()).await.ok();
                        } else if cmd.starts_with("RSET") {
                            if beh.accept_rset {
                                socket.write_all(b"250 OK\r\n").await.ok();
                            } else {
                                socket
                                    .write_all(b"500 Command not recognized\r\n")
                                    .await
                                    .ok();
                            }
                        } else if cmd.starts_with("MAIL") {
                            if beh.accept_mail {
                                socket.write_all(b"250 OK\r\n").await.ok();
                            } else {
                                socket.write_all(b"550 Rejected\r\n").await.ok();
                            }
                        } else if cmd.starts_with("RCPT") {
                            if beh.accept_rcpt {
                                socket.write_all(b"250 OK\r\n").await.ok();
                            } else {
                                socket.write_all(b"550 Rejected\r\n").await.ok();
                            }
                        } else if cmd.starts_with("DATA") {
                            if beh.accept_data {
                                socket.write_all(b"354 Go ahead\r\n").await.ok();
                                // Read until ".\r\n"
                                let mut data_buf = [0u8; 4096];
                                loop {
                                    let dn = socket.read(&mut data_buf).await.unwrap_or(0);
                                    if dn == 0 {
                                        break;
                                    }
                                    let chunk = String::from_utf8_lossy(&data_buf[..dn]);
                                    if chunk.contains("\r\n.\r\n") || chunk.trim() == "." {
                                        socket.write_all(b"250 Queued\r\n").await.ok();
                                        break;
                                    }
                                }
                            } else {
                                socket.write_all(b"550 Rejected\r\n").await.ok();
                            }
                        } else if cmd.starts_with("QUIT") {
                            socket.write_all(b"221 Bye\r\n").await.ok();
                            break;
                        }
                        // Unknown commands: ignore.
                    }
                });
            }
        });

        (port, rx)
    }

    // ── Helper: build a pool with the fake server ─────────────────────────

    fn make_pool(
        config: OutboundPoolConfig,
    ) -> (Arc<OutboundPool>, tokio::sync::watch::Sender<bool>) {
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let pool = OutboundPool::new(config, shutdown_rx);
        (pool, shutdown_tx)
    }

    // ── Tests ─────────────────────────────────────────────────────────────

    /// Two back-to-back `get_or_connect` + `return_conn` cycles should result
    /// in only ONE TCP connection being established (the second delivery reuses
    /// the pooled connection).
    #[tokio::test]
    async fn test_outbound_pool_basic_reuse() {
        let beh = FakeServerBehaviour {
            accept_count: 2, // allow up to 2 but we should only use 1
            ..Default::default()
        };
        let (port, connect_rx) = spawn_fake_smtp(beh).await;
        let remote = format!("127.0.0.1:{}", port);

        let config = OutboundPoolConfig {
            per_remote_cap: 4,
            global_cap: 16,
            idle_timeout: Duration::from_secs(30),
        };
        let (pool, _tx) = make_pool(config);

        // First delivery.
        let conn1 = pool
            .get_or_connect(&remote)
            .await
            .expect("first connect should succeed");
        assert_eq!(
            *connect_rx.borrow(),
            1,
            "one TCP connection after first get"
        );
        pool.return_conn(conn1).await;
        assert_eq!(pool.idle_count(), 1, "one idle conn after return");

        // Second delivery — should reuse.
        let conn2 = pool
            .get_or_connect(&remote)
            .await
            .expect("second get should succeed");
        // Still exactly 1 TCP connect.
        assert_eq!(
            *connect_rx.borrow(),
            1,
            "connection count must stay at 1 (pooled reuse)"
        );
        pool.return_conn(conn2).await;
        assert_eq!(pool.idle_count(), 1);
    }

    /// After idle_timeout elapses the background reaper must close idle connections.
    #[tokio::test]
    async fn test_outbound_pool_idle_reaper() {
        let beh = FakeServerBehaviour {
            accept_count: 1,
            ..Default::default()
        };
        let (port, _connect_rx) = spawn_fake_smtp(beh).await;
        let remote = format!("127.0.0.1:{}", port);

        // Very short idle_timeout so the reaper fires quickly.
        let idle_timeout = Duration::from_millis(80);
        let config = OutboundPoolConfig {
            per_remote_cap: 4,
            global_cap: 16,
            idle_timeout,
        };
        let (pool, _tx) = make_pool(config);

        // Get a connection, return it, then wait longer than idle_timeout.
        let conn = pool
            .get_or_connect(&remote)
            .await
            .expect("connect must succeed");
        pool.return_conn(conn).await;
        assert_eq!(pool.idle_count(), 1, "one idle conn before timeout");

        // Wait for the reaper to run (reaps every idle_timeout/2 = 40 ms).
        tokio::time::sleep(idle_timeout * 3).await;

        assert_eq!(
            pool.idle_count(),
            0,
            "idle conn must be reaped after timeout"
        );
    }

    /// `return_conn` must send RSET before putting the connection back.
    #[tokio::test]
    async fn test_outbound_pool_rset_on_return() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        let remote = format!("127.0.0.1:{}", port);

        // Collect commands seen by the server.
        let (seen_tx, mut seen_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            socket.write_all(b"220 localhost ESMTP\r\n").await.ok();

            let mut buf = [0u8; 4096];
            loop {
                let n = match socket.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let raw = String::from_utf8_lossy(&buf[..n]).to_string();
                let cmd = raw.trim().to_ascii_uppercase();

                if cmd.starts_with("EHLO") || cmd.starts_with("HELO") {
                    socket.write_all(b"250 localhost\r\n").await.ok();
                } else if cmd.starts_with("RSET") {
                    let _ = seen_tx.send("RSET".to_string());
                    socket.write_all(b"250 OK\r\n").await.ok();
                } else if cmd.starts_with("QUIT") {
                    socket.write_all(b"221 Bye\r\n").await.ok();
                    break;
                }
            }
        });

        let config = OutboundPoolConfig::default();
        let (pool, _tx) = make_pool(config);

        let conn = pool
            .get_or_connect(&remote)
            .await
            .expect("connect must succeed");
        pool.return_conn(conn).await;

        // The server should have received RSET.
        let cmd = tokio::time::timeout(Duration::from_secs(2), seen_rx.recv())
            .await
            .expect("timed out waiting for RSET")
            .expect("channel closed");
        assert_eq!(cmd, "RSET");
    }

    /// `SmtpExtensions::from_ehlo` correctly parses a multi-line EHLO response.
    #[test]
    fn test_smtp_extensions_parsing() {
        let ehlo = "250-localhost\r\n250-SIZE 10240000\r\n250-PIPELINING\r\n250-8BITMIME\r\n250 STARTTLS\r\n";
        let ext = SmtpExtensions::from_ehlo(ehlo);
        assert_eq!(ext.max_size, Some(10_240_000));
        assert!(ext.pipelining);
        assert!(ext.eight_bit_mime);
        assert!(ext.starttls);
    }
}
