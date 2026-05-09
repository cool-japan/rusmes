//! Filesystem message helper utilities: serialization.
//!
//! Split out from `mod.rs` to keep that file under the 2000-line limit.

use rusmes_proto::Mail;

/// Serialize a [`Mail`] object to raw maildir bytes.
///
/// The message ID is prepended as an `X-Rusmes-Message-Id` header so it can
/// be recovered when reading files back from disk.
///
/// For `MessageBody::Large` bodies the bytes are read into memory via the
/// async reader before writing.  This is acceptable for the typical write-path
/// (once per delivery); the resulting bytes are then flushed to the maildir
/// file by the caller.
pub(super) async fn serialize_message_to_bytes(mail: &Mail) -> anyhow::Result<Vec<u8>> {
    let message = mail.message();
    let headers = message.headers();
    let body = message.body();

    let mut output = Vec::new();

    // Write custom header with MessageId for retrieval.
    // This is stored as X-Rusmes-Message-Id to avoid conflicts.
    output.extend_from_slice(b"X-Rusmes-Message-Id: ");
    output.extend_from_slice(mail.message_id().to_string().as_bytes());
    output.extend_from_slice(b"\r\n");

    // Write original headers.
    for (name, values) in headers.iter() {
        for value in values {
            output.extend_from_slice(name.as_bytes());
            output.extend_from_slice(b": ");
            output.extend_from_slice(value.as_bytes());
            output.extend_from_slice(b"\r\n");
        }
    }

    // Blank line separating headers from body.
    output.extend_from_slice(b"\r\n");

    // Write body.
    match body {
        rusmes_proto::MessageBody::Small(bytes) => {
            output.extend_from_slice(bytes);
        }
        rusmes_proto::MessageBody::Large(large) => {
            let data = large.read_to_bytes().await.map_err(|e| {
                anyhow::anyhow!("failed to read large message body for serialization: {e}")
            })?;
            output.extend_from_slice(&data);
        }
    }

    Ok(output)
}
