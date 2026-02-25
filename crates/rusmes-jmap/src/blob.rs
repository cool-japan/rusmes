//! Blob upload/download endpoints for JMAP
//!
//! Implements RFC 8620 Section 6.1 and 6.2:
//! - GET /download/:account/:blob/:name - download blobs
//! - POST /upload/:account - upload blobs
//! - Blob size limits and validation

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Maximum blob size (10MB)
const MAX_BLOB_SIZE: usize = 10 * 1024 * 1024;

/// Blob storage (in-memory for now)
#[derive(Clone)]
pub struct BlobStorage {
    blobs: Arc<RwLock<HashMap<String, BlobData>>>,
}

/// Blob data
#[derive(Clone)]
pub struct BlobData {
    data: Vec<u8>,
    content_type: String,
}

impl BlobStorage {
    /// Create new blob storage
    pub fn new() -> Self {
        Self {
            blobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Store a blob
    pub fn store(&self, blob_id: String, data: Vec<u8>, content_type: String) {
        let blob_data = BlobData { data, content_type };
        if let Ok(mut blobs) = self.blobs.write() {
            blobs.insert(blob_id, blob_data);
        }
    }

    /// Retrieve a blob
    pub fn get(&self, blob_id: &str) -> Option<BlobData> {
        self.blobs
            .read()
            .ok()
            .and_then(|blobs| blobs.get(blob_id).cloned())
    }

    /// Get blob size
    pub fn size(&self, blob_id: &str) -> Option<usize> {
        self.blobs
            .read()
            .ok()
            .and_then(|blobs| blobs.get(blob_id).map(|b| b.data.len()))
    }
}

impl Default for BlobStorage {
    fn default() -> Self {
        Self::new()
    }
}

/// Upload response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub account_id: String,
    pub blob_id: String,
    #[serde(rename = "type")]
    pub content_type: String,
    pub size: usize,
}

/// Upload error response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Create blob router
pub fn blob_routes() -> Router<BlobStorage> {
    Router::new()
        .route("/download/:account/:blob/:name", get(download_blob))
        .route("/upload/:account", post(upload_blob))
}

/// Download blob endpoint
async fn download_blob(
    Path((account, blob_id, name)): Path<(String, String, String)>,
    State(storage): State<BlobStorage>,
) -> Response {
    // Validate account (in production, check authentication)
    if account.is_empty() {
        return (StatusCode::BAD_REQUEST, "Invalid account ID".to_string()).into_response();
    }

    // Retrieve blob
    match storage.get(&blob_id) {
        Some(blob_data) => {
            // Return blob with appropriate headers
            match Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, blob_data.content_type)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", name),
                )
                .header(header::CONTENT_LENGTH, blob_data.data.len())
                .body(Body::from(blob_data.data))
            {
                Ok(response) => response,
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to build response: {}", e),
                )
                    .into_response(),
            }
        }
        None => (StatusCode::NOT_FOUND, "Blob not found".to_string()).into_response(),
    }
}

/// Upload blob endpoint
async fn upload_blob(
    Path(account): Path<String>,
    State(storage): State<BlobStorage>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // Validate account
    if account.is_empty() {
        let error = UploadError {
            error_type: "urn:ietf:params:jmap:error:invalidArguments".to_string(),
            status: 400,
            detail: Some("Invalid account ID".to_string()),
        };
        return (StatusCode::BAD_REQUEST, axum::Json(error)).into_response();
    }

    // Check size limit
    if body.len() > MAX_BLOB_SIZE {
        let error = UploadError {
            error_type: "urn:ietf:params:jmap:error:tooLarge".to_string(),
            status: 413,
            detail: Some(format!(
                "Blob size {} exceeds maximum of {}",
                body.len(),
                MAX_BLOB_SIZE
            )),
        };
        return (StatusCode::PAYLOAD_TOO_LARGE, axum::Json(error)).into_response();
    }

    // Get content type
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    // Generate blob ID
    let blob_id = generate_blob_id(&body);

    // Store blob
    let data = body.to_vec();
    let size = data.len();
    storage.store(blob_id.clone(), data, content_type.clone());

    // Return upload response
    let response = UploadResponse {
        account_id: account,
        blob_id,
        content_type,
        size,
    };

    (StatusCode::CREATED, axum::Json(response)).into_response()
}

/// Generate blob ID from data
fn generate_blob_id(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    format!("G{:x}", result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blob_storage_store_and_get() {
        let storage = BlobStorage::new();
        let data = b"test data".to_vec();
        let blob_id = "blob123".to_string();

        storage.store(blob_id.clone(), data.clone(), "text/plain".to_string());

        let retrieved = storage.get(&blob_id).unwrap();
        assert_eq!(retrieved.data, data);
        assert_eq!(retrieved.content_type, "text/plain");
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

        // Same data should produce same ID
        assert_eq!(id1, id2);

        // Different data should produce different ID
        assert_ne!(id1, id3);

        // Should start with 'G'
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
            storage.store(blob_id.clone(), data.clone(), "text/plain".to_string());
        }

        // Verify all blobs exist
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

        let retrieved = storage.get(&blob_id).unwrap();
        assert_eq!(retrieved.data, b"updated");
        assert_eq!(retrieved.content_type, "text/html");
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

        let retrieved = storage.get(&blob_id).unwrap();
        assert_eq!(retrieved.data.len(), 0);
    }

    #[test]
    fn test_blob_storage_large_data() {
        let storage = BlobStorage::new();
        let data = vec![0u8; 1024 * 1024]; // 1MB
        let blob_id = "large".to_string();

        storage.store(
            blob_id.clone(),
            data.clone(),
            "application/octet-stream".to_string(),
        );

        assert_eq!(storage.size(&blob_id), Some(1024 * 1024));
    }

    #[test]
    fn test_upload_response_serialization() {
        let response = UploadResponse {
            account_id: "acc1".to_string(),
            blob_id: "blob123".to_string(),
            content_type: "image/png".to_string(),
            size: 1024,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("blob123"));
        assert!(json.contains("image/png"));
    }

    #[test]
    fn test_upload_error_serialization() {
        let error = UploadError {
            error_type: "urn:ietf:params:jmap:error:tooLarge".to_string(),
            status: 413,
            detail: Some("Too large".to_string()),
        };

        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("tooLarge"));
        assert!(json.contains("413"));
    }

    #[test]
    fn test_max_blob_size_constant() {
        assert_eq!(MAX_BLOB_SIZE, 10 * 1024 * 1024);
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
        assert_eq!(data1.data, data2.data);
        assert_eq!(data1.content_type, data2.content_type);
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

        // Store from one reference
        storage.store(
            "blob1".to_string(),
            b"data1".to_vec(),
            "text/plain".to_string(),
        );

        // Clone and access from another
        let storage2 = storage.clone();
        storage2.store(
            "blob2".to_string(),
            b"data2".to_vec(),
            "text/html".to_string(),
        );

        // Both should see both blobs
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

        // Should be hex characters after 'G'
        assert!(blob_id.chars().skip(1).all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_upload_error_without_detail() {
        let error = UploadError {
            error_type: "urn:ietf:params:jmap:error:serverFail".to_string(),
            status: 500,
            detail: None,
        };

        let json = serde_json::to_string(&error).unwrap();
        assert!(!json.contains("detail"));
    }
}
