# rusmes-imap

IMAP protocol implementation for RusMES, targeting RFC 9051 (IMAP4rev2) compliance.

## Status

Foundation is in place: command types, parser skeleton, session state machine, response builder, and server skeleton. The full IMAP command set needs implementation.

## Architecture

```
ImapServer
  |-- accepts TCP connections (143 / 993)
  |-- spawns ImapSession per connection
  |     |-- ImapState machine
  |     |-- Command parser (nom-based)
  |     |-- Response builder (tagged/untagged)
  |     |-- Authentication (via rusmes-auth)
  |     '-- Mailbox access (via rusmes-storage)
  '-- TLS support via rustls
```

## Modules

| Module | Description |
|--------|-------------|
| `server` | `ImapServer` - TCP listener and connection dispatch |
| `session` | `ImapSession` - per-connection state machine |
| `command` | `ImapCommand` enum - IMAP command types |
| `parser` | IMAP command parser (nom-based) |
| `response` | `ImapResponse` builder for tagged and untagged responses |

## Session States (RFC 9051)

```
Not Authenticated
  |-- LOGIN / AUTHENTICATE -> Authenticated
  |-- STARTTLS -> Not Authenticated (upgraded)
  '-- LOGOUT -> Logout

Authenticated
  |-- SELECT / EXAMINE -> Selected
  |-- CREATE / DELETE / RENAME / LIST / SUBSCRIBE
  '-- LOGOUT -> Logout

Selected
  |-- FETCH / STORE / SEARCH / EXPUNGE / COPY / MOVE
  |-- CLOSE -> Authenticated
  |-- SELECT -> Selected (different mailbox)
  '-- IDLE (push notifications)
```

## Dependencies
- `rusmes-proto` - mail types
- `rusmes-storage` - mailbox and message storage
- `rusmes-auth` - authentication
- `nom` - command parsing
- `rustls` / `tokio-rustls` - TLS
- `tokio` - async networking

## Tests

```bash
cargo test -p rusmes-imap   # foundation tests
```
