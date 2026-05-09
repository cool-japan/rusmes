# RusMES TODO

## Legend
- [ ] Not started
- [x] Complete
- [-] In progress / partial

---

## Project Status

**Last Updated**: 2026-05-09
**Total Lines of Code**: 112,187 Rust lines (92,215 code lines, 324 files)
**Crates**: 17 (including rusmes-acme, rusmes-loadtest)
**Test Coverage**: 2,309 unit tests passing (100%), 60 skipped, integration tests require live server
**Build Status**: ✅ Clean (ZERO warnings from our code)
**Release**: v0.1.2 (2026-05-09) 🎉

---

## Core Foundation ✅
- [x] Workspace structure with 17 crates
- [x] Core types (`Mail`, `MailAddress`, `Domain`, `Username`, `MessageId`)
- [x] Mail state machine (`Root` → `Transport` → `LocalDelivery` → `Error` → `Ghost`)
- [x] Mailet trait and `MailetAction` enum
- [x] Matcher trait (11 matchers including composite `And`/`Or`/`Not`)
- [x] Processor chain with fork/split support
- [x] Mail processor router with state-based routing
- [x] Storage abstraction traits (`MailboxStore`, `MessageStore`, `MetadataStore`, `QuotaStore`)
- [x] MIME multipart parsing (RFC 2045), header folding/unfolding (RFC 5322)
- [x] Content-Transfer-Encoding decoding (base64, quoted-printable)
- [x] Persistent queue with dead letter queue and atomic filesystem persistence
- [x] Rate limiting (per-IP connections + messages/time window, hot-reload)
- [x] Sieve scripting engine (RFC 5228 parser + interpreter)
- [x] DSN bounce message generation (RFC 3464)

## SMTP Server ✅
- [x] nom-based command parser, full session state machine, response builder
- [x] Async server with TLS (rustls) and rate limiting integration
- [x] Full command set: EHLO/HELO, MAIL FROM, RCPT TO, DATA, QUIT, STARTTLS, AUTH
- [x] AUTH: PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256 via SASL
- [x] Extensions: PIPELINING, 8BITMIME, SMTPUTF8, SIZE, DSN (RFC 3461), BDAT/CHUNKING (RFC 3030)
- [x] Submission server (port 587, mandatory STARTTLS + AUTH)
- [x] SCRAM-SHA-256 salt/iteration count — real credential storage via AuthBackend (landed 2026-05-03)

## IMAP Server ✅
- [x] nom-based parser (1,222 lines, LITERAL+ aware)
- [x] Full command set: LOGIN, SELECT, EXAMINE, FETCH, STORE, SEARCH, APPEND, LIST, CREATE, DELETE, RENAME, LSUB, SUBSCRIBE/UNSUBSCRIBE, COPY, MOVE, EXPUNGE, CLOSE, IDLE, NAMESPACE, CAPABILITY, NOOP, LOGOUT
- [x] UID variants of all applicable commands
- [x] Extensions: CONDSTORE, QRESYNC (RFC 7162), LITERAL+ (RFC 7888), SPECIAL-USE (RFC 6154), UIDPLUS (RFC 4315), MOVE (RFC 6851), IDLE (RFC 2177), NAMESPACE (RFC 2342)
- [x] SASL AUTHENTICATE (multi-step processing)
- [x] COMPRESS=DEFLATE (RFC 4978) (landed 2026-05-05)
- [x] Concurrent mailbox change notifications across sessions (completed 2026-05-05)

## POP3 Server ✅
- [x] Full RFC 1939 implementation: USER/PASS, STAT, LIST, RETR, DELE, QUIT, RSET, NOOP, TOP, UIDL
- [x] APOP (MD5 digest), STLS (STARTTLS, RFC 2595), CAPA

## JMAP Server ✅
- [x] axum-based API server with session endpoint (`/.well-known/jmap`)
- [x] Request validation, error responses (RFC 8620)
- [x] Email/set: create + update + destroy (RFC 8620 §5.1)
- [x] Email: get/query/changes/copy/import/parse
- [x] Identity/set: create + update + destroy (RFC 8620 §5.3), file-backed IdentityStore
- [x] VacationResponse/set: create + update + destroy, file-backed VacationStore
- [x] EmailSubmission/set: create + update + destroy (RFC 8620 §7), MailTransport abstraction
- [x] Mailbox (get/set/query/changes), Thread (get/changes)
- [x] SearchSnippet/get, Blob download/upload, EventSource push (SSE)
- [x] RFC 5256 email threading (References-chain + subject fallback, SHA-256 thread IDs, per-mailbox .thread_index.json)
- [x] **Bearer token authentication**: `AuthBackend::verify_bearer_token` (OAuth2 backend override)
- [x] Blob storage persistence (currently in-memory)
- [x] Back-reference resolution between method calls
- [x] **JMAP correctness**: `EmailConversionContext`, `compute_blob_id` (SHA-256/RFC 8620), `jmap_keywords_from_flags` (RFC 8621 §4.1.1) — all placeholder values eliminated (landed 2026-05-06)

## Storage Backends ✅
- [x] Filesystem (maildir) — flags, atomic delivery, subscriptions, quota (1,582 lines)
- [x] PostgreSQL — connection pool, full-text search, quota, MODSEQ tracking (2,592 lines)
- [x] AmateRS — real client integration landed 2026-05-05 (`amaters-sdk-rust v0.2.0` wired; feature-gate `amaters-backend`; `AmatersBackend::connect_real` available)
- [x] Storage metrics (Prometheus-compatible, 1,000 lines)
- [x] Directory locking for concurrent filesystem access (landed 2026-05-05)
- [x] Database migration tooling (sqlx migrate + SQLite backend) (landed 2026-05-05)

## Mailets ✅ (16 mailets)
- [x] AddHeader, LocalDelivery, RemoteDelivery, Bounce, RemoveMimeHeader
- [x] DkimVerify (ed25519/RSA, 962 lines), SpfCheck (RFC 7208, 972 lines), DmarcVerify
- [x] SpamAssassin (spamd protocol), VirusScan (ClamAV clamd)
- [x] ForwardMailet (1,121 lines), SieveMailet (RFC 5228)
- [x] OxiFYMailet (AI analysis, 1,411 lines), LegalisMailet (legal archiving, 1,040 lines)
- [x] DNSBL, Greylist

## Authentication ✅
- [x] `AuthBackend` trait with full user management
- [x] File (bcrypt), LDAP (802 lines), SQL (1,154 lines), OAuth2/OIDC (1,469 lines), PAM
- [x] SASL (1,495 lines): PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256, XOAUTH2
- [x] Security: brute-force protection, password strength, audit logging, IP rate limiting (885 lines)
- [x] **Server integration**: `AuthBackendKind` config-driven factory; LDAP/SQL/OAuth2 no longer fall back to `DummyAuthBackend` (landed 2026-05-05)

## Search ✅
- [x] `SearchIndex` trait + `TantivySearchIndex` (index, delete, search, commit)
- [x] HTML-to-text conversion, attachment filename indexing
- [x] Background reindex worker, result caching, `rebuild()` (landed 2026-05-03)

## Configuration ✅
- [x] TOML/YAML auto-detect, 30+ `RUSMES_*` env overrides, validation, hot-reload (SIGHUP)
- [x] All sections: SMTP, IMAP, JMAP, POP3, Storage, Auth, Queue, Security, Metrics, Tracing, Connections
- [x] `[performance]` section, per-protocol TLS paths, unknown key warnings

## CLI ✅
- [x] clap-based: init, start, stop, user, mailbox, queue, metrics, check-config, status
- [x] backup (1,099 lines), restore (943 lines), migrate (1,217 lines)
- [x] `status` and `migrate` commands — fully implemented (landed 2026-05-05)
- [x] Colored output, `--json` mode, shell completions, man pages (landed 2026-05-05)

## Observability ✅
- [x] Prometheus metrics + histograms, HTTP `/metrics` endpoint on port 9090
- [x] Active connections gauge per protocol (RAII `ConnectionGuard` via `Arc<AtomicI64>`)
- [x] OpenTelemetry tracing (OTLP gRPC/HTTP)
- [x] Structured logging with session UUID, health/ready/live endpoints
- [x] Grafana dashboard (16 panels)

## ACME / TLS ✅
- [x] ACME v2 client, auto-renewal, HTTP-01/DNS-01 challenges, cert/CSR generation
- [x] **JWK thumbprint**: RFC 7638 SHA-256 thumbprint implemented (landed 2026-05-03)

## Deployment ✅
- [x] Dockerfile (multi-stage), docker-compose (RusMES + PostgreSQL + Prometheus + Grafana)
- [x] systemd service, Kubernetes manifests, Helm chart
- [x] Graceful shutdown, signal handling

## Testing & Quality ✅
- [x] 2,309 unit tests passing (60 skipped), integration tests, RFC compliance tests
- [x] Fuzz testing (5 targets), load testing, performance benchmarks
- [x] NO WARNINGS POLICY (0 warnings)

---

## 🔧 Known Issues & Remaining Work

### Critical (resolved)
1. ~~**Auth backend integration**~~ — ✅ `AuthBackendKind` factory wired; no DummyAuthBackend fallback
2. ~~**JMAP authentication**~~ — ✅ Bearer token + Basic auth via real `AuthBackend` (landed 2026-05-03)
3. ~~**ACME JWK thumbprint**~~ — ✅ RFC 7638 SHA-256 thumbprint implemented (landed 2026-05-03)

### Still Open
4. [x] **AmateRS backend** — real client integration landed 2026-05-05 (`amaters-sdk-rust v0.2.0` wired; feature-gate `amaters-backend`)
5. ~~**SMTP SCRAM-SHA-256**~~ — ✅ Real salt/iteration count via AuthBackend (landed 2026-05-03)
6. ~~**JMAP blob storage**~~ — ✅ Filesystem-backed BlobStorage with persistence (landed 2026-05-05)
7. ~~**JMAP back-references**~~ — ✅ RFC 8620 §3.7 back-reference resolution implemented (landed 2026-05-05)

### Nice-to-have
8. ~~IMAP COMPRESS=DEFLATE~~ — ✅ `oxiarc-deflate 0.2.7` async streaming adapter + session stream-swap landed 2026-05-05
9. ~~Filesystem directory locking~~, ~~DB migration tooling~~ — ✅ both landed 2026-05-05
10. ~~Search enhancements~~ (HTML extraction, indexing, caching, reindex) — ✅ all landed 2026-05-03/05
11. ~~Config `[performance]` section, per-protocol TLS, unknown key warnings~~ — ✅ landed 2026-05-05
12. ~~CLI colored output, `--json`, shell completions, man pages~~ — ✅ landed 2026-05-05
13. ~~Metrics endpoint basic auth~~ — ✅ landed 2026-05-03
14. ~~Server privilege drop (setuid/chroot)~~ — ✅ `PrivilegeDrop` with chroot+setgid+setuid (Linux-only); `[server] run_as_user/run_as_group/chroot` config fields landed 2026-05-06

---

## 📊 Statistics

| Metric | Value |
|--------|-------|
| Total Lines of Code | 112,187 Rust lines (92,215 code lines, 324 files) |
| Crates | 17 |
| Unit Tests | 2,309 (100% passing, 60 skipped) |
| Warnings | 0 |
| Protocols | SMTP, IMAP, POP3, JMAP |
| Auth Backends | 5 (File, LDAP, SQL, OAuth2, PAM) |
| SASL Mechanisms | 5 (PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256, XOAUTH2) |
| Storage Backends | 3 (Filesystem, PostgreSQL, AmateRS) |
| Mailets | 16 |
| Matchers | 11 |
| SMTP Extensions | 8 |
| IMAP Extensions | 9 |
| POP3 Extensions | 4 |

## Last updated: 2026-05-09