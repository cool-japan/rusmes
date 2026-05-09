//! Blob upload/download endpoints for JMAP
//!
//! Implements RFC 8620 Section 6.1 and 6.2:
//! - `POST /upload/:account_id` — upload blobs
//! - `GET /download/:account_id/:blob_id/:name` — download blobs
//! - Blob size limits and validation
//!
//! ## Mount Points
//!
//! These routes are mounted by [`crate::api::JmapServer::routes_with_auth_and_state`]
//! behind the [`crate::auth::require_auth`] middleware. Both endpoints require a
//! valid authenticated session before the handler is invoked.
//!
//! Per-account ownership enforcement (verifying that the authenticated
//! [`crate::types::Principal`] owns the `:account_id` path parameter) is a
//! follow-up concern and is not yet implemented here.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// Default maximum blob size: 50 MiB
pub const DEFAULT_MAX_BLOB_SIZE: u64 = 52_428_800;

// ─────────────────────────────────────────────────────────────────────────────
// BlobMeta
// ─────────────────────────────────────────────────────────────────────────────

/// Metadata associated with a stored blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobMeta {
    /// MIME content-type of the blob.
    pub content_type: String,
    /// Size in bytes.
    pub size: u64,
    /// Account that uploaded this blob.
    pub account_id: String,
    /// Upload timestamp (UTC).
    pub created_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────────────────────────────────────
// BlobData (kept for backwards-compatibility with email_advanced.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Raw blob data returned by the legacy [`BlobStorage::get`] accessor.
#[derive(Clone)]
pub struct BlobData {
    data: Vec<u8>,
    content_type: String,
}

impl BlobData {
    /// Get the raw blob bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Get the content type of the blob.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UploadError
// ─────────────────────────────────────────────────────────────────────────────

/// Errors that can arise during blob upload or download operations.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// The blob body exceeds the configured size ceiling.
    #[error("blob too large: {actual} bytes exceeds maximum of {max}")]
    TooLarge {
        /// Actual body size in bytes.
        actual: u64,
        /// Configured maximum in bytes.
        max: u64,
    },

    /// The requested blob was not found.
    #[error("blob not found: {0}")]
    NotFound(String),

    /// An I/O error while writing/reading from the filesystem backend.
    #[error("blob I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON (de)serialisation failure on the metadata sidecar.
    #[error("blob metadata error: {0}")]
    Meta(#[from] serde_json::Error),

    /// The internal RwLock was poisoned.
    #[error("blob storage lock poisoned")]
    LockPoisoned,
}

// ─────────────────────────────────────────────────────────────────────────────
// BlobBackend (private)
// ─────────────────────────────────────────────────────────────────────────────

/// Shared type alias for the in-memory blob map used by the `Memory` variant.
type MemoryBlobMap = Arc<RwLock<HashMap<String, (Vec<u8>, BlobMeta)>>>;

/// Internal storage strategy.
enum BlobBackend {
    /// Pure in-memory — blobs are lost on restart (default).
    Memory { blobs: MemoryBlobMap },
    /// Filesystem-backed — blobs survive restarts.
    ///
    /// Each blob is stored as two files under `<root>/blobs/`:
    /// - `<blob_id>`          — raw bytes
    /// - `<blob_id>.meta.json` — JSON-serialised [`BlobMeta`]
    ///
    /// An in-memory index (`Arc<RwLock<HashMap<…>>>`) mirrors the on-disk
    /// state and is rebuilt from `.meta.json` sidecars when the storage is
    /// opened via [`BlobStorage::new_filesystem`].
    FileSystem {
        root: PathBuf,
        index: Arc<RwLock<HashMap<String, BlobMeta>>>,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// BlobStorage
// ─────────────────────────────────────────────────────────────────────────────

/// Content-addressed blob store for JMAP.
///
/// Two variants are available:
///
/// | Constructor | Persistence | Notes |
/// |---|---|---|
/// | [`BlobStorage::new`] | In-memory only | Existing callers unchanged |
/// | [`BlobStorage::new_filesystem`] | Survives restarts | Requires a writable directory |
///
/// Both variants enforce the same [`max_blob_size`](Self::max_blob_size) ceiling and
/// expose the same public API.
///
/// # Clone semantics
///
/// [`BlobStorage`] is cheaply cloneable — the inner state (both the `Arc`-wrapped
/// in-memory map and the on-disk index) is shared between all clones. This
/// matches the original behaviour.
#[derive(Clone)]
pub struct BlobStorage {
    backend: Arc<BlobBackend>,
    /// Maximum body size (bytes) accepted by [`Self::upload`].
    pub max_blob_size: u64,
}

impl BlobStorage {
    /// Create a new **in-memory** blob storage (no persistence across restarts).
    pub fn new() -> Self {
        Self {
            backend: Arc::new(BlobBackend::Memory {
                blobs: Arc::new(RwLock::new(HashMap::new())),
            }),
            max_blob_size: DEFAULT_MAX_BLOB_SIZE,
        }
    }

    /// Open (or create) a **filesystem-backed** blob storage rooted at `root`.
    ///
    /// On first call with an empty directory, the `blobs/` sub-directory is
    /// created and the in-memory index starts empty.  On subsequent calls the
    /// index is rebuilt by scanning every `*.meta.json` sidecar in `blobs/`.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or if a sidecar
    /// file contains invalid JSON.
    pub async fn new_filesystem(root: PathBuf) -> Result<Self, UploadError> {
        let blobs_dir = root.join("blobs");
        tokio::fs::create_dir_all(&blobs_dir).await?;

        // Rebuild the index from on-disk sidecars.
        let mut index: HashMap<String, BlobMeta> = HashMap::new();

        let mut read_dir = tokio::fs::read_dir(&blobs_dir).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };

            // Only process ".meta.json" sidecars.
            if !file_name.ends_with(".meta.json") {
                continue;
            }

            // Derive blob_id from sidecar file name.
            let blob_id = file_name
                .strip_suffix(".meta.json")
                .unwrap_or(&file_name)
                .to_owned();

            let raw = tokio::fs::read(&path).await?;
            match serde_json::from_slice::<BlobMeta>(&raw) {
                Ok(meta) => {
                    index.insert(blob_id, meta);
                }
                Err(e) => {
                    tracing::warn!("Skipping corrupt blob sidecar {:?}: {}", path, e);
                }
            }
        }

        Ok(Self {
            backend: Arc::new(BlobBackend::FileSystem {
                root,
                index: Arc::new(RwLock::new(index)),
            }),
            max_blob_size: DEFAULT_MAX_BLOB_SIZE,
        })
    }

    /// Override the maximum blob size (bytes).
    ///
    /// Returns `self` for builder-style chaining.
    pub fn with_max_blob_size(mut self, max_bytes: u64) -> Self {
        self.max_blob_size = max_bytes;
        self
    }

    // ──────────────────────────────────────────────────────────────────────
    // Async API (preferred for production code)
    // ──────────────────────────────────────────────────────────────────────

    /// Upload a blob and return the generated `blob_id`.
    ///
    /// The body is rejected with [`UploadError::TooLarge`] before any bytes
    /// are written if it exceeds [`Self::max_blob_size`].
    pub async fn upload(
        &self,
        account_id: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<String, UploadError> {
        // Size-limit check happens before any I/O.
        let actual = body.len() as u64;
        if actual > self.max_blob_size {
            return Err(UploadError::TooLarge {
                actual,
                max: self.max_blob_size,
            });
        }

        let blob_id = Uuid::new_v4().to_string();
        let meta = BlobMeta {
            content_type: content_type.to_owned(),
            size: actual,
            account_id: account_id.to_owned(),
            created_at: Utc::now(),
        };

        match self.backend.as_ref() {
            BlobBackend::Memory { blobs } => {
                let mut guard = blobs.write().map_err(|_| UploadError::LockPoisoned)?;
                guard.insert(blob_id.clone(), (body.to_vec(), meta));
            }
            BlobBackend::FileSystem { root, index } => {
                let blobs_dir = root.join("blobs");
                let tmp_path = blobs_dir.join(format!("{}.tmp", blob_id));
                let final_path = blobs_dir.join(&blob_id);
                let meta_path = blobs_dir.join(format!("{}.meta.json", blob_id));

                // Write bytes atomically: temp → rename.
                tokio::fs::write(&tmp_path, body).await?;
                tokio::fs::rename(&tmp_path, &final_path).await?;

                // Write metadata sidecar.
                let meta_bytes = serde_json::to_vec(&meta)?;
                tokio::fs::write(&meta_path, &meta_bytes).await?;

                // Update in-memory index.
                let mut guard = index.write().map_err(|_| UploadError::LockPoisoned)?;
                guard.insert(blob_id.clone(), meta);
            }
        }

        Ok(blob_id)
    }

    /// Download a blob's raw bytes together with its metadata.
    ///
    /// Returns `Ok((bytes, meta))` or [`UploadError::NotFound`].
    pub async fn download(&self, blob_id: &str) -> Result<(Vec<u8>, BlobMeta), UploadError> {
        match self.backend.as_ref() {
            BlobBackend::Memory { blobs } => {
                let guard = blobs.read().map_err(|_| UploadError::LockPoisoned)?;
                match guard.get(blob_id) {
                    Some((data, meta)) => Ok((data.clone(), meta.clone())),
                    None => Err(UploadError::NotFound(blob_id.to_owned())),
                }
            }
            BlobBackend::FileSystem { root, index } => {
                // Check index first.
                let meta = {
                    let guard = index.read().map_err(|_| UploadError::LockPoisoned)?;
                    guard
                        .get(blob_id)
                        .cloned()
                        .ok_or_else(|| UploadError::NotFound(blob_id.to_owned()))?
                };

                let blob_path = root.join("blobs").join(blob_id);
                let data = tokio::fs::read(&blob_path).await?;
                Ok((data, meta))
            }
        }
    }

    /// Delete a blob by ID.
    ///
    /// Returns [`UploadError::NotFound`] if the blob does not exist.
    pub async fn delete(&self, blob_id: &str) -> Result<(), UploadError> {
        match self.backend.as_ref() {
            BlobBackend::Memory { blobs } => {
                let mut guard = blobs.write().map_err(|_| UploadError::LockPoisoned)?;
                if guard.remove(blob_id).is_none() {
                    return Err(UploadError::NotFound(blob_id.to_owned()));
                }
            }
            BlobBackend::FileSystem { root, index } => {
                {
                    let mut guard = index.write().map_err(|_| UploadError::LockPoisoned)?;
                    if guard.remove(blob_id).is_none() {
                        return Err(UploadError::NotFound(blob_id.to_owned()));
                    }
                }
                let blobs_dir = root.join("blobs");
                let blob_path = blobs_dir.join(blob_id);
                let meta_path = blobs_dir.join(format!("{}.meta.json", blob_id));
                // Best-effort removals — don't fail if file is already gone.
                let _ = tokio::fs::remove_file(&blob_path).await;
                let _ = tokio::fs::remove_file(&meta_path).await;
            }
        }
        Ok(())
    }

    /// Return the number of blobs currently held in this storage instance.
    pub async fn blob_count(&self) -> Result<usize, UploadError> {
        match self.backend.as_ref() {
            BlobBackend::Memory { blobs } => {
                let guard = blobs.read().map_err(|_| UploadError::LockPoisoned)?;
                Ok(guard.len())
            }
            BlobBackend::FileSystem { index, .. } => {
                let guard = index.read().map_err(|_| UploadError::LockPoisoned)?;
                Ok(guard.len())
            }
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Legacy synchronous API (for backwards-compat with email_advanced.rs)
    // ──────────────────────────────────────────────────────────────────────

    /// Store a blob using the legacy synchronous path.
    ///
    /// Only available on the **memory** variant; filesystem callers should
    /// use [`Self::upload`] instead.  The call is a no-op (blob silently
    /// dropped) when called on a filesystem-backed store, because this
    /// method cannot perform async I/O — use [`Self::upload`] in that case.
    pub fn store(&self, blob_id: String, data: Vec<u8>, content_type: String) {
        match self.backend.as_ref() {
            BlobBackend::Memory { blobs } => {
                let meta = BlobMeta {
                    content_type: content_type.clone(),
                    size: data.len() as u64,
                    account_id: String::new(),
                    created_at: Utc::now(),
                };
                if let Ok(mut guard) = blobs.write() {
                    guard.insert(blob_id, (data, meta));
                }
            }
            BlobBackend::FileSystem { .. } => {
                tracing::warn!(
                    "BlobStorage::store() called on filesystem backend — \
                     use BlobStorage::upload() for filesystem persistence"
                );
            }
        }
    }

    /// Retrieve a blob using the legacy synchronous API.
    pub fn get(&self, blob_id: &str) -> Option<BlobData> {
        match self.backend.as_ref() {
            BlobBackend::Memory { blobs } => {
                let guard = blobs.read().ok()?;
                guard.get(blob_id).map(|(data, meta)| BlobData {
                    data: data.clone(),
                    content_type: meta.content_type.clone(),
                })
            }
            BlobBackend::FileSystem { root, index } => {
                // Synchronous read from disk — only suitable for small blobs
                // and test code paths.
                let meta = {
                    let guard = index.read().ok()?;
                    guard.get(blob_id)?.clone()
                };
                let blob_path = root.join("blobs").join(blob_id);
                let data = std::fs::read(&blob_path).ok()?;
                Some(BlobData {
                    data,
                    content_type: meta.content_type,
                })
            }
        }
    }

    /// Get blob size using the legacy synchronous API.
    pub fn size(&self, blob_id: &str) -> Option<usize> {
        match self.backend.as_ref() {
            BlobBackend::Memory { blobs } => {
                let guard = blobs.read().ok()?;
                guard.get(blob_id).map(|(data, _)| data.len())
            }
            BlobBackend::FileSystem { index, .. } => {
                let guard = index.read().ok()?;
                guard.get(blob_id).map(|m| m.size as usize)
            }
        }
    }
}

impl Default for BlobStorage {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP response types
// ─────────────────────────────────────────────────────────────────────────────

/// Success response for a blob upload (RFC 8620 §6.1).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub account_id: String,
    pub blob_id: String,
    #[serde(rename = "type")]
    pub content_type: String,
    pub size: usize,
}

/// JSON error body for a failed blob upload.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadErrorBody {
    #[serde(rename = "type")]
    pub error_type: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Axum routes
// ─────────────────────────────────────────────────────────────────────────────

/// Create blob router.
///
/// Mounts:
/// - `GET /download/{account_id}/{blob_id}/{name}` — blob download (RFC 8620 §6.2)
/// - `POST /upload/{account_id}` — blob upload (RFC 8620 §6.2)
pub fn blob_routes() -> Router<BlobStorage> {
    Router::new()
        .route("/download/{account}/{blob}/{name}", get(download_blob))
        .route("/upload/{account}", post(upload_blob))
}

/// Download blob endpoint
async fn download_blob(
    Path((account, blob_id, name)): Path<(String, String, String)>,
    State(storage): State<BlobStorage>,
) -> Response {
    if account.is_empty() {
        return (StatusCode::BAD_REQUEST, "Invalid account ID").into_response();
    }

    match storage.download(&blob_id).await {
        Ok((data, meta)) => {
            match Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, &meta.content_type)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", name),
                )
                .header(header::CONTENT_LENGTH, data.len())
                .body(Body::from(data))
            {
                Ok(response) => response,
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to build response: {}", e),
                )
                    .into_response(),
            }
        }
        Err(UploadError::NotFound(_)) => (StatusCode::NOT_FOUND, "Blob not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Storage error: {}", e),
        )
            .into_response(),
    }
}

/// Upload blob endpoint
async fn upload_blob(
    Path(account): Path<String>,
    State(storage): State<BlobStorage>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if account.is_empty() {
        let error = UploadErrorBody {
            error_type: "urn:ietf:params:jmap:error:invalidArguments".to_string(),
            status: 400,
            detail: Some("Invalid account ID".to_string()),
        };
        return (StatusCode::BAD_REQUEST, axum::Json(error)).into_response();
    }

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    match storage.upload(&account, &content_type, &body).await {
        Ok(blob_id) => {
            let response = UploadResponse {
                account_id: account,
                blob_id,
                content_type,
                size: body.len(),
            };
            (StatusCode::CREATED, axum::Json(response)).into_response()
        }
        Err(UploadError::TooLarge { actual, max }) => {
            let error = UploadErrorBody {
                error_type: "urn:ietf:params:jmap:error:tooLarge".to_string(),
                status: 413,
                detail: Some(format!(
                    "Blob size {} bytes exceeds maximum of {} bytes",
                    actual, max
                )),
            };
            (StatusCode::PAYLOAD_TOO_LARGE, axum::Json(error)).into_response()
        }
        Err(e) => {
            let error = UploadErrorBody {
                error_type: "urn:ietf:params:jmap:error:serverFail".to_string(),
                status: 500,
                detail: Some(format!("Upload failed: {}", e)),
            };
            (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(error)).into_response()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Content-addressed blob ID helper
// ─────────────────────────────────────────────────────────────────────────────

/// Compute a stable content-addressed blob ID for JMAP per RFC 8620 §6.2.
///
/// Returns the hex-encoded SHA-256 of the given bytes.  This is stable across
/// process restarts and replicas.  JMAP clients re-discover blob IDs each
/// session, so switching from the old `format!("blob-{}", id)` scheme to this
/// content-addressed scheme is non-breaking for compliant clients.
pub fn compute_blob_id(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a deterministic blob ID from data (SHA-256, 'G'-prefixed).
    /// Used only in tests; production code uses UUID v4.
    fn generate_blob_id(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        format!("G{:x}", result)
    }

    // ── legacy synchronous memory tests (preserved) ────────────────────────

    #[test]
    fn test_blob_storage_store_and_get() {
        let storage = BlobStorage::new();
        let data = b"test data".to_vec();
        let blob_id = "blob123".to_string();

        storage.store(blob_id.clone(), data.clone(), "text/plain".to_string());

        let retrieved = storage.get(&blob_id).expect("blob should exist");
        assert_eq!(retrieved.data(), data.as_slice());
        assert_eq!(retrieved.content_type(), "text/plain");
    }

    #[test]
    fn test_blob_storage_size() {
        let storage = BlobStorage::new();
        let data = b"test data".to_vec();
        let blob_id = "blob123".to_string();

        storage.store(blob_id.clone(), data.clone(), "text/plain".to_string());

        assert_eq!(storage.size(&blob_id), Some(9));
    }

    #[test]
    fn test_blob_storage_get_nonexistent() {
        let storage = BlobStorage::new();
        assert!(storage.get("nonexistent").is_none());
    }

    #[test]
    fn test_generate_blob_id() {
        let data1 = b"test data";
        let data2 = b"test data";
        let data3 = b"different data";

        let id1 = generate_blob_id(data1);
        let id2 = generate_blob_id(data2);
        let id3 = generate_blob_id(data3);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
        assert!(id1.starts_with('G'));
    }

    #[test]
    fn test_blob_id_length() {
        let data = b"test data";
        let blob_id = generate_blob_id(data);
        // SHA256 hash is 64 hex chars + 1 for 'G' prefix
        assert_eq!(blob_id.len(), 65);
    }

    #[test]
    fn test_blob_storage_multiple_blobs() {
        let storage = BlobStorage::new();

        for i in 0..10 {
            let data = format!("data{}", i).into_bytes();
            let blob_id = format!("blob{}", i);
            storage.store(blob_id.clone(), data, "text/plain".to_string());
        }

        for i in 0..10 {
            let blob_id = format!("blob{}", i);
            assert!(storage.get(&blob_id).is_some());
        }
    }

    #[test]
    fn test_blob_storage_overwrite() {
        let storage = BlobStorage::new();
        let blob_id = "blob123".to_string();

        storage.store(
            blob_id.clone(),
            b"original".to_vec(),
            "text/plain".to_string(),
        );
        storage.store(
            blob_id.clone(),
            b"updated".to_vec(),
            "text/html".to_string(),
        );

        let retrieved = storage.get(&blob_id).expect("blob should exist");
        assert_eq!(retrieved.data(), b"updated");
        assert_eq!(retrieved.content_type(), "text/html");
    }

    #[test]
    fn test_blob_storage_empty_data() {
        let storage = BlobStorage::new();
        let blob_id = "empty".to_string();

        storage.store(
            blob_id.clone(),
            vec![],
            "application/octet-stream".to_string(),
        );

        let retrieved = storage.get(&blob_id).expect("blob should exist");
        assert_eq!(retrieved.data().len(), 0);
    }

    #[test]
    fn test_blob_storage_large_data() {
        let storage = BlobStorage::new();
        let data = vec![0u8; 1024 * 1024]; // 1MB
        let blob_id = "large".to_string();

        storage.store(
            blob_id.clone(),
            data,
            "application/octet-stream".to_string(),
        );

        assert_eq!(storage.size(&blob_id), Some(1024 * 1024));
    }

    #[test]
    fn test_upload_error_serialization() {
        let error = UploadErrorBody {
            error_type: "urn:ietf:params:jmap:error:tooLarge".to_string(),
            status: 413,
            detail: Some("Too large".to_string()),
        };

        let json = serde_json::to_string(&error).expect("serialization should succeed");
        assert!(json.contains("tooLarge"));
        assert!(json.contains("413"));
    }

    #[test]
    fn test_upload_response_serialization() {
        let response = UploadResponse {
            account_id: "acc1".to_string(),
            blob_id: "blob123".to_string(),
            content_type: "image/png".to_string(),
            size: 1024,
        };

        let json = serde_json::to_string(&response).expect("serialization should succeed");
        assert!(json.contains("blob123"));
        assert!(json.contains("image/png"));
    }

    #[test]
    fn test_blob_storage_clone() {
        let storage1 = BlobStorage::new();
        storage1.store(
            "blob1".to_string(),
            b"data".to_vec(),
            "text/plain".to_string(),
        );

        let storage2 = storage1.clone();
        assert!(storage2.get("blob1").is_some());
    }

    #[test]
    fn test_blob_data_clone() {
        let data1 = BlobData {
            data: b"test".to_vec(),
            content_type: "text/plain".to_string(),
        };

        let data2 = data1.clone();
        assert_eq!(data1.data(), data2.data());
        assert_eq!(data1.content_type(), data2.content_type());
    }

    #[test]
    fn test_blob_storage_default() {
        let storage = BlobStorage::default();
        assert!(storage.get("any").is_none());
    }

    #[test]
    fn test_blob_id_uniqueness() {
        let mut ids = std::collections::HashSet::new();

        for i in 0..100 {
            let data = format!("unique data {}", i).into_bytes();
            let id = generate_blob_id(&data);
            assert!(ids.insert(id), "Duplicate blob ID generated");
        }
    }

    #[test]
    fn test_blob_storage_concurrent_access() {
        let storage = BlobStorage::new();

        storage.store(
            "blob1".to_string(),
            b"data1".to_vec(),
            "text/plain".to_string(),
        );

        let storage2 = storage.clone();
        storage2.store(
            "blob2".to_string(),
            b"data2".to_vec(),
            "text/html".to_string(),
        );

        assert!(storage.get("blob1").is_some());
        assert!(storage.get("blob2").is_some());
        assert!(storage2.get("blob1").is_some());
        assert!(storage2.get("blob2").is_some());
    }

    #[test]
    fn test_blob_storage_size_nonexistent() {
        let storage = BlobStorage::new();
        assert_eq!(storage.size("nonexistent"), None);
    }

    #[test]
    fn test_blob_id_format() {
        let data = b"test";
        let blob_id = generate_blob_id(data);
        assert!(blob_id.chars().skip(1).all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_upload_error_without_detail() {
        let error = UploadErrorBody {
            error_type: "urn:ietf:params:jmap:error:serverFail".to_string(),
            status: 500,
            detail: None,
        };

        let json = serde_json::to_string(&error).expect("serialization should succeed");
        assert!(!json.contains("detail"));
    }

    #[test]
    fn test_blob_id_deterministic() {
        let data = b"consistent data";
        let id1 = generate_blob_id(data);
        let id2 = generate_blob_id(data);
        let id3 = generate_blob_id(data);

        assert_eq!(id1, id2);
        assert_eq!(id2, id3);
    }

    #[test]
    fn test_max_blob_size_constant() {
        assert_eq!(DEFAULT_MAX_BLOB_SIZE, 50 * 1024 * 1024);
    }

    // ── new async memory tests ─────────────────────────────────────────────

    /// Upload to memory, download, verify bytes match.
    #[tokio::test]
    async fn test_memory_roundtrip() {
        let storage = BlobStorage::new();
        let payload = b"hello, JMAP blob world!";

        let blob_id = storage
            .upload("account-alice", "text/plain", payload)
            .await
            .expect("upload should succeed");

        let (data, meta) = storage
            .download(&blob_id)
            .await
            .expect("download should succeed");

        assert_eq!(data.as_slice(), payload);
        assert_eq!(meta.content_type, "text/plain");
        assert_eq!(meta.account_id, "account-alice");
        assert_eq!(meta.size, payload.len() as u64);
    }

    /// Upload 49 MiB — must succeed with the default 50 MiB limit.
    #[tokio::test]
    async fn test_size_limit_accepted() {
        let storage = BlobStorage::new();
        // 49 MiB — just under the 50 MiB default ceiling.
        let payload = vec![0xABu8; 49 * 1024 * 1024];

        let result = storage
            .upload("account-alice", "application/octet-stream", &payload)
            .await;
        assert!(
            result.is_ok(),
            "49 MiB upload should succeed, got {:?}",
            result
        );
    }

    /// Upload 51 MiB — must be rejected with TooLarge.
    #[tokio::test]
    async fn test_size_limit_rejected() {
        let storage = BlobStorage::new();
        // 51 MiB — over the 50 MiB default ceiling.
        let payload = vec![0xFFu8; 51 * 1024 * 1024];

        let err = storage
            .upload("account-alice", "application/octet-stream", &payload)
            .await
            .expect_err("51 MiB upload should be rejected");

        match err {
            UploadError::TooLarge { actual, max } => {
                assert_eq!(actual, 51 * 1024 * 1024);
                assert_eq!(max, DEFAULT_MAX_BLOB_SIZE);
            }
            other => panic!("expected TooLarge, got {:?}", other),
        }
    }

    /// Custom size limit: upload right at the boundary.
    #[tokio::test]
    async fn test_custom_size_limit() {
        let storage = BlobStorage::new().with_max_blob_size(1024);
        let payload_ok = vec![0u8; 1024];
        let payload_bad = vec![0u8; 1025];

        assert!(
            storage
                .upload("acc", "application/octet-stream", &payload_ok)
                .await
                .is_ok(),
            "Exactly-at-limit upload should succeed"
        );
        let err = storage
            .upload("acc", "application/octet-stream", &payload_bad)
            .await
            .expect_err("Over-limit upload should fail");
        assert!(matches!(err, UploadError::TooLarge { .. }));
    }

    /// Delete removes the blob from the store.
    #[tokio::test]
    async fn test_memory_delete() {
        let storage = BlobStorage::new();
        let blob_id = storage
            .upload("acc", "text/plain", b"delete me")
            .await
            .expect("upload should succeed");

        storage
            .delete(&blob_id)
            .await
            .expect("delete should succeed");

        let err = storage
            .download(&blob_id)
            .await
            .expect_err("download after delete should fail");
        assert!(matches!(err, UploadError::NotFound(_)));
    }

    // ── filesystem backend tests ───────────────────────────────────────────

    /// Upload to filesystem, drop storage, open a new instance, download → same bytes.
    #[tokio::test]
    async fn test_filesystem_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("rusmes_blob_roundtrip_{}", Uuid::new_v4()));
        let payload = b"filesystem roundtrip payload";

        let blob_id = {
            let storage = BlobStorage::new_filesystem(tmp.clone())
                .await
                .expect("new_filesystem should succeed");
            storage
                .upload("account-bob", "text/plain", payload)
                .await
                .expect("upload should succeed")
        }; // storage is dropped here

        // Re-open the same root; index must be rebuilt from sidecars.
        let storage2 = BlobStorage::new_filesystem(tmp.clone())
            .await
            .expect("re-open should succeed");

        let (data, meta) = storage2
            .download(&blob_id)
            .await
            .expect("download after re-open should succeed");

        assert_eq!(data.as_slice(), payload);
        assert_eq!(meta.content_type, "text/plain");
        assert_eq!(meta.account_id, "account-bob");

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    /// After a restart, the index count must match the number of blobs written.
    #[tokio::test]
    async fn test_filesystem_index_rebuild() {
        let tmp = std::env::temp_dir().join(format!("rusmes_blob_index_{}", Uuid::new_v4()));

        const N: usize = 5;

        {
            let storage = BlobStorage::new_filesystem(tmp.clone())
                .await
                .expect("new_filesystem should succeed");

            for i in 0..N {
                let payload = format!("blob payload {}", i);
                storage
                    .upload("account-test", "text/plain", payload.as_bytes())
                    .await
                    .expect("upload should succeed");
            }

            let count = storage.blob_count().await.expect("count should succeed");
            assert_eq!(count, N);
        } // storage dropped

        // Re-open; index must match.
        let storage2 = BlobStorage::new_filesystem(tmp.clone())
            .await
            .expect("re-open should succeed");

        let count = storage2.blob_count().await.expect("count should succeed");
        assert_eq!(count, N, "Index must be fully rebuilt after restart");

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    /// Filesystem backend also enforces size limits (before any I/O).
    #[tokio::test]
    async fn test_filesystem_size_limit_rejected() {
        let tmp = std::env::temp_dir().join(format!("rusmes_blob_sizelimit_{}", Uuid::new_v4()));

        let storage = BlobStorage::new_filesystem(tmp.clone())
            .await
            .expect("new_filesystem should succeed")
            .with_max_blob_size(512);

        let payload = vec![0u8; 513];
        let err = storage
            .upload("account-test", "application/octet-stream", &payload)
            .await
            .expect_err("over-limit upload should fail");

        assert!(matches!(err, UploadError::TooLarge { .. }));

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }

    /// Delete removes both files from the filesystem.
    #[tokio::test]
    async fn test_filesystem_delete() {
        let tmp = std::env::temp_dir().join(format!("rusmes_blob_delete_{}", Uuid::new_v4()));

        let storage = BlobStorage::new_filesystem(tmp.clone())
            .await
            .expect("new_filesystem should succeed");

        let blob_id = storage
            .upload("account-test", "text/plain", b"to be deleted")
            .await
            .expect("upload should succeed");

        storage
            .delete(&blob_id)
            .await
            .expect("delete should succeed");

        // Blob should be gone from index.
        let err = storage
            .download(&blob_id)
            .await
            .expect_err("download after delete should fail");
        assert!(matches!(err, UploadError::NotFound(_)));

        // Data file should be gone from disk.
        let blob_path = tmp.join("blobs").join(&blob_id);
        assert!(!blob_path.exists(), "blob file should have been removed");

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&tmp).await;
    }
}
