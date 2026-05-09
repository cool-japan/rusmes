# rusmes-smtp TODO

## Implemented ✅
- [x] nom-based SMTP command parser (HELO/EHLO/MAIL/RCPT/DATA/BDAT/AUTH/STARTTLS etc.)
- [x] Full session state machine (Initial → Connected → Authenticated → MailTransaction → Data → Quit)
- [x] SMTP response builder
- [x] Async server with TLS (rustls) and rate limiter integration
- [x] PIPELINING (RFC 2920), 8BITMIME (RFC 6152), SMTPUTF8 (RFC 6531)
- [x] SIZE (RFC 1870) — declared and enforced
- [x] DSN (RFC 3461) — delivery status notifications (586 lines)
- [x] BDAT/CHUNKING (RFC 3030) — binary data transfer (769 lines)
- [x] AUTH: PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256
- [x] Relay authorization based on `relay_networks` config
- [x] Recipient validation against storage backend
- [x] Submission server (port 587, mandatory STARTTLS + AUTH)
- [x] Connection timeout enforcement
- [x] MailTransport trait abstraction for JMAP EmailSubmission integration; SmtpMailTransport client with send/send_at/cancel (landed 2026-05-05)

## Remaining
- [x] SCRAM-SHA-256: salt/iteration count are placeholder values — needs real credential storage (landed 2026-05-03 via Cluster 1D)
  - **Outcome.** `crates/rusmes-smtp/src/session.rs::handle_auth_scram_sha256` now calls `auth_backend.fetch_scram_credentials(user)` and uses the user's real salt + iteration count in the server-first message. `handle_scram_client_final` verifies the client proof against the stored key (RFC 5802 §3 AuthMessage construction) and emits `v=<server signature>` (base64) on the 235 success line per RFC 4954 §6. When the backend returns `Ok(None)` the server replies `504 5.5.4 SCRAM-SHA-256 mechanism not available for this user`; on credential-lookup error it returns `454 4.7.0 Temporary authentication failure`.
  - **Test added.** `scram_rejected_when_credentials_missing` (in `session::tests`) drives `handle_auth_scram_sha256` with a backend that inherits the trait default (`Ok(None)`) and asserts the 504 reply, no session-level authentication, and no retained SCRAM state.
  - **Follow-up.** A full `scram_sha256_roundtrip_against_file_backend` end-to-end happy-path test was scoped to the cluster plan but is deferred — it requires plumbing two SCRAM round-trips through `SmtpSessionHandler` (or refactoring the SCRAM helpers out of the private session struct) and is best added alongside the SASL helper extraction proposed in `rusmes-auth`'s `sasl.rs`.
- [x] Per-event metrics recording (connect, auth, message, error) (landed 2026-05-05)
  - **Outcome.** Three new counters added to `rusmes-metrics::MetricsCollector`: `smtp_auth_success_total`, `smtp_auth_failure_total`, `smtp_messages_rejected_total` (with `inc_*`, read accessors, and Prometheus export lines). Wire points in `session.rs`: `inc_smtp_connections()` at session accept; auth success/failure in `handle_auth_plain`, `handle_cram_md5_response`, `handle_auth_scram_sha256`, and `handle_scram_client_final`; `inc_smtp_messages_received()` on DATA acceptance; `inc_smtp_messages_rejected()` on size-limit rejection; `inc_smtp_errors()` on unexpected command handler errors. All calls use `rusmes_metrics::global_metrics()` (established pattern — no struct threading needed).
  - **Counters:** `smtp_connections_total` (was already present, now called at session start), `smtp_messages_received` (reused as "accepted"), `smtp_messages_rejected_total` (new), `smtp_auth_success_total` (new), `smtp_auth_failure_total` (new), `smtp_errors_total` (was already present, now wired to command-handler `Err` path).
  - **Tests added.** `test_smtp_connection_counter_increments` (unit, MetricsCollector); `test_smtp_auth_success_counter` (async, delta on global); `test_smtp_auth_failure_counter` (async, delta on global); `test_smtp_message_accepted_counter` (async, drives `handle_data_input` with in-memory reader, delta on global).

## Proposed follow-ups

Planned 2026-05-05 for implementation.

- [x] Reject connections from blocked IPs (configuration-driven CIDR list) (landed 2026-05-05)
  - **Goal:** Silent drop of connections from IPs matching `SecurityConfig.blocked_ips` CIDR list, before the SMTP banner is sent.
  - **Design:** In `server.rs::run_loop` accept arm, parse peer IP, check against `Vec<ipnetwork::IpNetwork>` (pre-built at startup from config). On match: `tracing::info!` + drop socket, increment `smtp_connections_rejected_blocked_total` counter. `SecurityConfig.blocked_ips` and validation already exist in `crates/rusmes-config/src/lib.rs:1472`.
  - **Files:** `crates/rusmes-smtp/src/server.rs`, `crates/rusmes-metrics/src/lib.rs`
  - **Tests:** `test_blocked_ip_rejected` — bind server with `blocked_ips: ["127.0.0.1/32"]`, connect, assert `read_to_end == 0` and counter +1.
  - **Risk:** Silent drop vs 554 banner — RFC 5321 permits silent drop for blocked IPs; chosen for rate-limit resistance.
- [x] Max connections per IP enforcement (landed 2026-05-05)
  - **Goal:** Reject (421) new connections from an IP that already has `max_connections_per_ip` active sessions.
  - **Design:** Same accept site in `server.rs`. Call `rate_limiter.count_active(&RateLimitKey::Ip(ip)) >= config.security.rate_limit.max_connections_per_ip`. On exceed: write `421 4.7.0 Too many concurrent connections from your IP\r\n`, flush, drop. Counter `smtp_connections_rejected_overload_total`. `count_active` already exists at `crates/rusmes-core/src/rate_limit.rs:340`. Config field `max_connections_per_ip` already at `crates/rusmes-config/src/lib.rs:1248`.
  - **Files:** `crates/rusmes-smtp/src/server.rs`, `crates/rusmes-metrics/src/lib.rs`
  - **Tests:** `test_max_connections_per_ip` — limit=2, open 3 connections from same IP, third gets `421 4.7.0`.
  - **Risk:** Requires the server to thread the `RateLimiter` Arc into the accept loop.
- [x] Idle connection reaping (configurable timeout, 421 close) (landed 2026-05-05)
  - **Goal:** Emit `421 4.4.2 Connection timed out due to inactivity\r\n` and close when the configured idle timeout fires, rather than silently dropping.
  - **Design:** `session.rs:271-273` already wraps command-read in `tokio::time::timeout(idle_timeout, ...)`. Audit the `Elapsed` error path — if it currently just `?`-propagates (silent drop), change it to write the `421` response, flush, then return. If already correct, add a regression test. Add `idle_timeout` to the per-connection tracing span.
  - **Files:** `crates/rusmes-smtp/src/session.rs`
  - **Tests:** `test_idle_timeout_421` — connect, send EHLO, sleep 200ms with `idle_timeout=100ms`, assert next read returns `421 4.4.2`.
  - **Risk:** session.rs is 1,737 lines; run `rslines 50` after edits and `splitrs` if it crosses 2000.
- [x] Per-session structured logging (session ID, client IP) (landed 2026-05-05)
  - **Goal:** Every log event emitted during an SMTP session carries a unique `session_id` field and the client `peer` IP, enabling per-session log aggregation.
  - **Design:** Build a `tracing::Span` at session start: `let span = tracing::info_span!("smtp.session", session_id = %Uuid::new_v4(), peer = %peer_addr);` and `.instrument(span)` the entire session future. Log key events: connect, EHLO/HELO, AUTH result, MAIL FROM, RCPT TO, DATA result, QUIT, idle close, error close. Replace any bare `eprintln!` with `info!` / `warn!` inside the span.
  - **Files:** `crates/rusmes-smtp/src/session.rs`, `crates/rusmes-smtp/src/server.rs`
  - **Tests:** `test_session_span_carries_id` — drive a session through `tracing_test::traced_test` and assert every emitted log carries the same `session_id` field.
  - **Risk:** session.rs line count; run rslines 50 after edits.
- [x] Mutual TLS (client certificate verification via rustls) (landed 2026-05-05)
  - **Goal:** Submission server (port 587/465) optionally requires or accepts client TLS certificates, verified against a configured CA. Post-handshake cert chain stored on session for future AUTH EXTERNAL use.
  - **Design:** Extend `TlsEndpointConfig` (`crates/rusmes-config/src/tls.rs`) with `client_ca_path: Option<PathBuf>` and `client_auth: ClientAuthMode { Disabled, Optional, Required }` (default `Disabled`). When `client_auth != Disabled`, build rustls `ServerConfig` with `WebPkiClientVerifier::builder(roots)` loading CA from `client_ca_path`. After STARTTLS, capture `ServerConnection.peer_certificates()` into `SmtpSession` as `Option<Vec<rustls::pki_types::CertificateDer<'static>>>`. AUTH EXTERNAL not wired in this slice — cert stored only.
  - **Files:** `crates/rusmes-config/src/tls.rs`, `crates/rusmes-smtp/src/submission.rs`
  - **Tests:** `test_mtls_required_rejects_no_cert`, `test_mtls_optional_accepts_no_cert`, `test_mtls_required_accepts_signed_cert`.
  - **Risk:** rustls `WebPkiClientVerifier` API changed in rustls 0.23 — verify exact builder API against the workspace rustls version.
- [x] Outbound connection pooling for remote delivery (landed 2026-05-05)
  - **Goal:** Reuse SMTP connections to the same remote MX across multiple deliveries, reducing TCP+TLS handshake overhead.
  - **Design:** New `OutboundPool` struct in new `crates/rusmes-smtp/src/outbound_pool.rs`. Uses `dashmap::DashMap<SocketAddr, VecDeque<PooledSmtpConn>>`. Each `PooledSmtpConn` carries stream + `SystemTime` last-use + negotiated extensions. `get_or_connect(remote)` returns front of deque or opens new; on drop returns to tail. Background reaper drops idle > `outbound.idle_timeout` (default 30s). Pool sends `RSET\r\n` + waits for `250` before MAIL FROM reuse; on any error the conn is dropped. Per-remote cap (default 8), global cap (default 256). `MailTransport` impl in `transport.rs` acquires from pool.
  - **Files:** new `crates/rusmes-smtp/src/outbound_pool.rs`, `crates/rusmes-smtp/src/transport.rs`, `crates/rusmes-config/src/lib.rs` (add `[smtp.outbound]` section)
  - **Tests:** `test_outbound_pool_reuse` — assert only one TCP connect for two back-to-back deliveries. `test_outbound_pool_idle_reaper`. `test_outbound_pool_rset_reuse`.
  - **Risk:** Shutdown ordering — pool must flush on server shutdown; wire via the existing shutdown channel.
