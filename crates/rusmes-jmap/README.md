# rusmes-jmap

JMAP (JSON Meta Application Protocol) server for RusMES, targeting RFC 8620 (JMAP Core) and RFC 8621 (JMAP Mail) compliance.

## Status

Alpha. All core JMAP methods are implemented and tested. Bearer token authentication is wired to the real `AuthBackend`. Email threading (RFC 5256), email import, and email parse are fully operational.

## Architecture

JMAP uses HTTP/JSON instead of a text-based protocol:

```
JmapServer (axum)
  |-- GET  /.well-known/jmap     -> Session resource
  |-- POST /jmap                 -> API endpoint (method calls)
  |-- GET  /download/:acct/:blob -> Blob download
  |-- POST /upload/:acct         -> Blob upload
  '-- GET  /eventsource          -> Push notifications (SSE)
```

## Implemented JMAP Methods

### Core (RFC 8620)
- Session endpoint (`/.well-known/jmap`) with capabilities
- Request validation (`using`, `methodCalls` structure)
- Error responses (`unknownMethod`, `invalidArguments`, `unknownCapability`, `limit`, etc.)
- Account discovery
- Account permission enforcement (per RFC 8620 §3.3 `forbidden` on account mismatch)

### Email Methods (RFC 8621)
- `Email/get`, `Email/set` (create / update / destroy), `Email/query`
- `Email/changes`, `Email/copy`
- `Email/import` — import raw RFC 5322 messages into a mailbox
- `Email/parse` — parse a blob as an RFC 5322 message without storing it
- `EmailSubmission/set`, `EmailSubmission/get`, `EmailSubmission/query`, `EmailSubmission/changes`

### Mailbox Methods
- `Mailbox/get`, `Mailbox/set`, `Mailbox/query`, `Mailbox/changes`

### Other Methods
- `Thread/get`, `Thread/changes`
- `SearchSnippet/get`
- `Identity/get`, `Identity/set` (create / update / destroy)
- `VacationResponse/get`, `VacationResponse/set`

### Email Conversion Correctness
- `EmailConversionContext<'_>`: content-addressed blob IDs (`compute_blob_id` via SHA-256), RFC 8621 keywords (`$seen`, `$flagged`, `$answered`, `$draft`, `$deleted`), real `received_at`, `mailbox_ids`, `thread_id` — no placeholder values in `Email/get` responses

### Threading
- RFC 5256 thread ID assignment via References-chain algorithm

### Authentication
- Bearer token authentication — wired to real `AuthBackend` (token introspection)
- HTTP Basic authentication — delegated to `AuthBackend::authenticate`
- `Principal { account_id, scopes }` attached to request extensions for downstream enforcement

### Blob & Push
- Blob download endpoint (`/download/:account/:blob/:name`)
- Blob upload endpoint (`/upload/:account`)
- EventSource (Server-Sent Events) with broadcast channel

## Modules

| Module | Description |
|--------|-------------|
| `api` | `JmapServer` — axum router and endpoint handlers |
| `auth` | Bearer / Basic credential extraction and `AuthBackend` delegation |
| `types` | `JmapRequest`, `JmapResponse`, `JmapMethod`, `Principal` |
| `methods` | Method dispatch and per-method handlers |

## Key Types

```rust
pub struct JmapRequest {
    pub using: Vec<String>,            // Capability URIs
    pub method_calls: Vec<JmapMethod>, // Batched method invocations
}

pub struct JmapResponse {
    pub method_responses: Vec<JmapMethod>,
    pub session_state: String,
}

pub enum JmapMethod {
    EmailGet { ... },
    EmailSet { ... },
    EmailQuery { ... },
    MailboxGet { ... },
    // ...
}

pub struct Principal {
    pub account_id: String,
    pub scopes: Vec<String>,
}
```

## Dependencies
- `rusmes-proto` - mail types
- `rusmes-storage` - mailbox and message storage
- `rusmes-auth` - authentication
- `axum` / `hyper` - HTTP server
- `serde` / `serde_json` - JSON serialization
- `tokio` - async runtime

## Tests

```bash
cargo nextest run -p rusmes-jmap --all-features   # 135 tests
```
