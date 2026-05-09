//! Integration tests for the JMAP authentication middleware.
//!
//! These tests exercise the full Axum stack via `JmapServer::routes_with_auth`
//! and an in-memory [`AuthBackend`] fixture, validating that:
//!
//! - Missing or malformed `Authorization` headers are rejected with 401.
//! - Bearer tokens are currently unsupported (rejected with 401) — this guards
//!   against the legacy "DEVELOPMENT ONLY" path that fabricated principals.
//! - Successful Basic auth attaches a [`Principal`] downstream so the JMAP
//!   session endpoint reflects the authenticated user.

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use rusmes_auth::AuthBackend;
use rusmes_jmap::{Account, JmapServer, Session, SharedAuth};
use rusmes_proto::Username;
use std::sync::Arc;
use tower::ServiceExt;

/// Test backend: accepts only `alice` / `hunter2`.
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

fn router_with_alice() -> axum::Router {
    let auth: SharedAuth = Arc::new(AliceBackend);
    JmapServer::routes_with_auth(auth)
}

/// `base64("alice:hunter2") = YWxpY2U6aHVudGVyMg==`.
const ALICE_BASIC: &str = "Basic YWxpY2U6aHVudGVyMg==";
/// `base64("alice:wrong") = YWxpY2U6d3Jvbmc=`.
const ALICE_BASIC_BAD: &str = "Basic YWxpY2U6d3Jvbmc=";

#[tokio::test]
async fn no_authorization_header_returns_401() {
    let app = router_with_alice();
    let req = Request::builder()
        .method("GET")
        .uri("/.well-known/jmap")
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let www = resp
        .headers()
        .get(axum::http::header::WWW_AUTHENTICATE)
        .expect("WWW-Authenticate header present");
    assert!(www.to_str().unwrap_or_default().starts_with("Basic"));
}

#[tokio::test]
async fn malformed_authorization_header_returns_401() {
    let app = router_with_alice();
    let req = Request::builder()
        .method("GET")
        .uri("/.well-known/jmap")
        .header(axum::http::header::AUTHORIZATION, "totally not valid")
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bearer_token_currently_rejected() {
    // The middleware does not yet integrate with a token introspection
    // backend; until then, every Bearer attempt must 401 (NOT silently
    // accept any token, which is what the previous DEVELOPMENT ONLY code
    // did).
    let app = router_with_alice();
    let req = Request::builder()
        .method("GET")
        .uri("/.well-known/jmap")
        .header(
            axum::http::header::AUTHORIZATION,
            "Bearer some-jwt-looking.token.value",
        )
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn basic_auth_wrong_password_returns_401() {
    let app = router_with_alice();
    let req = Request::builder()
        .method("GET")
        .uri("/.well-known/jmap")
        .header(axum::http::header::AUTHORIZATION, ALICE_BASIC_BAD)
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn basic_auth_unknown_user_returns_401() {
    // base64("bob:hunter2") = "Ym9iOmh1bnRlcjI="
    let app = router_with_alice();
    let req = Request::builder()
        .method("GET")
        .uri("/.well-known/jmap")
        .header(axum::http::header::AUTHORIZATION, "Basic Ym9iOmh1bnRlcjI=")
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn basic_auth_alice_returns_session_with_alice_principal() {
    let app = router_with_alice();
    let req = Request::builder()
        .method("GET")
        .uri("/.well-known/jmap")
        .header(axum::http::header::AUTHORIZATION, ALICE_BASIC)
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    let session: Session = serde_json::from_slice(&body).expect("parse session");
    assert_eq!(session.username, "alice");
    let account_id = session
        .accounts
        .keys()
        .next()
        .expect("at least one account")
        .clone();
    assert_eq!(account_id, "account-alice");
    let acct: &Account = session
        .accounts
        .get(&account_id)
        .expect("account is present");
    assert_eq!(acct.name, "alice");
    assert!(acct.is_personal);
}

#[tokio::test]
async fn auth_less_routes_reject_everything_with_401() {
    // The bare `routes()` constructor is the safe default for callers that
    // forget to wire an AuthBackend — every request must 401, even without
    // an Authorization header.
    let app = JmapServer::routes();
    let req = Request::builder()
        .method("GET")
        .uri("/.well-known/jmap")
        .header(axum::http::header::AUTHORIZATION, ALICE_BASIC)
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("dispatch");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
