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
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    let mut session = ImapSession::new_with_timeout(idle_timeout);

    // Send greeting
    let greeting = ImapResponse::new(None, "OK", "RusMES IMAP Server ready");
    writer.write_all(greeting.format().as_bytes()).await?;
    writer.flush().await?;

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
            match handle_literal_command(line_trimmed, size, literal_type, &mut reader, &mut writer)
                .await
            {
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
        let response = crate::handler::handle_command(&ctx, &mut session, &tag, command).await?;

        writer.write_all(response.format().as_bytes()).await?;
        writer.flush().await?;

        // Check if we entered IDLE mode
        if matches!(session.state(), ImapState::Idle { .. }) {
            // Enter IDLE loop
            if let Err(e) = handle_idle_mode(&ctx, &mut session, &mut reader, &mut writer).await {
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
    let check_interval = Duration::from_secs(5); // Check for changes every 5 seconds

    let mut interval = tokio::time::interval(check_interval);
    let idle_deadline = tokio::time::Instant::now() + idle_timeout;

    loop {
        let mut line = String::new();

        tokio::select! {
            // Check for DONE command
            result = reader.read_line(&mut line) => {
                let n = result?;
                if n == 0 {
                    // Connection closed
                    break;
                }

                let line_trimmed = line.trim();
                if line_trimmed.eq_ignore_ascii_case("DONE") {
                    // Exit IDLE mode
                    break;
                }
                // Ignore other input during IDLE
            }

            // Check for mailbox changes periodically
            _ = interval.tick() => {
                let current_state = watcher.get_mailbox_state(&mailbox_id).await?;

                if current_state.has_changes(&last_state) {
                    // Send untagged responses for changes
                    if current_state.exists != last_state.exists {
                        let exists_response = format!("* {} EXISTS\r\n", current_state.exists);
                        writer.write_all(exists_response.as_bytes()).await?;
                    }

                    if current_state.recent != last_state.recent {
                        let recent_response = format!("* {} RECENT\r\n", current_state.recent);
                        writer.write_all(recent_response.as_bytes()).await?;
                    }

                    writer.flush().await?;
                    last_state = current_state;
                }

                // Check if we've exceeded the timeout
                if tokio::time::Instant::now() >= idle_deadline {
                    tracing::debug!("IDLE timeout reached after 29 minutes");
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
