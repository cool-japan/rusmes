//! DATA command pipeline: in-memory/tempfile hybrid sink and message routing.
//!
//! This module implements the RFC 5321 DATA receiver with a threshold-based
//! spill policy:
//!
//! - Messages that fit within [`SmtpConfig::data_tempfile_threshold`] are kept
//!   in a `Vec<u8>` and handed off as [`rusmes_proto::MessageBody::Small`].
//! - Messages that exceed the threshold are progressively written to a
//!   [`tempfile::NamedTempFile`] and handed off as
//!   [`rusmes_proto::MessageBody::Large`].
//!
//! Both paths produce the same SMTP wire behaviour: a `250 OK` followed by
//! async message processing via [`rusmes_core::MailProcessorRouter`].

use super::{SmtpSession, SmtpSessionHandler, SmtpState};
use crate::response::SmtpResponse;
use std::io::Write as IoWrite;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// Receive the DATA payload from the wire, enforce size limits, and dispatch
/// the message through the processing pipeline.
///
/// Called by [`SmtpSessionHandler::handle_data_input`] which is the public
/// entry point used by the main session loop and by tests that drive the DATA
/// path with an in-memory reader.
pub(super) async fn handle_data_input<R, W>(
    session: &mut SmtpSession,
    reader: &mut R,
    writer: &mut W,
    remote_addr: &SocketAddr,
) -> anyhow::Result<()>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let threshold = session.config.data_tempfile_threshold;
    let max_size = session.config.max_message_size;
    let spill_dir = session.config.data_spill_dir.clone();

    // Hybrid sink: start in memory, spill to disk when threshold is crossed.
    let mut mem_buf: Vec<u8> = Vec::new();
    let mut temp_file: Option<NamedTempFile> = None;
    let mut total_bytes: usize = 0;

    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("Unexpected EOF during DATA"));
        }

        // RFC 5321 §4.5.2 transparency: a bare `.<CRLF>` ends the message.
        if line.trim() == "." {
            break;
        }

        // Dot-unstuffing: leading `..` becomes `.`.
        let line_to_add: &str = if line.starts_with("..") {
            &line[1..]
        } else {
            &line
        };

        let chunk = line_to_add.as_bytes();
        let new_total = total_bytes.saturating_add(chunk.len());

        // Enforce max_message_size incrementally to avoid buffering huge messages.
        if new_total > max_size {
            tracing::info!(
                peer = %remote_addr,
                current_size = new_total,
                max_size = max_size,
                "smtp.session DATA rejected incrementally: size limit exceeded"
            );
            rusmes_metrics::global_metrics().inc_smtp_messages_rejected();
            SmtpSessionHandler::write_response_to(
                writer,
                SmtpResponse::storage_exceeded(format!(
                    "Message size exceeds maximum {}",
                    max_size
                )),
                remote_addr,
            )
            .await?;
            session.transaction.reset();
            session.state = SmtpState::Authenticated;
            return Ok(());
        }

        total_bytes = new_total;

        // Spill to tempfile when threshold is exceeded.
        if temp_file.is_none() && mem_buf.len() + chunk.len() > threshold {
            let mut tf = NamedTempFile::new_in(&spill_dir)
                .map_err(|e| anyhow::anyhow!("Failed to create tempfile for DATA spill: {}", e))?;
            tf.write_all(&mem_buf)
                .map_err(|e| anyhow::anyhow!("Failed to flush mem_buf to tempfile: {}", e))?;
            mem_buf = Vec::new(); // release memory
            temp_file = Some(tf);
        }

        match &mut temp_file {
            None => mem_buf.extend_from_slice(chunk),
            Some(tf) => tf
                .write_all(chunk)
                .map_err(|e| anyhow::anyhow!("Failed to write DATA chunk to tempfile: {}", e))?,
        }
    }

    // Final size is total_bytes (accumulated above).
    let message_size = total_bytes;

    // Check declared size if provided (RFC 1870 SIZE extension).
    if let Some(declared_size) = session.transaction.declared_size {
        let max_allowed = declared_size + (declared_size / 10);
        if message_size > max_allowed {
            tracing::info!(
                peer = %remote_addr,
                message_size = message_size,
                declared_size = declared_size,
                "smtp.session DATA rejected: size exceeds declared value"
            );
            rusmes_metrics::global_metrics().inc_smtp_messages_rejected();
            SmtpSessionHandler::write_response_to(
                writer,
                SmtpResponse::storage_exceeded(format!(
                    "Message size {} exceeds declared size {}",
                    message_size, declared_size
                )),
                remote_addr,
            )
            .await?;
            session.transaction.reset();
            session.state = SmtpState::Authenticated;
            return Ok(());
        }
    }

    // Build the MessageBody from whichever sink was active.
    // `cleanup_path` is Some(PathBuf) when the body was spilled to a tempfile;
    // the spawned task is responsible for deleting it after reading.
    let body_result = build_message_body(mem_buf, temp_file, message_size).await;
    let (message_body, cleanup_path) = match body_result {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(peer = %remote_addr, "Failed to finalise DATA body: {}", e);
            SmtpSessionHandler::write_response_to(
                writer,
                SmtpResponse::local_error("Internal error finalising message body"),
                remote_addr,
            )
            .await?;
            session.transaction.reset();
            session.state = SmtpState::Authenticated;
            return Ok(());
        }
    };

    // Message accepted — increment counter before sending 250 so the metric
    // is consistent even if the client drops after the acknowledgement.
    rusmes_metrics::global_metrics().inc_smtp_messages_received();

    session.transaction.message_size = message_size;

    let sender_display = session
        .transaction
        .sender
        .as_ref()
        .map(|a| a.to_string())
        .unwrap_or_else(|| "<unknown>".to_string());
    tracing::info!(
        peer = %remote_addr,
        mail_from = %sender_display,
        message_size = message_size,
        recipient_count = session.transaction.recipients.len(),
        "smtp.session DATA accepted: message queued"
    );

    // Send immediate acknowledgement to the client.
    SmtpSessionHandler::write_response_to(
        writer,
        SmtpResponse::ok("Message accepted for delivery"),
        remote_addr,
    )
    .await?;

    // Spawn asynchronous processing so the client is not blocked.
    let sender = session.transaction.sender.clone();
    let recipients = session.transaction.recipients.clone();
    let router = session.processor_router.clone();

    tracing::info!(
        peer = %remote_addr,
        recipient_count = recipients.len(),
        "smtp.session spawning async message processing task"
    );

    tokio::spawn(async move {
        if let Err(e) =
            process_accepted_message(sender, recipients, message_body, router, cleanup_path).await
        {
            tracing::error!("Failed to process message: {}", e);
        }
    });

    // Reset for the next transaction.
    session.transaction.reset();
    session.state = SmtpState::Authenticated;

    Ok(())
}

/// Finalise the hybrid sink into a [`rusmes_proto::MessageBody`] and an
/// optional cleanup path.
///
/// When the data stayed in memory, returns `(MessageBody::Small, None)`.
/// When it was spilled to a tempfile, flushes, keeps the file (moves it to a
/// stable temp path via [`tempfile::TempPath::keep`]), then opens it as
/// `MessageBody::Large`.  The caller is responsible for deleting the returned
/// `PathBuf` once the body has been consumed.
async fn build_message_body(
    mem_buf: Vec<u8>,
    temp_file: Option<NamedTempFile>,
    _message_size: usize,
) -> anyhow::Result<(rusmes_proto::MessageBody, Option<std::path::PathBuf>)> {
    use bytes::Bytes;
    use rusmes_proto::message::{LargeBody, MessageBody};

    match temp_file {
        None => Ok((MessageBody::Small(Bytes::from(mem_buf)), None)),
        Some(mut tf) => {
            tf.flush()
                .map_err(|e| anyhow::anyhow!("Failed to flush tempfile: {}", e))?;
            // Persist the tempfile so it survives until the spawned task reads it.
            let temp_path = tf.into_temp_path();
            let kept_path = temp_path
                .keep()
                .map_err(|e| anyhow::anyhow!("Failed to keep tempfile: {}", e))?;
            let large = LargeBody::from_path(&kept_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to open kept tempfile as LargeBody: {}", e))?;
            Ok((MessageBody::Large(large), Some(kept_path)))
        }
    }
}

/// Process an accepted message through the mail processor pipeline.
///
/// Handles both `MessageBody::Small` (in-memory) and `MessageBody::Large`
/// (tempfile-backed) inputs.  For large messages, all bytes are read into
/// memory for header parsing; this is a deliberate trade-off because the
/// [`rusmes_proto::mime::parse_headers`] API operates on a `&[u8]` slice.
/// Future work can stream header parsing to avoid this allocation.
///
/// `cleanup_path` — when `Some`, this is a tempfile path that was created by
/// the DATA spill logic.  It is deleted with [`tokio::fs::remove_file`] after
/// the body bytes have been consumed, regardless of whether subsequent
/// processing succeeds or fails.
async fn process_accepted_message(
    sender: Option<rusmes_proto::MailAddress>,
    recipients: Vec<rusmes_proto::MailAddress>,
    message_body: rusmes_proto::MessageBody,
    router: Arc<rusmes_core::MailProcessorRouter>,
    cleanup_path: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    use bytes::Bytes;
    use rusmes_proto::{HeaderMap, Mail, MessageBody, MimeMessage};

    tracing::info!(
        "Starting message processing for {} recipients",
        recipients.len()
    );

    // Materialise the full message bytes so we can parse MIME headers.
    // For Large bodies this reads from the tempfile; for Small it is zero-copy.
    let raw_bytes: Bytes = match message_body {
        MessageBody::Small(b) => b,
        MessageBody::Large(ref large) => large
            .read_to_bytes()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read large body for header parsing: {}", e))?,
    };

    // Clean up the spill tempfile as soon as the bytes are in memory.
    // We do this before routing so the file is removed even if routing fails.
    if let Some(ref path) = cleanup_path {
        if let Err(e) = tokio::fs::remove_file(path).await {
            // Log but do not abort — the message has already been read.
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "smtp.data Failed to remove DATA spill tempfile after read"
            );
        } else {
            tracing::debug!(
                path = %path.display(),
                "smtp.data DATA spill tempfile removed after read"
            );
        }
    }

    // Parse MIME headers from the raw bytes.
    let (headers, body_offset) = rusmes_proto::mime::parse_headers(&raw_bytes)?;

    let mut header_map = HeaderMap::new();
    for (name, value) in headers {
        header_map.insert(name, value);
    }

    let body_bytes = if body_offset < raw_bytes.len() {
        raw_bytes.slice(body_offset..)
    } else {
        Bytes::new()
    };

    let message = MimeMessage::new(header_map, MessageBody::Small(body_bytes));
    let mail = Mail::new(sender, recipients, message, None, None);

    tracing::info!("Processing mail {} through pipeline", mail.id());

    router.route(mail).await?;

    tracing::info!("Mail processing completed");
    Ok(())
}
