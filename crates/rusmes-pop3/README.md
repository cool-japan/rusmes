# rusmes-pop3

POP3 protocol implementation for RusMES. Provides legacy POP3 (RFC 1939) support for clients that do not support IMAP or JMAP.

## Status

Complete. Full RFC 1939 implementation with APOP, STARTTLS (RFC 2595), and CAPA
(RFC 2449).

## Architecture

```
Pop3Server
  |-- accepts TCP connections (110 / 995)
  |-- spawns Pop3Session per connection
  |     |-- AUTHORIZATION state (USER/PASS or APOP)
  |     |-- TRANSACTION state (STAT, LIST, RETR, DELE, etc.)
  |     '-- UPDATE state (commit deletions on QUIT)
  '-- TLS support via rustls
```

## Dependencies
- `rusmes-proto` - mail types
- `rusmes-storage` - message access
- `rusmes-auth` - authentication
- `nom` - command parsing
- `rustls` / `tokio-rustls` - TLS
- `tokio` - async networking
