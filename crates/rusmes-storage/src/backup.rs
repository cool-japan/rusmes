//! Backup and restore library API for rusmes storage backends.
//!
//! `backup` archives all mailbox data for a filesystem backend into an OxiARC
//! archive at `dest`. `restore` extracts a previously created archive and
//! reconstructs the backend storage at `src`'s root.
//!
//! For non-filesystem backends (postgres, amaters) these functions return
//! `Err(anyhow::anyhow!("backup not yet supported for this backend type"))`.
//! Full backup for postgres will be added in a future iteration (pg_dump
//! integration or WAL-based streaming).
//!
//! Per the COOLJAPAN Pure Rust Policy, compression is handled exclusively by
//! OxiARC; `flate2`, `zip`, `zstd`, `tar`, and `brotli` are **never** used.

use crate::StorageBackend;
use std::path::Path;

/// Archive all data managed by `backend` into the OxiARC archive at `dest`.
///
/// Currently supports filesystem backends only.
pub async fn backup(backend: &dyn StorageBackend, dest: &Path) -> anyhow::Result<()> {
    // Check if the backend is a FilesystemBackend via the as_filesystem_path
    // hook on the trait. Since we can't downcast trait objects directly, the
    // backend opts in by returning `Some(path)` from `as_filesystem_path()`.
    if let Some(path) = backend.as_filesystem_path() {
        return crate::backends::filesystem::backup::backup_fs_dir(path, dest).await;
    }

    Err(anyhow::anyhow!(
        "backup: not yet supported for this backend type (only filesystem is supported)"
    ))
}

/// Restore backend data from the OxiARC archive at `src` into `dest_dir`.
///
/// Currently supports filesystem backends only.
pub async fn restore(backend: &dyn StorageBackend, src: &Path) -> anyhow::Result<()> {
    if let Some(path) = backend.as_filesystem_path() {
        return crate::backends::filesystem::backup::restore_fs_dir(src, path).await;
    }

    Err(anyhow::anyhow!(
        "restore: not yet supported for this backend type (only filesystem is supported)"
    ))
}
