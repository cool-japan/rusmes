# rusmes-imap

IMAP protocol implementation for RusMES, targeting RFC 9051 (IMAP4rev2) compliance.

## Status

Fully implemented. The complete IMAP command set (RFC 9051) is in production, including UID variants,
nine protocol extensions (including COMPRESS=DEFLATE), a 1,222-line nom-based parser, and 187 passing tests.

## Architecture

```
ImapServer
  |-- accepts TCP connections (143 / 993)
  |-- spawns ImapSession per connection
  |     |-- ImapState machine
  |     |-- Command parser (nom-based, LITERAL+ aware)
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
| `parser` | IMAP command parser (nom-based, 1,222 lines, LITERAL+ aware) |
| `response` | `ImapResponse` builder for tagged and untagged responses |
| `authenticate` | Multi-step SASL authentication handler |
| `condstore` | CONDSTORE extension (RFC 7162, 470 lines) |
| `qresync` | QRESYNC extension (RFC 7162, 1,063 lines) |
| `special_use` | SPECIAL-USE extension (RFC 6154) |
| `mailbox_registry` | Per-mailbox broadcast channel registry for push notifications |

## Session States (RFC 9051)

```
Not Authenticated
  |-- LOGIN / AUTHENTICATE -> Authenticated
  |-- STARTTLS -> Not Authenticated (upgraded)
  '-- LOGOUT -> Logout

Authenticated
  |-- SELECT / EXAMINE -> Selected
  |-- CREATE / DELETE / RENAME / LIST / LSUB / SUBSCRIBE / UNSUBSCRIBE
  |-- NAMESPACE / CAPABILITY / NOOP
  '-- LOGOUT -> Logout

Selected
  |-- FETCH / STORE / SEARCH / EXPUNGE / COPY / MOVE
  |-- UID FETCH / UID STORE / UID SEARCH / UID COPY / UID MOVE / UID EXPUNGE
  |-- CLOSE -> Authenticated
  |-- SELECT -> Selected (different mailbox)
  '-- IDLE (push notifications via broadcast channel)
```

## Implemented Commands

| Command | RFC | Notes |
|---------|-----|-------|
| `LOGIN` | RFC 9051 | Plaintext login |
| `AUTHENTICATE` | RFC 9051 | Multi-step SASL (PLAIN, LOGIN) |
| `SELECT` | RFC 9051 | Opens mailbox read-write |
| `EXAMINE` | RFC 9051 | Opens mailbox read-only |
| `FETCH` | RFC 9051 | All data items: BODY, ENVELOPE, FLAGS, etc. |
| `STORE` | RFC 9051 | +FLAGS, -FLAGS, FLAGS |
| `SEARCH` | RFC 9051 | Full criteria support |
| `APPEND` | RFC 9051 | RFC 9051 compliant, APPENDUID response |
| `LIST` | RFC 9051 | Pattern matching |
| `LSUB` | RFC 9051 | Subscribed mailboxes |
| `SUBSCRIBE` | RFC 9051 | Subscribe to mailbox |
| `UNSUBSCRIBE` | RFC 9051 | Unsubscribe from mailbox |
| `CREATE` | RFC 9051 | Create mailbox |
| `DELETE` | RFC 9051 | Delete mailbox |
| `RENAME` | RFC 9051 | Rename mailbox |
| `COPY` | RFC 9051 | Copy messages |
| `MOVE` | RFC 6851 | Atomic message move |
| `EXPUNGE` | RFC 9051 | Expunge deleted messages |
| `CLOSE` | RFC 9051 | Close selected mailbox |
| `IDLE` | RFC 2177 | Push notifications |
| `NAMESPACE` | RFC 2342 | Namespace advertisement |
| `CAPABILITY` | RFC 9051 | Server capability list |
| `NOOP` | RFC 9051 | No-op / poll for updates |
| `LOGOUT` | RFC 9051 | End session |
| `UID FETCH` | RFC 9051 | UID-based FETCH |
| `UID STORE` | RFC 9051 | UID-based STORE |
| `UID SEARCH` | RFC 9051 | UID-based SEARCH |
| `UID COPY` | RFC 9051 | UID-based COPY |
| `UID MOVE` | RFC 6851 | UID-based MOVE |
| `UID EXPUNGE` | RFC 4315 | UID-based EXPUNGE |

## Extensions

| Extension | RFC | Notes |
|-----------|-----|-------|
| IDLE | RFC 2177 | Push notifications via tokio broadcast channel |
| NAMESPACE | RFC 2342 | Namespace advertisement |
| UIDPLUS | RFC 4315 | APPENDUID, COPYUID responses |
| MOVE | RFC 6851 | Atomic message move |
| CONDSTORE | RFC 7162 | Conditional STORE / MODSEQ tracking (470 lines) |
| QRESYNC | RFC 7162 | Quick mailbox resynchronization (1,063 lines) |
| LITERAL+ | RFC 7888 | Non-synchronizing literals |
| SPECIAL-USE | RFC 6154 | Mailbox role flags (\Inbox, \Sent, \Drafts, etc.) |
| COMPRESS=DEFLATE | RFC 4978 | Per-session stream compression via `RawDeflateWriter`/`RawInflateReader` (oxiarc-deflate 0.2.7) |

## Dependencies
- `rusmes-proto` - mail types
- `rusmes-storage` - mailbox and message storage
- `rusmes-auth` - authentication
- `nom` - command parsing
- `rustls` / `tokio-rustls` - TLS
- `tokio` - async networking
- `dashmap` - per-mailbox broadcast registry

## Tests

```bash
cargo nextest run -p rusmes-imap --all-features
```

187 tests run: 187 passed, 0 skipped.

Coverage: authenticate, CONDSTORE, QRESYNC, SPECIAL-USE, COMPRESS=DEFLATE, parser (LITERAL+ awareness,
APPEND, sequence sets), and mailbox broadcast registry.
