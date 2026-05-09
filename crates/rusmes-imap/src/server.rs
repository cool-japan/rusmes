//! IMAP server implementation

use crate::config::ImapConfig;
use crate::handler::HandlerContext;
use crate::mailbox_watcher::{MailboxChanges, MailboxWatcher};
use crate::parser::{has_literal, parse_append_command, parse_command, LiteralType};
use crate::response::ImapResponse;
use crate::session::{ImapSession, ImapState};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

/// Type-erased async reader used by the IMAP session loop.
type DynReader = Box<dyn tokio::io::AsyncRead + Send + Unpin>;
/// Type-erased async writer used by the IMAP session loop.
type DynWriter = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;

/// IMAP server
pub struct ImapServer {
    bind_addr: String,
    context: HandlerContext,
    idle_timeout: Duration,
}

impl ImapServer {
    /// Create a new IMAP server with default idle timeout (30 minutes)
    pub fn new(bind_addr: impl Into<String>, context: HandlerContext) -> Self {
        Self::new_with_timeout(bind_addr, context, Duration::from_secs(1800))
    }

    /// Create a new IMAP server with custom idle timeout
    pub fn new_with_timeout(
        bind_addr: impl Into<String>,
        context: HandlerContext,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            bind_addr: bind_addr.into(),
            context,
            idle_timeout,
        }
    }

    /// Create a new IMAP server from configuration
    pub fn from_config(config: ImapConfig, context: HandlerContext) -> Self {
        Self {
            bind_addr: config.bind_addr(),
            context,
            idle_timeout: config.idle_timeout,
        }
    }

    /// Run the IMAP server
    pub async fn run(self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!("IMAP server listening on {}", self.bind_addr);

        let context = std::sync::Arc::new(self.context);
        let idle_timeout = self.idle_timeout;

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            tracing::info!("New IMAP connection from {}", remote_addr);

            let ctx = context.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, ctx, idle_timeout).await {
                    tracing::error!("IMAP session error: {}", e);
                }
            });
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    ctx: std::sync::Arc<HandlerContext>,
    idle_timeout: Duration,
) -> anyhow::Result<()> {
    // Track this session in the active-connections gauge and the TLS counter.
    // The guard's Drop decrements the gauge regardless of how this fn exits.
    let metrics = rusmes_metrics::global_metrics();
    let _conn_guard = metrics.connection_guard("imap");
    // Plaintext IMAP listener; an implicit-TLS ("imaps" on 993) listener would be wrapped
    // before reaching this fn. STARTTLS upgrade should call inc_tls_session(STARTTLS) on
    // success — handled in the STARTTLS command path inside crate::handler.
    metrics.inc_tls_session(rusmes_metrics::tls_label::NO);

    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader: BufReader<DynReader> = BufReader::new(Box::new(read_half));
    let mut writer: BufWriter<DynWriter> = BufWriter::new(Box::new(write_half));

    let mut session = ImapSession::new_with_timeout(idle_timeout);

    // Send greeting
    let greeting = ImapResponse::new(None, "OK", "RusMES IMAP Server ready");
    writer.write_all(greeting.format().as_bytes()).await?;
    writer.flush().await?;

    imap_session_loop(ctx, &mut session, &mut reader, &mut writer, idle_timeout).await
}

/// Core IMAP command loop.
///
/// Uses type-erased reader/writer (`DynReader`/`DynWriter`) so that DEFLATE wrapping
/// can be swapped in transparently after `COMPRESS DEFLATE` negotiation without
/// recursion or infinite generic type chains.  Per RFC 4978 §3, COMPRESS can be
/// negotiated at most once per session, so the swap happens at most once.
async fn imap_session_loop(
    ctx: std::sync::Arc<HandlerContext>,
    session: &mut ImapSession,
    reader: &mut BufReader<DynReader>,
    writer: &mut BufWriter<DynWriter>,
    idle_timeout: Duration,
) -> anyhow::Result<()> {
    let mut line = String::new();

    loop {
        line.clear();

        // Read command with idle timeout
        let n = match tokio::time::timeout(idle_timeout, reader.read_line(&mut line)).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                tracing::error!("Read error: {}", e);
                return Err(e.into());
            }
            Err(_) => {
                // Idle timeout - auto-logout
                tracing::info!(
                    "Idle timeout for connection - auto-logout after {} seconds",
                    idle_timeout.as_secs()
                );
                let bye_msg = "* BYE Autologout; idle too long\r\n";
                let _ = writer.write_all(bye_msg.as_bytes()).await;
                let _ = writer.flush().await;
                break;
            }
        };

        if n == 0 {
            break; // EOF
        }

        let line_trimmed = line.trim();
        tracing::debug!("IMAP command: {}", line_trimmed);

        // Check for literal in command (APPEND, etc.)
        let (tag, command) = if let Some((size, literal_type)) = has_literal(line_trimmed) {
            // Handle literal data
            match handle_literal_command(line_trimmed, size, literal_type, reader, writer).await {
                Ok(cmd) => cmd,
                Err(e) => {
                    let response = ImapResponse::bad("*", format!("Literal error: {}", e));
                    writer.write_all(response.format().as_bytes()).await?;
                    writer.flush().await?;
                    continue;
                }
            }
        } else {
            // Parse regular command
            match parse_command(line_trimmed) {
                Ok(cmd) => cmd,
                Err(e) => {
                    let response = ImapResponse::bad("*", format!("Parse error: {}", e));
                    writer.write_all(response.format().as_bytes()).await?;
                    writer.flush().await?;
                    continue;
                }
            }
        };

        // Handle command
        let response = crate::handler::handle_command(&ctx, session, &tag, command).await?;

        // Drain any cross-session mailbox notifications accumulated since the last command
        // and emit them as untagged responses before the tagged response (RFC 3501 §5.2).
        for untagged in session.drain_mailbox_events() {
            writer.write_all(untagged.as_bytes()).await?;
            writer.write_all(b"\r\n").await?;
        }

        writer.write_all(response.format().as_bytes()).await?;
        writer.flush().await?;

        // COMPRESS=DEFLATE: switch to compressed reader/writer.
        // The OK response is already sent and flushed above.  Swap the inner reader and
        // writer for DEFLATE-wrapped versions, then `continue` to the top of the loop
        // where `read_line` now reads from the compressed stream.
        //
        // We consume the current BufReader/BufWriter with `into_inner()` and build new
        // ones wrapping the DEFLATE adapters.  Any buffered-but-unread bytes in the old
        // BufReader are discarded, which is safe: the client does not send compressed data
        // until it receives our OK (RFC 4978 §2.2).
        if session.compress_pending {
            session.compress_pending = false;
            // Swap out the old BufReader/BufWriter using a sentinel placeholder so we
            // can call .into_inner() on the owned value.  The placeholder is immediately
            // replaced before any further reads/writes occur.
            let placeholder_r: BufReader<DynReader> = BufReader::new(Box::new(tokio::io::empty()));
            let placeholder_w: BufWriter<DynWriter> = BufWriter::new(Box::new(tokio::io::sink()));
            let old_reader = std::mem::replace(reader, placeholder_r);
            let old_writer = std::mem::replace(writer, placeholder_w);
            let inner_r: DynReader = old_reader.into_inner();
            let inner_w: DynWriter = old_writer.into_inner();
            let wrapped_r: DynReader =
                Box::new(oxiarc_deflate::raw_stream::RawInflateReader::new(inner_r));
            let wrapped_w: DynWriter = Box::new(oxiarc_deflate::raw_stream::RawDeflateWriter::new(
                inner_w, 6,
            ));
            *reader = BufReader::new(wrapped_r);
            *writer = BufWriter::new(wrapped_w);
            continue;
        }

        // Check if we entered IDLE mode
        if matches!(session.state(), ImapState::Idle { .. }) {
            // Enter IDLE loop
            if let Err(e) = handle_idle_mode(&ctx, session, reader, writer).await {
                tracing::error!("IDLE mode error: {}", e);
                // Exit IDLE and continue
                if let Some(mailbox_id) = session.mailbox_id() {
                    session.state = ImapState::Selected {
                        mailbox_id: *mailbox_id,
                    };
                }
            }
        }

        // Check for logout
        if matches!(session.state(), ImapState::Logout) {
            break;
        }
    }

    Ok(())
}

/// Handle IDLE mode - wait for DONE or mailbox changes
async fn handle_idle_mode<R, W>(
    ctx: &HandlerContext,
    session: &mut ImapSession,
    reader: &mut BufReader<R>,
    writer: &mut BufWriter<W>,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mailbox_id = match session.state() {
        ImapState::Idle { mailbox_id } => *mailbox_id,
        _ => return Ok(()), // Not in IDLE state
    };

    let tag = session.tag.clone().unwrap_or_else(|| "A001".to_string());
    let watcher = MailboxWatcher::new(ctx.metadata_store.clone());

    // Get initial snapshot
    let mut last_state = if let Some(ref snapshot) = session.mailbox_snapshot {
        MailboxChanges::new(snapshot.exists, snapshot.recent)
    } else {
        watcher.get_mailbox_state(&mailbox_id).await?
    };

    // IDLE loop with 29-minute timeout (RFC 2177)
    let idle_timeout = Duration::from_secs(29 * 60);
    // Polling fallback interval in case the broadcast channel has no senders yet.
    let check_interval = Duration::from_secs(30);

    let mut interval = tokio::time::interval(check_interval);
    let idle_deadline = tokio::time::Instant::now() + idle_timeout;

    // Subscribe to the broadcast channel for immediate cross-session notifications.
    // If no channel exists yet (no sender), fall through to the polling path.
    let mut event_rx = session
        .mailbox_event_rx
        .take()
        .unwrap_or_else(|| ctx.mailbox_registry.subscribe(mailbox_id));

    loop {
        let mut line = String::new();

        tokio::select! {
            // Check for DONE command from client
            result = reader.read_line(&mut line) => {
                let n = result?;
                if n == 0 {
                    // Connection closed — put the receiver back before returning
                    session.mailbox_event_rx = Some(event_rx);
                    break;
                }

                let line_trimmed = line.trim();
                if line_trimmed.eq_ignore_ascii_case("DONE") {
                    // Exit IDLE mode — put the receiver back
                    session.mailbox_event_rx = Some(event_rx);
                    break;
                }
                // Ignore other input during IDLE
            }

            // Receive cross-session broadcast events immediately
            event_result = event_rx.recv() => {
                match event_result {
                    Ok(event) => {
                        use crate::session::format_mailbox_event_pub;
                        if let Some(line_str) = format_mailbox_event_pub(&event) {
                            writer.write_all(line_str.as_bytes()).await?;
                            writer.write_all(b"\r\n").await?;
                            writer.flush().await?;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            "IDLE session lagged {n} broadcast events — re-polling mailbox state"
                        );
                        // Re-sync via polling fallback
                        let current_state = watcher.get_mailbox_state(&mailbox_id).await?;
                        if current_state.exists != last_state.exists {
                            let resp = format!("* {} EXISTS\r\n", current_state.exists);
                            writer.write_all(resp.as_bytes()).await?;
                        }
                        if current_state.recent != last_state.recent {
                            let resp = format!("* {} RECENT\r\n", current_state.recent);
                            writer.write_all(resp.as_bytes()).await?;
                        }
                        writer.flush().await?;
                        last_state = current_state;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Channel dropped — stop trying to recv() to avoid tight-loop.
                        // Replace with a fresh subscription (creates a new sender via registry)
                        // or simply break out of the IDLE loop.
                        tracing::debug!("IDLE broadcast channel closed, exiting IDLE");
                        session.mailbox_event_rx = None;
                        break;
                    }
                }
            }

            // Polling fallback — also handles 29-min timeout
            _ = interval.tick() => {
                let current_state = watcher.get_mailbox_state(&mailbox_id).await?;

                if current_state.has_changes(&last_state) {
                    if current_state.exists != last_state.exists {
                        let resp = format!("* {} EXISTS\r\n", current_state.exists);
                        writer.write_all(resp.as_bytes()).await?;
                    }
                    if current_state.recent != last_state.recent {
                        let resp = format!("* {} RECENT\r\n", current_state.recent);
                        writer.write_all(resp.as_bytes()).await?;
                    }
                    writer.flush().await?;
                    last_state = current_state;
                }

                if tokio::time::Instant::now() >= idle_deadline {
                    tracing::debug!("IDLE timeout reached after 29 minutes");
                    session.mailbox_event_rx = Some(event_rx);
                    break;
                }
            }
        }
    }

    // Send completion response
    let completion = ImapResponse::ok(&tag, "IDLE terminated");
    writer.write_all(completion.format().as_bytes()).await?;
    writer.flush().await?;

    // Return to Selected state
    session.state = ImapState::Selected { mailbox_id };
    session.tag = None;

    Ok(())
}

/// Handle a command with literal data
///
/// This function implements RFC 7888 LITERAL+ support:
/// - Synchronizing literals {size}: Server sends "+ Ready for literal data" continuation
/// - Non-synchronizing literals {size+}: Server accepts data immediately without continuation
async fn handle_literal_command<R, W>(
    command_line: &str,
    literal_size: usize,
    literal_type: LiteralType,
    reader: &mut BufReader<R>,
    writer: &mut BufWriter<W>,
) -> anyhow::Result<(String, crate::command::ImapCommand)>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    // For synchronizing literals, send continuation response
    if literal_type == LiteralType::Synchronizing {
        let continuation = "+ Ready for literal data\r\n";
        writer.write_all(continuation.as_bytes()).await?;
        writer.flush().await?;
        tracing::debug!(
            "Sent continuation for synchronizing literal of {} bytes",
            literal_size
        );
    } else {
        tracing::debug!(
            "Processing non-synchronizing literal (LITERAL+) of {} bytes",
            literal_size
        );
    }

    // Read literal data
    let mut literal_data = vec![0u8; literal_size];
    reader.read_exact(&mut literal_data).await?;

    // Read the trailing CRLF after literal
    let mut crlf = [0u8; 2];
    reader.read_exact(&mut crlf).await?;

    // Parse the APPEND command with literal data
    parse_append_command(command_line, literal_data)
        .map_err(|e| anyhow::anyhow!("Failed to parse APPEND command: {}", e))
}
