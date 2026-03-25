# RusMES TODO

## Legend
- [ ] Not started
- [x] Complete
- [-] In progress / partial

---

## Project Status

**Last Updated**: 2026-03-25
**Total Lines of Code**: ~87,579 Rust lines (71,926 code lines)
**Crates**: 17 (including rusmes-acme, rusmes-loadtest)
**Test Coverage**: 1,942 unit tests passing (100%), integration tests require live server
**Build Status**: ✅ Clean (ZERO warnings from our code)
**Release**: v0.1.1 (2026-03-25) 🎉

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
- [-] SCRAM-SHA-256 salt/iteration count — placeholder values

## IMAP Server ✅
- [x] nom-based parser (1,222 lines, LITERAL+ aware)
- [x] Full command set: LOGIN, SELECT, EXAMINE, FETCH, STORE, SEARCH, APPEND, LIST, CREATE, DELETE, RENAME, LSUB, SUBSCRIBE/UNSUBSCRIBE, COPY, MOVE, EXPUNGE, CLOSE, IDLE, NAMESPACE, CAPABILITY, NOOP, LOGOUT
- [x] UID variants of all applicable commands
- [x] Extensions: CONDSTORE, QRESYNC (RFC 7162), LITERAL+ (RFC 7888), SPECIAL-USE (RFC 6154), UIDPLUS (RFC 4315), MOVE (RFC 6851), IDLE (RFC 2177), NAMESPACE (RFC 2342)
- [x] SASL AUTHENTICATE (multi-step processing)
- [ ] COMPRESS=DEFLATE (RFC 4978)
- [ ] Concurrent mailbox change notifications across sessions

## POP3 Server ✅
- [x] Full RFC 1939 implementation: USER/PASS, STAT, LIST, RETR, DELE, QUIT, RSET, NOOP, TOP, UIDL
- [x] APOP (MD5 digest), STLS (STARTTLS, RFC 2595), CAPA

## JMAP Server ✅ (auth integration incomplete)
- [x] axum-based API server with session endpoint (`/.well-known/jmap`)
- [x] Request validation, error responses (RFC 8620)
- [x] Email: get/set/query/changes/copy/import/parse
- [x] EmailSubmission, Mailbox (get/set/query/changes), Thread (get/changes)
- [x] SearchSnippet/get, Identity/*, VacationResponse/*
- [x] Blob download/upload, EventSource push (SSE)
- [-] **Authentication**: Basic/Bearer hardcoded — needs real AuthBackend integration
- [ ] Blob storage persistence (currently in-memory)
- [ ] Back-reference resolution between method calls

## Storage Backends ✅
- [x] Filesystem (maildir) — flags, atomic delivery, subscriptions, quota (1,582 lines)
- [x] PostgreSQL — connection pool, full-text search, quota, MODSEQ tracking (2,592 lines)
- [-] AmateRS — **mock implementation** (no real distributed client)
- [x] Storage metrics (Prometheus-compatible, 1,000 lines)
- [ ] Directory locking for concurrent filesystem access
- [ ] Database migration tooling (sqlx-migrate or refinery)

## Mailets ✅ (16 mailets)
- [x] AddHeader, LocalDelivery, RemoteDelivery, Bounce, RemoveMimeHeader
- [x] DkimVerify (ed25519/RSA, 962 lines), SpfCheck (RFC 7208, 972 lines), DmarcVerify
- [x] SpamAssassin (spamd protocol), VirusScan (ClamAV clamd)
- [x] ForwardMailet (1,121 lines), SieveMailet (RFC 5228)
- [x] OxiFYMailet (AI analysis, 1,411 lines), LegalisMailet (legal archiving, 1,040 lines)
- [x] DNSBL, Greylist

## Authentication ✅ (server integration incomplete)
- [x] `AuthBackend` trait with full user management
- [x] File (bcrypt), LDAP (802 lines), SQL (1,154 lines), OAuth2/OIDC (1,469 lines), PAM
- [x] SASL (1,495 lines): PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256, XOAUTH2
- [x] Security: brute-force protection, password strength, audit logging, IP rate limiting (885 lines)
- [-] **Server integration**: Only file-based works — LDAP/SQL/OAuth2 fall back to `DummyAuthBackend`

## Search ✅ (minimal)
- [x] `SearchIndex` trait + `TantivySearchIndex` (index, delete, search, commit)
- [ ] HTML-to-text conversion, attachment filename indexing
- [ ] Background reindex worker, result caching, `rebuild()`

## Configuration ✅
- [x] TOML/YAML auto-detect, 30+ `RUSMES_*` env overrides, validation, hot-reload (SIGHUP)
- [x] All sections: SMTP, IMAP, JMAP, POP3, Storage, Auth, Queue, Security, Metrics, Tracing, Connections
- [ ] `[performance]` section, per-protocol TLS paths, unknown key warnings

## CLI ✅
- [x] clap-based: init, start, stop, user, mailbox, queue, metrics, check-config, status
- [x] backup (1,099 lines), restore (943 lines), migrate (1,217 lines)
- [-] `status` and `migrate` commands — partially placeholder
- [ ] Colored output, `--json` mode, shell completions, man pages

## Observability ✅
- [x] Prometheus metrics + histograms, HTTP `/metrics` endpoint
- [x] OpenTelemetry tracing (OTLP gRPC/HTTP)
- [x] Structured logging with session UUID, health/ready/live endpoints
- [x] Grafana dashboard (16 panels)

## ACME / TLS [-] (partial)
- [x] ACME v2 client, auto-renewal, HTTP-01/DNS-01 challenges, cert/CSR generation
- [-] **JWK thumbprint**: placeholder values — blocks real Let's Encrypt issuance

## Deployment ✅
- [x] Dockerfile (multi-stage), docker-compose (RusMES + PostgreSQL + Prometheus + Grafana)
- [x] systemd service, Kubernetes manifests, Helm chart
- [x] Graceful shutdown, signal handling

## Testing & Quality ✅
- [x] 1,602+ unit tests, integration tests, RFC compliance tests
- [x] Fuzz testing (5 targets), load testing, performance benchmarks
- [x] NO WARNINGS POLICY (0 warnings)

---

## 🔧 Known Issues & Remaining Work

### Critical (blocks production use)
1. **Auth backend integration** — `rusmes-server` falls back to `DummyAuthBackend` for LDAP/SQL/OAuth2
2. **JMAP authentication** — Hardcoded dev-only auth, needs `AuthBackend` integration
3. **ACME JWK thumbprint** — Placeholder values prevent real certificate issuance

### Important
4. **AmateRS backend** — Mock implementation only
5. **SMTP SCRAM-SHA-256** — Placeholder salt/iteration values
6. **JMAP blob storage** — In-memory only, no persistence
7. **JMAP back-references** — Not implemented

### Nice-to-have
8. IMAP COMPRESS=DEFLATE, concurrent mailbox notifications
9. Filesystem directory locking, DB migration tooling
10. Search enhancements (HTML extraction, indexing, caching, reindex)
11. Config `[performance]` section, per-protocol TLS, unknown key warnings
12. CLI colored output, `--json`, shell completions, man pages
13. Metrics endpoint basic auth

---

## 📊 Statistics

| Metric | Value |
|--------|-------|
| Total Lines of Code | ~87,579 Rust lines (71,926 code lines) |
| Crates | 17 |
| Unit Tests | 1,942 (100% passing) |
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