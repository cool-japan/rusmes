# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-05-09

### Added
- JMAP: `EmailConversionContext<'_>` struct eliminates hardcoded placeholder values in `convert_mail_to_email`; adds `compute_blob_id` (SHA-256 content-addressed per RFC 8620 §6.2), `jmap_keywords_from_flags` (RFC 8621 §4.1.1: `$seen`, `$flagged`, `$answered`, `$draft`, `$deleted`)
- JMAP: `make_placeholder_email` (Email/import) now extracts sender/from/to/subject/sent_at from parsed Mail headers
- IMAP: COMPRESS=DEFLATE (RFC 4978) activated using `oxiarc-deflate 0.2.7` `RawDeflateWriter`/`RawInflateReader`; LZ77 sliding window preserved across sync-flush frames
- Storage: AmateRS initial-connect endpoint cycling — sequential failover through `cluster_endpoints` Vec; server bootstraps successfully as long as any endpoint is reachable
- Server: Privilege drop (`PrivilegeDrop`) with `chroot` + `setgid` + `setuid` in correct order; `[server] run_as_user`, `run_as_group`, `chroot` config fields; Linux-only (macOS emits `tracing::warn!`); `nix 0.31.2` for syscalls

### Fixed
- Sieve mailet: register `Sieve` (and the `SieveMailet` alias) in the
  mailet factory so configurations using `mailet: Sieve` are recognised
  instead of failing with `Unknown mailet: Sieve` (#1).

## [0.1.1] - 2026-03-25

### Fixed
- Updated rand API usage: `rand::Rng` → `rand::RngExt` for rand 0.10 compatibility (rusmes-loadtest)
- Fixed clippy `sort_by` → `sort_by_key` with `Reverse` in rusmes-jmap
- Fixed deprecated `criterion::black_box` → `std::hint::black_box` in rusmes-benches
- Fixed clippy `collapsible_match` in rusmes-server
- Fixed clippy `manual_checked_ops` in rusmes-loadtest
- Upgraded oxiarc-deflate, oxiarc-zstd, oxiarc-archive from 0.2.5 to 0.2.6

## [0.1.0] - 2026-02-25

### Added

#### Core Foundation
- Workspace structure with 15 crates covering the full mail server stack
- Core protocol types: `Mail`, `MailAddress`, `Domain`, `Username`, `MessageId`
- Mail state machine: `Root` → `Transport` → `LocalDelivery` → `Error` → `Ghost`
- Mailet trait and `MailetAction` enum for composable message processing
- 11 matchers including composite `And`/`Or`/`Not` for flexible recipient filtering
- Processor chain with fork/split support and state-based mail routing
- MIME multipart parsing (RFC 2045), header folding/unfolding (RFC 5322)
- Content-Transfer-Encoding decoding (base64, quoted-printable)
- Persistent message queue with dead letter queue and atomic filesystem persistence
- Per-IP connection and message rate limiting with hot-reload support
- Sieve scripting engine (RFC 5228 parser and interpreter)
- DSN bounce message generation (RFC 3464)

#### SMTP Server (`rusmes-smtp`)
- nom-based SMTP command parser with full session state machine
- Async server with rustls TLS and integrated rate limiting
- Full command set: EHLO/HELO, MAIL FROM, RCPT TO, DATA, QUIT, STARTTLS, AUTH
- AUTH mechanisms: PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256 via SASL
- SMTP extensions: PIPELINING, 8BITMIME, SMTPUTF8, SIZE, DSN (RFC 3461), BDAT/CHUNKING (RFC 3030)
- Submission server on port 587 with mandatory STARTTLS and AUTH

#### IMAP Server (`rusmes-imap`)
- nom-based IMAP parser (1,222 lines) with LITERAL+ awareness
- Full command set: LOGIN, SELECT, EXAMINE, FETCH, STORE, SEARCH, APPEND, LIST, CREATE, DELETE, RENAME, LSUB, SUBSCRIBE/UNSUBSCRIBE, COPY, MOVE, EXPUNGE, CLOSE, IDLE, NAMESPACE, CAPABILITY, NOOP, LOGOUT
- UID variants of all applicable commands
- IMAP extensions: CONDSTORE, QRESYNC (RFC 7162), LITERAL+ (RFC 7888), SPECIAL-USE (RFC 6154), UIDPLUS (RFC 4315), MOVE (RFC 6851), IDLE (RFC 2177), NAMESPACE (RFC 2342)
- SASL AUTHENTICATE with multi-step processing

#### POP3 Server (`rusmes-pop3`)
- Full RFC 1939 implementation: USER/PASS, STAT, LIST, RETR, DELE, QUIT, RSET, NOOP, TOP, UIDL
- APOP (MD5 digest), STLS (STARTTLS, RFC 2595), and CAPA extension

#### JMAP Server (`rusmes-jmap`)
- axum-based HTTP API server with session endpoint (`/.well-known/jmap`)
- Request validation and structured error responses (RFC 8620)
- Email: get/set/query/changes/copy/import/parse method support
- EmailSubmission, Mailbox (get/set/query/changes), Thread (get/changes)
- SearchSnippet/get, Identity/*, VacationResponse/* support
- Blob download/upload and EventSource push (SSE) for real-time updates

#### Storage Layer (`rusmes-storage`)
- Trait-based storage abstraction: `MailboxStore`, `MessageStore`, `MetadataStore`, `QuotaStore`
- Filesystem backend (maildir format) with flags, atomic delivery, subscriptions, quota (1,582 lines)
- PostgreSQL backend with connection pool, full-text search, quota, MODSEQ tracking (2,592 lines)
- AmateRS distributed storage backend (mock implementation)
- Prometheus-compatible storage metrics (1,000 lines)

#### Mailets (`rusmes-core`, 16 mailets)
- AddHeader, LocalDelivery, RemoteDelivery, Bounce, RemoveMimeHeader
- DkimVerify (ed25519/RSA, 962 lines), SpfCheck (RFC 7208, 972 lines), DmarcVerify
- SpamAssassin integration (spamd protocol), VirusScan (ClamAV clamd)
- ForwardMailet (1,121 lines), SieveMailet (RFC 5228 Sieve script execution)
- OxiFYMailet for AI-powered mail analysis (1,411 lines)
- LegalisMailet for legal archiving integration (1,040 lines)
- DNSBL and Greylist mailets for spam prevention

#### Authentication (`rusmes-auth`)
- `AuthBackend` trait with full user lifecycle management
- Backends: File (bcrypt), LDAP (802 lines), SQL (1,154 lines), OAuth2/OIDC (1,469 lines), PAM (feature-gated)
- SASL mechanisms (1,495 lines): PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256, XOAUTH2
- Security hardening: brute-force protection, password strength validation, audit logging, IP rate limiting (885 lines)

#### Full-text Search (`rusmes-search`)
- `SearchIndex` trait for backend-agnostic search
- Tantivy-based search index: index, delete, search, commit operations

#### Configuration (`rusmes-config`)
- TOML and YAML auto-detection with 30+ `RUSMES_*` environment variable overrides
- Configuration validation and hot-reload via SIGHUP
- Full protocol sections: SMTP, IMAP, JMAP, POP3, Storage, Auth, Queue, Security, Metrics, Tracing, Connections

#### Observability (`rusmes-metrics`)
- Prometheus metrics with histograms and HTTP `/metrics` endpoint
- OpenTelemetry tracing (OTLP gRPC/HTTP)
- Structured logging with session UUID correlation
- Health, readiness, and liveness check endpoints
- Grafana dashboard definition (16 panels)

#### CLI Tool (`rusmes-cli`)
- clap-based CLI: init, start, stop, user, mailbox, queue, metrics, check-config, status
- Backup (1,099 lines), restore (943 lines), migrate (1,217 lines) commands
- User and mailbox management subcommands

#### ACME / TLS (`rusmes-acme`)
- ACME v2 protocol client with automatic certificate renewal
- HTTP-01 and DNS-01 challenge support
- Certificate and CSR generation via rcgen

#### Load Testing (`rusmes-loadtest`)
- Multi-protocol load test runner for SMTP, IMAP, and JMAP
- HDR histogram latency reporting
- Interactive TUI dashboard via ratatui

#### Deployment
- Multi-stage Dockerfile for minimal production images
- docker-compose stack: RusMES + PostgreSQL + Prometheus + Grafana
- systemd service unit file
- Kubernetes manifests and Helm chart
- Graceful shutdown and UNIX signal handling

#### Testing & Quality
- 1,602+ unit tests (100% passing), integration tests, RFC compliance tests
- Fuzz testing (5 targets), load testing, performance benchmarks
- Zero-warnings policy enforced across all crates

[0.1.0]: https://github.com/cool-japan/rusmes/releases/tag/v0.1.0
