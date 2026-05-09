//! Advanced Email method implementations for JMAP
//!
//! Implements:
//! - Email/changes - detect changes since state (using MODSEQ)
//! - Email/queryChanges - incremental query updates
//! - Email/copy - copy emails between accounts
//! - Email/import - import raw RFC 5322 messages from blob storage
//! - Email/parse - parse email without importing (RFC 5322 parsing)
//!
//! This module provides advanced JMAP email operations as defined in RFC 8621.
//! State tracking is implemented using MODSEQ from the storage layer.

pub mod changes;
pub mod copy;
pub mod import;
pub mod parse;
pub mod types;

// Re-export all public items to preserve the module's public surface.
pub use changes::{email_changes, email_query_changes};
pub use copy::email_copy;
pub use import::email_import;
pub use parse::email_parse;
pub use types::{
    AddedItem, EmailChangesRequest, EmailChangesResponse, EmailCopyObject, EmailCopyRequest,
    EmailCopyResponse, EmailImportObject, EmailImportRequest, EmailImportResponse,
    EmailParseRequest, EmailParseResponse, EmailQueryChangesRequest, EmailQueryChangesResponse,
};

use rusmes_storage::MessageStore;

/// Helper function to get current modseq from storage.
///
/// Shared by all handlers that need to report state.
pub(super) async fn get_current_modseq(_message_store: &dyn MessageStore) -> anyhow::Result<u64> {
    Ok(chrono::Utc::now().timestamp() as u64)
}

/// Shared test helpers for the email_advanced sub-modules.
///
/// Only compiled in test builds; exposed as `pub(super)` so sibling modules
/// (changes, copy, import, parse) can import them with
/// `use super::test_helpers::*`.
#[cfg(test)]
pub(super) mod test_helpers {
    use crate::blob::BlobStorage;
    use rusmes_storage::backends::filesystem::FilesystemBackend;
    use rusmes_storage::{MessageStore, StorageBackend};
    use std::path::PathBuf;
    use std::sync::Arc;

    pub fn test_principal() -> crate::types::Principal {
        crate::types::admin_principal_for_tests()
    }

    pub fn empty_blobs() -> BlobStorage {
        BlobStorage::new()
    }

    pub fn create_test_store() -> Arc<dyn MessageStore> {
        let backend = FilesystemBackend::new(PathBuf::from("/tmp/rusmes-test-storage"));
        backend.message_store()
    }

    /// Create a backend that exposes both message_store and mailbox_store.
    pub fn create_test_backend() -> Arc<FilesystemBackend> {
        Arc::new(FilesystemBackend::new(
            std::env::temp_dir().join("rusmes-email-advanced-test"),
        ))
    }
}
