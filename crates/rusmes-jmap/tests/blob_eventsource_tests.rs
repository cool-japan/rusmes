//! Integration tests for blob upload/download and EventSource push routes.
//!
//! Exercises the routes mounted by [`rusmes_jmap::JmapServer::routes_with_auth_and_state`]:
//! - `POST /upload/:account_id`           — RFC 8620 §6.2 blob upload
//! - `GET /download/:account_id/:blob_id/:name` — RFC 8620 §6.2 blob download
//! - `GET /eventsource`                   — RFC 8620 §7.3 SSE push channel
//!
//! All routes are behind `require_auth`, so we exercise both the 401 rejection
//! path and the authenticated success path.

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use rusmes_auth::AuthBackend;
use rusmes_jmap::{BlobStorage, EventSourceManager, JmapServer, SharedAuth, UploadResponse};
use rusmes_proto::Username;
use std::sync::Arc;
use tower::ServiceExt;

// ─── Auth fixture ────────────────────────────────────────────────────────────

/// Test-only backend: accepts only `alice` / `hunter2`.
struct AliceBackend;

#[async_trait]
impl AuthBackend for AliceBackend {
    async fn authenticate(&self, username: &Username, password: &str) -> anyhow::Result<bool> {
        Ok(username.as_str() == "alice" && password == "hunter2")
    }
    async fn verify_identity(&self, username: &Username) -> anyhow::Result<bool> {
        Ok(username.as_str() == "alice")
    }
    async fn list_users(&self) -> anyhow::Result<Vec<Username>> {
        Ok(vec![Username::new("alice".to_string())?])
    }
    async fn create_user(&self, _u: &Username, _p: &str) -> anyhow::Result<()> {
        anyhow::bail!("read-only test backend")
    }
    async fn delete_user(&self, _u: &Username) -> anyhow::Result<()> {
        anyhow::bail!("read-only test backend")
    }
    async fn change_password(&self, _u: &Username, _p: &str) -> anyhow::Result<()> {
        anyhow::bail!("read-only test backend")
    }
}

/// `base64("alice:hunter2") = YWxpY2U6aHVudGVyMg==`
const ALICE_BASIC: &str = "Basic YWxpY2U6aHVudGVyMg==";

fn make_router() -> axum::Router {
    let auth: SharedAuth = Arc::new(AliceBackend);
    let blobs = BlobStorage::new();
    let events = EventSourceManager::new();
    JmapServer::routes_with_auth_and_state(auth, blobs, events)
}

// ─── Blob upload tests ────────────────────────────────────────────────────────

/// Blob upload without an Authorization header must return 401.
#[tokio::test]
async fn blob_upload_requires_auth() {
    let app = make_router();
    let req = Request::builder()
        .method("POST")
        .uri("/upload/account-alice")
        .header("content-type", "text/plain")
        .body(Body::from("hello world"))
        .expect("build request");

    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Blob upload with valid credentials returns 201 Created and a JSON body
/// containing a non-empty `blobId`.
#[tokio::test]
async fn blob_upload_with_auth_returns_201_with_blob_id() {
    let app = make_router();
    let payload = b"hello, JMAP blob!";
    let req = Request::builder()
        .method("POST")
        .uri("/upload/account-alice")
        .header("authorization", ALICE_BASIC)
        .header("content-type", "text/plain")
        .body(Body::from(payload.as_ref()))
        .expect("build request");

    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body = to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    let upload_resp: UploadResponse = serde_json::from_slice(&body).expect("parse upload response");

    assert!(!upload_resp.blob_id.is_empty(), "blobId must be non-empty");
    assert_eq!(upload_resp.account_id, "account-alice");
    assert_eq!(upload_resp.size, payload.len());
}

/// Upload a blob then download it — the downloaded bytes must be bit-identical
/// to the uploaded payload.
#[tokio::test]
async fn blob_download_roundtrip() {
    // Re-use the same app instance so upload + download share the same BlobStorage.
    let auth: SharedAuth = Arc::new(AliceBackend);
    let blobs = BlobStorage::new();
    let events = EventSourceManager::new();
    let app = JmapServer::routes_with_auth_and_state(auth, blobs.clone(), events);

    let payload = b"roundtrip test payload, byte-identical check";

    // Upload
    let upload_req = Request::builder()
        .method("POST")
        .uri("/upload/account-alice")
        .header("authorization", ALICE_BASIC)
        .header("content-type", "application/octet-stream")
        .body(Body::from(payload.as_ref()))
        .expect("build upload request");

    let upload_resp = app
        .clone()
        .oneshot(upload_req)
        .await
        .expect("upload dispatch");
    assert_eq!(upload_resp.status(), StatusCode::CREATED);

    let upload_body = to_bytes(upload_resp.into_body(), 1 << 20)
        .await
        .expect("read upload body");
    let upload_json: UploadResponse =
        serde_json::from_slice(&upload_body).expect("parse upload response");
    let blob_id = upload_json.blob_id;

    // Download
    let download_uri = format!("/download/account-alice/{}/file.bin", blob_id);
    let download_req = Request::builder()
        .method("GET")
        .uri(&download_uri)
        .header("authorization", ALICE_BASIC)
        .body(Body::empty())
        .expect("build download request");

    let download_resp = app.oneshot(download_req).await.expect("download dispatch");
    assert_eq!(download_resp.status(), StatusCode::OK);

    let downloaded = to_bytes(download_resp.into_body(), 1 << 20)
        .await
        .expect("read download body");
    assert_eq!(
        downloaded.as_ref(),
        payload,
        "downloaded bytes must be identical to uploaded payload"
    );
}

// ─── EventSource tests ───────────────────────────────────────────────────────

/// EventSource SSE endpoint without an Authorization header must return 401.
#[tokio::test]
async fn eventsource_requires_auth() {
    let app = make_router();
    let req = Request::builder()
        .method("GET")
        .uri("/eventsource")
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// EventSource SSE endpoint with valid credentials returns 200 and the
/// `Content-Type: text/event-stream` header required by RFC 8620 §7.3 / W3C SSE.
#[tokio::test]
async fn eventsource_returns_text_event_stream_content_type() {
    let app = make_router();
    // Use `closeafter=0` so the stream terminates immediately and we don't
    // block waiting for the body.
    let req = Request::builder()
        .method("GET")
        .uri("/eventsource?closeafter=0")
        .header("authorization", ALICE_BASIC)
        .body(Body::empty())
        .expect("build request");

    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::OK);

    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("text/event-stream"),
        "Content-Type must be text/event-stream, got: {}",
        content_type
    );
}
