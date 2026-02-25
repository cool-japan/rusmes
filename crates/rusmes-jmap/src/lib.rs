//! JMAP protocol implementation for RusMES
//!
//! This crate implements the JSON Meta Application Protocol (JMAP) for the RusMES mail
//! server, providing an HTTP/JSON API for email access as defined in:
//!
//! - **RFC 8620** — The JSON Meta Application Protocol (core protocol)
//! - **RFC 8621** — Using JMAP for Email (Email, Mailbox, Thread, EmailSubmission,
//!   VacationResponse, SearchSnippet, Identity)
//!
//! # Module Overview
//!
//! | Module | Contents |
//! |--------|----------|
//! [`api`] | Axum-based HTTP server (`JmapServer`), request dispatch, auth middleware |
//! [`blob`] | Blob upload (`POST /upload/:account`) and download (`GET /download/…`) endpoints |
//! [`eventsource`] | Server-Sent Events push channel (`GET /eventsource`) for state-change notifications |
//! [`session`] | JMAP Session resource (`GET /.well-known/jmap`), capability advertisement |
//! [`types`] | Shared JSON types: `Email`, `JmapRequest`, `JmapResponse`, `JmapError`, etc. |
//! [`methods`] | Method handlers for all JMAP objects (see sub-modules below) |
//!
//! ## Method Handlers
//!
//! | Sub-module | RFC | Methods |
//! |-----------|-----|---------|
//! `methods::mailbox` | RFC 8621 §2 | `Mailbox/get`, `Mailbox/set`, `Mailbox/query`, `Mailbox/changes`, `Mailbox/queryChanges` |
//! `methods::email` | RFC 8621 §4 | `Email/get`, `Email/set`, `Email/query` |
//! `methods::email_advanced` | RFC 8621 §4 | `Email/changes`, `Email/queryChanges`, `Email/copy`, `Email/import`, `Email/parse` |
//! `methods::thread` | RFC 8621 §3 | `Thread/get`, `Thread/changes` |
//! `methods::submission` | RFC 8621 §7 | `EmailSubmission/get`, `EmailSubmission/set`, `EmailSubmission/query`, `EmailSubmission/changes` |
//! `methods::identity` | RFC 8621 §5 | `Identity/get`, `Identity/set`, `Identity/changes` |
//! `methods::vacation` | RFC 8621 §8 | `VacationResponse/get`, `VacationResponse/set` |
//! `methods::search_snippet` | RFC 8621 §9 | `SearchSnippet/get` |
//!
//! # Example — Starting the JMAP server
//!
//! ```rust,no_run
//! use rusmes_jmap::JmapServer;
//! use axum::Router;
//!
//! // Obtain the JMAP routes and nest them inside an Axum application.
//! let app: Router = JmapServer::routes();
//! // e.g. axum::serve(listener, app).await.unwrap();
//! ```
//!
//! # EventSource Push
//!
//! The [`eventsource::EventSourceManager`] can be used to broadcast state-change
//! notifications to JMAP clients that maintain a long-lived SSE connection:
//!
//! ```rust,no_run
//! use rusmes_jmap::EventSourceManager;
//!
//! let manager = EventSourceManager::new();
//! // Notify all connected clients that the Email state has changed:
//! manager.notify_change("Email".to_string(), "new-state-token".to_string());
//! ```
//!
//! # Blob Handling
//!
//! [`BlobStorage`] provides a content-addressed in-memory blob store keyed by
//! SHA-256 hash.  Uploaded blobs are referenced by `blobId` strings throughout
//! JMAP Email import/parse operations.

pub mod api;
pub mod blob;
pub mod eventsource;
pub mod methods;
pub mod session;
pub mod types;

pub use api::JmapServer;
pub use blob::{BlobStorage, UploadError, UploadResponse};
pub use eventsource::{EventSourceManager, PushSubscription, StateChange};
pub use session::{Account, AccountCapability, Capability, Session};
pub use types::{
    Email, EmailAddress, EmailGetRequest, EmailGetResponse, EmailQueryRequest, EmailQueryResponse,
    EmailSetRequest, EmailSetResponse, JmapError, JmapErrorType, JmapMethod, JmapRequest,
    JmapResponse,
};
