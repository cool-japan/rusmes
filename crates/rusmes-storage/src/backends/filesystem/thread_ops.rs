//! Thread ID lookup operations for the filesystem backend.
//!
//! Extracted from `mod.rs` to keep that file under the 2 000-line limit.

use super::threading::{self, ThreadingEngine};
use rusmes_proto::MessageId;
use std::path::Path;
use std::str::FromStr;

/// Scan all mailbox sub-directories under `mailboxes_dir` to find the message
/// identified by `message_id`, then return its thread ID from the per-mailbox
/// `.thread_index.json`.
///
/// Returns `Ok(None)` if the message is not found or has no thread ID stored.
pub(super) async fn scan_for_thread_id(
    mailboxes_dir: &Path,
    message_id: &MessageId,
) -> anyhow::Result<Option<String>> {
    if !tokio::fs::try_exists(mailboxes_dir).await.unwrap_or(false) {
        return Ok(None);
    }

    let mut mailbox_entries = tokio::fs::read_dir(mailboxes_dir).await?;
    while let Some(mbx_entry) = mailbox_entries.next_entry().await? {
        let mbx_file_type = match mbx_entry.file_type().await {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !mbx_file_type.is_dir() {
            continue;
        }
        let mailbox_dir = mbx_entry.path();

        for subdir in ["new", "cur"] {
            let msg_dir = mailbox_dir.join(subdir);
            if !tokio::fs::try_exists(&msg_dir).await.unwrap_or(false) {
                continue;
            }
            let mut msg_entries = match tokio::fs::read_dir(&msg_dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Some(msg_entry) = msg_entries.next_entry().await? {
                let msg_ft = match msg_entry.file_type().await {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if !msg_ft.is_file() {
                    continue;
                }

                let file_path = msg_entry.path();
                let data = match tokio::fs::read(&file_path).await {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let mime = match rusmes_proto::MimeMessage::parse_from_bytes(&data) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let stored_id =
                    mime.headers()
                        .get_first("x-rusmes-message-id")
                        .and_then(|id_str| {
                            uuid::Uuid::from_str(id_str.trim())
                                .ok()
                                .map(MessageId::from_uuid)
                        });

                if stored_id.as_ref() == Some(message_id) {
                    // Found the message; look up its RFC 5322 Message-ID in the index.
                    if let Some(rfc_id) = mime
                        .headers()
                        .get_first("message-id")
                        .map(threading::strip_angle_brackets)
                    {
                        let engine = ThreadingEngine::new(&mailbox_dir);
                        return engine.get_thread_id(&rfc_id).await;
                    }
                    return Ok(None);
                }
            }
        }
    }

    Ok(None)
}
