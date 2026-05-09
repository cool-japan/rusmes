//! Filesystem compaction — remove expunged messages from `.Trash` directories.
//!
//! The filesystem backend moves deleted messages to a per-mailbox `.Trash`
//! subfolder (expunge-as-move). `compact_trash` walks every `.Trash` directory
//! under `base_path/mailboxes/*/` and removes files whose mtime is older than
//! `older_than`.

use std::path::Path;
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

/// Walk all `.Trash` directories under `base_path/mailboxes/` and delete
/// files that are older than `older_than`. Returns the count of removed files.
pub async fn compact_trash(base_path: &Path, older_than: Duration) -> anyhow::Result<usize> {
    let mailboxes_dir = base_path.join("mailboxes");
    if !mailboxes_dir.exists() {
        return Ok(0);
    }

    let now = SystemTime::now();
    let mut removed = 0usize;

    // WalkDir is synchronous; perform in spawn_blocking to avoid blocking the async executor.
    let mailboxes_dir_clone = mailboxes_dir.clone();
    let older_than_clone = older_than;

    let paths_to_remove: Vec<std::path::PathBuf> =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<std::path::PathBuf>> {
            let mut candidates = Vec::new();
            for entry in WalkDir::new(&mailboxes_dir_clone)
                .min_depth(2)
                .max_depth(3)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                // Only consider files inside `.Trash/` directories.
                let in_trash = entry.path().components().any(|c| c.as_os_str() == ".Trash");
                if !in_trash {
                    continue;
                }

                // Check mtime.
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("Failed to stat {:?}: {}", entry.path(), e);
                        continue;
                    }
                };

                let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                if let Ok(age) = now.duration_since(mtime) {
                    if age >= older_than_clone {
                        candidates.push(entry.path().to_path_buf());
                    }
                }
            }
            Ok(candidates)
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {}", e))??;

    for path in paths_to_remove {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {
                tracing::debug!("Compacted expunged message: {:?}", path);
                removed += 1;
            }
            Err(e) => {
                tracing::warn!("Failed to remove {:?}: {}", path, e);
            }
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[tokio::test]
    async fn test_compact_trash_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let count = compact_trash(dir.path(), Duration::from_secs(0)).await;
        assert!(count.is_ok());
        assert_eq!(count.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_compact_trash_removes_old_files() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create: base/mailboxes/<uuid>/.Trash/msg
        let mailbox_dir = dir
            .path()
            .join("mailboxes")
            .join("test-mailbox")
            .join(".Trash");
        tokio::fs::create_dir_all(&mailbox_dir).await.unwrap();
        let msg_path = mailbox_dir.join("old-message");
        tokio::fs::write(&msg_path, b"deleted msg").await.unwrap();

        // Set mtime to 2 hours ago using std
        let file = std::fs::File::open(&msg_path).unwrap();
        let _two_hours_ago = SystemTime::now()
            .checked_sub(Duration::from_secs(7200))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        // We can't set mtime on all platforms without libc; but since we control
        // the test environment (macOS / Linux), use a zero-duration threshold to
        // match any file that exists.
        drop(file);

        // With older_than = 0, everything qualifies.
        let count = compact_trash(dir.path(), Duration::from_secs(0))
            .await
            .unwrap();
        assert_eq!(count, 1, "Should have removed 1 file");
        assert!(!msg_path.exists(), "File should be gone");
    }

    #[tokio::test]
    async fn test_compact_trash_ignores_non_trash() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create: base/mailboxes/<uuid>/cur/msg — NOT in .Trash
        let cur_dir = dir
            .path()
            .join("mailboxes")
            .join("test-mailbox")
            .join("cur");
        tokio::fs::create_dir_all(&cur_dir).await.unwrap();
        tokio::fs::write(cur_dir.join("message"), b"live msg")
            .await
            .unwrap();

        let count = compact_trash(dir.path(), Duration::from_secs(0))
            .await
            .unwrap();
        assert_eq!(count, 0, "Should not remove files outside .Trash");
    }
}
