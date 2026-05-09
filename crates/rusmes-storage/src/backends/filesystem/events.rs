//! Storage event broadcasting for the filesystem backend.
//!
//! A `broadcast::Sender<StorageEvent>` is held in the `FilesystemBackend`
//! and shared (cloned) into every `FilesystemMessageStore`. After a write
//! commits (rename to `new/` or delete from `cur/`), the sender fires the
//! appropriate `StorageEvent`. Subscribers receive events via
//! `FilesystemBackend::event_stream()`.

use crate::StorageEvent;
use tokio::sync::broadcast;

/// Channel capacity: 256 events. Lagging subscribers drop the oldest events.
pub const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Create a new broadcast channel pair for storage events.
pub fn new_event_channel() -> (
    broadcast::Sender<StorageEvent>,
    broadcast::Receiver<StorageEvent>,
) {
    broadcast::channel(EVENT_CHANNEL_CAPACITY)
}

/// Fire a `MessageStored` event, logging any send failure.
pub fn fire_stored(
    tx: &broadcast::Sender<StorageEvent>,
    account: String,
    mailbox: String,
    uid: u32,
) {
    let event = StorageEvent::MessageStored {
        account,
        mailbox,
        uid,
    };
    if let Err(e) = tx.send(event) {
        tracing::debug!(
            "No active subscribers for StorageEvent::MessageStored: {}",
            e
        );
    }
}

/// Fire a `MessageExpunged` event, logging any send failure.
pub fn fire_expunged(
    tx: &broadcast::Sender<StorageEvent>,
    account: String,
    mailbox: String,
    uid: u32,
) {
    let event = StorageEvent::MessageExpunged {
        account,
        mailbox,
        uid,
    };
    if let Err(e) = tx.send(event) {
        tracing::debug!(
            "No active subscribers for StorageEvent::MessageExpunged: {}",
            e
        );
    }
}
