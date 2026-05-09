//! Filesystem directory locking for concurrent maildir access safety.
//!
//! Each maildir folder has a hidden `.rusmes.lock` file. Before any mutating
//! operation (deliver, expunge, copy), the caller acquires an exclusive lock on
//! that file using `fs2::FileExt::try_lock_exclusive`. On contention, a
//! geometric retry sequence is applied with a total deadline of 2 seconds,
//! after which `ConcurrencyConflictError` is returned.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

/// Error returned when a lock cannot be acquired within the retry deadline.
#[derive(Debug, Error)]
#[error("ConcurrencyConflict: could not acquire exclusive lock on {path} within 2s")]
pub struct ConcurrencyConflictError {
    pub path: String,
}

/// Acquired directory lock guard. Releases the lock on drop.
pub struct DirLock {
    _file: File,
}

/// Retry intervals: 50ms → 100ms → 250ms → 500ms → 1s (total ≤ 2s before error).
const RETRY_DELAYS: &[Duration] = &[
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1_000),
];

/// Acquire an exclusive directory lock for the given mailbox directory.
///
/// Creates `.rusmes.lock` inside `mailbox_dir` if it doesn't exist, then
/// attempts `try_lock_exclusive`. On contention, retries according to
/// `RETRY_DELAYS`. Returns `Err(ConcurrencyConflictError)` if all retries
/// are exhausted.
pub async fn acquire_dir_lock(mailbox_dir: &Path) -> anyhow::Result<DirLock> {
    use fs2::FileExt;

    let lock_path = mailbox_dir.join(".rusmes.lock");

    // Ensure the mailbox directory exists before creating the lockfile.
    tokio::fs::create_dir_all(mailbox_dir).await?;

    // Open or create the lockfile synchronously (fs2 operates on std::fs::File).
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(&lock_path)
        .map_err(|e| anyhow::anyhow!("Failed to open lockfile {}: {}", lock_path.display(), e))?;

    // Try to acquire the lock with retries (non-blocking each attempt).
    for &delay in RETRY_DELAYS {
        match file.try_lock_exclusive() {
            Ok(()) => {
                tracing::trace!("Acquired exclusive lock on {}", lock_path.display());
                return Ok(DirLock { _file: file });
            }
            Err(_) => {
                tracing::trace!(
                    "Lock contention on {}, retrying in {:?}",
                    lock_path.display(),
                    delay
                );
                tokio::time::sleep(delay).await;
            }
        }
    }

    // Final attempt after the last sleep.
    match file.try_lock_exclusive() {
        Ok(()) => Ok(DirLock { _file: file }),
        Err(_) => Err(anyhow::anyhow!(ConcurrencyConflictError {
            path: lock_path.display().to_string(),
        })),
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        // Use the fs2 trait method explicitly via UFCS so we don't accidentally
        // pick up `std::fs::File::unlock` (which requires Rust 1.89+; our MSRV
        // is 1.75.0).
        if let Err(e) = fs2::FileExt::unlock(&self._file) {
            tracing::warn!("Failed to release directory lock: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lock_acquire_and_release() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = acquire_dir_lock(dir.path()).await;
        assert!(lock.is_ok(), "Should acquire lock");
        drop(lock.unwrap());

        // After releasing, should be acquirable again.
        let lock2 = acquire_dir_lock(dir.path()).await;
        assert!(lock2.is_ok(), "Should acquire lock after release");
    }

    #[tokio::test]
    async fn test_lock_file_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _lock = acquire_dir_lock(dir.path()).await.expect("lock");
        assert!(dir.path().join(".rusmes.lock").exists());
    }
}
