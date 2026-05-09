//! Filesystem backup and restore using OxiARC ZIP archives.
//!
//! `backup_fs_dir` archives the entire filesystem backend root into a
//! single OxiARC ZIP archive at `dest`. `restore_fs_dir` extracts a
//! previously created archive at `src` into `dest_dir`.
//!
//! These functions operate on raw directory trees and are independent of the
//! in-memory state of a running backend — the caller should ensure the backend
//! is quiesced before backup (or accept a crash-consistent snapshot).

use oxiarc_archive::zip::{ZipReader, ZipWriter};
use std::path::Path;

/// Archive the entire `src_dir` (filesystem backend root) into `dest_archive`.
///
/// The archive is created using OxiARC's ZIP format with default compression.
/// Each file is stored with its path relative to `src_dir`.
pub async fn backup_fs_dir(src_dir: &Path, dest_archive: &Path) -> anyhow::Result<()> {
    let src_dir = src_dir.to_path_buf();
    let dest_archive = dest_archive.to_path_buf();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        // Collect all files under src_dir.
        let mut entries: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
        for entry in walkdir::WalkDir::new(&src_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let abs = entry.path().to_path_buf();
            let rel = abs
                .strip_prefix(&src_dir)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| abs.clone());
            entries.push((abs, rel));
        }

        // Ensure dest directory exists.
        if let Some(parent) = dest_archive.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::File::create(&dest_archive)?;
        let mut writer = ZipWriter::new(file);
        for (abs_path, rel_path) in entries {
            let data = std::fs::read(&abs_path)?;
            let rel_str = rel_path.to_string_lossy();
            writer
                .add_file(rel_str.as_ref(), &data)
                .map_err(|e| anyhow::anyhow!("ZipWriter::add_file failed: {}", e))?;
        }
        writer
            .finish()
            .map_err(|e| anyhow::anyhow!("ZipWriter::finish failed: {}", e))?;

        tracing::info!(
            "Backup complete: {} → {}",
            src_dir.display(),
            dest_archive.display()
        );
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {}", e))?
}

/// Extract an OxiARC ZIP archive `src_archive` into `dest_dir`.
///
/// Existing files in `dest_dir` are overwritten.
pub async fn restore_fs_dir(src_archive: &Path, dest_dir: &Path) -> anyhow::Result<()> {
    let src_archive = src_archive.to_path_buf();
    let dest_dir = dest_dir.to_path_buf();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        std::fs::create_dir_all(&dest_dir)?;

        let file = std::fs::File::open(&src_archive)?;
        let mut reader =
            ZipReader::new(file).map_err(|e| anyhow::anyhow!("ZipReader::new failed: {}", e))?;
        let entries = reader.entries().to_vec();
        for entry in entries {
            let dest_path = dest_dir.join(&entry.name);

            // Create parent directories as needed.
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let data = reader
                .extract(&entry)
                .map_err(|e| anyhow::anyhow!("ZipReader::extract failed: {}", e))?;
            std::fs::write(&dest_path, &data)?;
        }

        tracing::info!(
            "Restore complete: {} → {}",
            src_archive.display(),
            dest_dir.display()
        );
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_backup_restore_roundtrip() {
        let src_dir = tempfile::tempdir().expect("src tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let archive_dir = tempfile::tempdir().expect("archive tempdir");
        let archive_path = archive_dir.path().join("backup.zip");

        // Create some test files.
        tokio::fs::create_dir_all(src_dir.path().join("mailboxes/inbox/new"))
            .await
            .expect("create_dir_all should succeed");
        tokio::fs::write(
            src_dir.path().join("mailboxes/inbox/new/msg1"),
            b"message 1",
        )
        .await
        .expect("write msg1 should succeed");
        tokio::fs::write(
            src_dir.path().join("mailboxes/inbox/new/msg2"),
            b"message 2",
        )
        .await
        .expect("write msg2 should succeed");

        // Backup.
        backup_fs_dir(src_dir.path(), &archive_path)
            .await
            .expect("backup should succeed");
        assert!(archive_path.exists(), "Archive file should exist");

        // Restore.
        restore_fs_dir(&archive_path, dest_dir.path())
            .await
            .expect("restore should succeed");

        // Verify.
        let restored1 = tokio::fs::read(dest_dir.path().join("mailboxes/inbox/new/msg1"))
            .await
            .expect("msg1 should exist after restore");
        assert_eq!(restored1, b"message 1");

        let restored2 = tokio::fs::read(dest_dir.path().join("mailboxes/inbox/new/msg2"))
            .await
            .expect("msg2 should exist after restore");
        assert_eq!(restored2, b"message 2");
    }
}
