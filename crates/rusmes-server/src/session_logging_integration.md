# Session Logging Integration Guide

This document provides examples of how to integrate the session logging module with the SMTP, IMAP, POP3, and JMAP servers.

## Overview

The session logging module provides:
- Unique session IDs (UUID v4) per connection
- Session context with client IP, protocol, and user information
- Structured logging with tracing-subscriber
- Helper macros for convenient logging
- Session ID headers for HTTP protocols (JMAP)

## SMTP Integration Example

```rust
use rusmes_server::session_logging::{SessionContext, SessionLogger};
use std::net::SocketAddr;

async fn handle_smtp_connection(stream: TcpStream, remote_addr: SocketAddr) -> anyhow::Result<()> {
    // Create session context
    let session_ctx = SessionContext::new(remote_addr.ip(), "SMTP");
    let mut logger = SessionLogger::new(session_ctx);

    // Enter session span (logs will include session context)
    let _guard = logger.enter();

    tracing::info!(
        "New SMTP connection established",
        session_id = %logger.context().session_id(),
    );

    // ... handle HELO/EHLO ...

    // After authentication, update the session with username
    let username = "alice@example.com";
    logger.set_username(username);

    // Re-enter the span with updated username
    let _guard = logger.enter();
    tracing::info!(
        "User authenticated",
        username = %username,
    );

    // ... handle mail transaction ...

    tracing::info!("Connection closing");

    Ok(())
}
```

## IMAP Integration Example

```rust
use rusmes_server::session_logging::{SessionContext, SessionLogger};
use std::net::SocketAddr;

async fn handle_imap_connection(stream: TcpStream, remote_addr: SocketAddr) -> anyhow::Result<()> {
    // Create session context
    let session_ctx = SessionContext::new(remote_addr.ip(), "IMAP");
    let mut logger = SessionLogger::new(session_ctx);

    let _guard = logger.enter();

    tracing::info!("IMAP connection established");

    // ... handle commands ...

    // After LOGIN command succeeds
    let username = "bob@example.com";
    logger.set_username(username);

    let _guard = logger.enter();
    tracing::info!(
        "User logged in",
        mailbox_count = 5,
    );

    // ... handle mailbox operations ...

    Ok(())
}
```

## POP3 Integration Example

```rust
use rusmes_server::session_logging::{SessionContext, SessionLogger};
use std::net::SocketAddr;

async fn handle_pop3_connection(stream: TcpStream, remote_addr: SocketAddr) -> anyhow::Result<()> {
    let session_ctx = SessionContext::new(remote_addr.ip(), "POP3");
    let mut logger = SessionLogger::new(session_ctx);

    let _guard = logger.enter();

    tracing::info!("POP3 connection established");

    // After USER/PASS authentication
    let username = "charlie@example.com";
    logger.set_username(username);

    let _guard = logger.enter();
    tracing::info!(
        "User authenticated",
        message_count = 10,
    );

    Ok(())
}
```

## JMAP Integration Example

```rust
use rusmes_server::session_logging::{SessionContext, SessionLogger, format_session_header};
use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};
use std::net::IpAddr;

/// JMAP middleware to add session logging
pub async fn session_logging_middleware(
    req: Request,
    next: Next,
) -> Response {
    // Extract client IP from request
    let client_ip = req
        .extensions()
        .get::<std::net::SocketAddr>()
        .map(|addr| addr.ip())
        .unwrap_or_else(|| IpAddr::from([127, 0, 0, 1]));

    // Create session context
    let session_ctx = SessionContext::new(client_ip, "JMAP");
    let logger = SessionLogger::new(session_ctx.clone());

    let _guard = logger.enter();

    tracing::info!(
        "JMAP request received",
        method = %req.method(),
        uri = %req.uri(),
    );

    // Process request
    let mut response = next.run(req).await;

    // Add session ID to response headers
    let session_header = format_session_header(&session_ctx);
    response.headers_mut().insert(
        "X-Session-Id",
        session_header.parse().unwrap(),
    );

    response
}
```

## Using Helper Macros

The module provides convenient macros for session-aware logging:

```rust
use rusmes_server::session_logging::{SessionContext, SessionLogger};
use rusmes_server::{session_info, session_debug, session_warn, session_error};

let session_ctx = SessionContext::new(client_ip, "SMTP");
let logger = SessionLogger::new(session_ctx);

// These macros automatically enter the session span
session_info!(logger, "Processing command", command = "RCPT TO");
session_debug!(logger, "State transition", from = "MAIL", to = "RCPT");
session_warn!(logger, "Rate limit approaching", remaining = 5);
session_error!(logger, "Command failed", error = "invalid recipient");
```

## Structured Logging Output

With the session logging module, all logs will include structured fields:

```json
{
  "timestamp": "2026-02-13T10:30:45.123Z",
  "level": "INFO",
  "message": "User authenticated",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "client_ip": "192.168.1.100",
  "protocol": "SMTP",
  "username": "alice@example.com"
}
```

## JSON Output

For external monitoring systems, you can get JSON representation of session context:

```rust
use rusmes_server::session_logging::{SessionContext, format_session_json};

let session_ctx = SessionContext::new(client_ip, "IMAP");
let json = format_session_json(&session_ctx);
// Send to external monitoring/logging system
```

## Benefits

1. **Easy Debugging**: Filter logs by session ID to trace entire connection lifecycle
2. **Performance Analysis**: Track which IPs generate most connections
3. **Security Monitoring**: Identify suspicious patterns by IP/user
4. **Audit Trail**: Complete record of authenticated sessions
5. **Structured Data**: Easy parsing for log aggregation systems (ELK, Splunk, etc.)
