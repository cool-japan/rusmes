# rusmes-smtp

SMTP protocol implementation for RusMES, targeting RFC 5321 compliance with modern extensions.

## Architecture

```
SmtpServer
  |-- accepts TCP connections
  |-- spawns SmtpSession per connection
  |     |-- SmtpState machine
  |     |-- Command parser (nom-based)
  |     |-- Response builder
  |     |-- Authentication (via rusmes-auth)
  |     '-- Hands off Mail to MailProcessorRouter
  '-- TLS support via rustls
```

## Modules

| Module | Description |
|--------|-------------|
| `server` | `SmtpServer` - TCP listener, connection dispatch |
| `session` | `SmtpSession` - per-connection state machine and command handling |
| `session/data` | DATA tempfile spill — payloads above threshold (default 1 MiB) spill to `tempfile::NamedTempFile`, returning `MessageBody::Large`; configurable via `SmtpConfig::data_tempfile_threshold` |
| `outbound_pool` | Outbound connection pool — reusable TLS+TCP sessions to relay destinations |
| `command` | `SmtpCommand` enum - all SMTP commands with parsed parameters |
| `parser` | `parse_command()` - nom-based SMTP command parser |
| `response` | `SmtpResponse` - SMTP reply codes and messages |

## SMTP Commands

| Command | Status | Description |
|---------|--------|-------------|
| EHLO/HELO | Implemented | Client greeting with capability negotiation |
| MAIL FROM | Implemented | Set envelope sender |
| RCPT TO | Implemented | Add envelope recipient |
| DATA | Implemented | Message body transfer |
| RSET | Implemented | Reset transaction |
| QUIT | Implemented | Close connection |
| STARTTLS | Implemented | TLS upgrade |
| AUTH | Implemented | PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256 mechanisms |
| mTLS | Implemented | Mutual TLS — client certificate verification on TLS listeners |
| NOOP | Implemented | No operation |
| VRFY | Stub | Verify address |

## Session States

```
Initial -> Connected (after greeting)
  -> Authenticated (after AUTH or if auth not required)
    -> MailTransaction (after MAIL FROM)
      -> Data (after DATA, collecting message body)
    -> Quit
```

## Configuration

```rust
pub struct SmtpConfig {
    pub hostname: String,         // Server hostname for greeting
    pub max_message_size: usize,  // Max message size in bytes
    pub require_auth: bool,       // Require AUTH before relay
    pub enable_starttls: bool,    // Advertise STARTTLS
}
```

## Dependencies
- `rusmes-proto` - mail types
- `rusmes-core` - mail processing router
- `rusmes-auth` - authentication backend
- `nom` - command parsing
- `rustls` / `tokio-rustls` - TLS
- `tokio` - async networking

## Tests

```bash
cargo test -p rusmes-smtp   # 165 tests
```

Tests cover: command parsing, response formatting, session state transitions, EHLO capability negotiation, and mail address extraction.
