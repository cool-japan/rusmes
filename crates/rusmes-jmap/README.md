# rusmes-jmap

JMAP (JSON Meta Application Protocol) server for RusMES, targeting RFC 8620 (JMAP Core) and RFC 8621 (JMAP Mail) compliance.

## Status

Foundation is in place: request/response types, method dispatch skeleton, and axum-based HTTP server. Full method implementations needed.

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

## Modules

| Module | Description |
|--------|-------------|
| `api` | `JmapServer` - axum router and endpoint handlers |
| `types` | `JmapRequest`, `JmapResponse`, `JmapMethod` |
| `methods` | Method dispatch and handler stubs |

## Key Types

```rust
pub struct JmapRequest {
    pub using: Vec<String>,           // Capability URIs
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
cargo test -p rusmes-jmap   # 1 test
```
