# rusmes-metrics TODO

## Implemented ✅
### Prometheus Metrics (664 lines)
- [x] SMTP/IMAP/JMAP/Queue/Storage counters + histograms
- [x] Prometheus text format exporter
- [x] `/metrics` HTTP endpoint (axum, configurable bind address)
- [x] Health check endpoints (`/health`, `/ready`, `/live`) for Kubernetes probes
- [x] Histogram for message processing latency
- [x] Histogram for SMTP session duration

### OpenTelemetry (477 lines)
- [x] OTLP exporter (gRPC + HTTP)
- [x] SMTP/IMAP/JMAP/Mailet pipeline span generation
- [x] Distributed tracing with span propagation

### Dashboards
- [x] Grafana dashboard JSON template (16 panels)
- [x] Alert rules

## Remaining
- [x] Basic auth for metrics endpoint (optional) (landed 2026-05-03)
  - **Goal:** Optional `[metrics.basic_auth] { username, password_hash }` config block. Endpoint handler verifies the `Authorization: Basic` header against the bcrypt hash. 401 on miss/mismatch; pass-through on success.
  - **Implementation:** `MetricsBasicAuthConfig { username, password_hash }` added to `rusmes-config::MetricsConfig` (back-compat: `#[serde(default)]` so existing TOML keeps working). `MetricsCollector::build_router` applies an axum `from_fn_with_state` middleware that decodes `Authorization: Basic`, runs constant-time username compare, then `bcrypt::verify` against the stored hash. 401 returns `WWW-Authenticate: Basic realm="rusmes-metrics"`. The `start_http_server` path delegates to `build_router` so HTTP-server callers and embedded-router test callers go through the same code.
  - **Tests:** `test_metrics_basic_auth_accepts_correct_credentials` (200 with correct creds + 401 without), `test_metrics_basic_auth_rejects_wrong_credentials` (wrong user + wrong password both 401), `test_metrics_no_basic_auth_serves_anonymously` (back-compat), `test_constant_time_eq`.
- [x] Active connections gauge per protocol (SMTP, IMAP, JMAP, POP3) (landed 2026-05-03)
  - **Goal:** Per-protocol gauge with label `protocol`. Each protocol server calls inc on accept and dec on close.
  - **Implementation:** New `MetricsCollector::inc_active_connections / dec_active_connections / connection_guard` (RAII wrapper that decrements on Drop so the gauge round-trips through ?, panic, and early break). Backed by `Arc<DashMap<String, Arc<AtomicI64>>>` (i64 so misuse surfaces as a negative value, not a wraparound). Added a `global_metrics()` `OnceLock` singleton so protocol crates can record events without threading the handle through every constructor.
    - `rusmes-smtp/src/session.rs::SmtpSessionHandler::handle()`: `let _conn_guard = global_metrics().connection_guard("smtp");`
    - `rusmes-imap/src/server.rs::handle_connection()`: `let _conn_guard = global_metrics().connection_guard("imap");`
    - `rusmes-jmap/src/api.rs`: new `metrics_middleware` (axum `from_fn`) layered on both `JmapServer::routes` and `routes_with_auth`. JMAP's `session.rs` is the RFC 8620 Session response object, not connection lifecycle — see deviation note below.
    - POP3 is owned by Cluster 5 (per task scope: "NOT pop3").
  - **Exposition:** `rusmes_active_connections{protocol="smtp|imap|jmap|..."}`. Sorted-label deterministic output for diff-friendly scrape parsers and stable test assertions.
  - **Tests:** `test_active_connections_guard_roundtrip` exercises nested guards on the same protocol (gauge counts up then back to zero) and per-protocol isolation (smtp vs imap).
- [x] TLS connection counters (plaintext vs encrypted) (landed 2026-05-03)
  - **Goal:** `IntCounterVec` with label `tls in {yes, no, starttls}` per the cluster spec.
  - **Implementation:** `rusmes_tls_sessions_total{tls=...}` exported as a counter. `pub mod tls_label { YES, NO, STARTTLS }` constants prevent stringly-typed drift across protocol crates. Increment sites:
    - SMTP `handle()`: `inc_tls_session(NO)` on session start (plaintext path); `handle_starttls()`: `inc_tls_session(STARTTLS)` on the upgrade-agreed event.
    - IMAP `handle_connection()`: `inc_tls_session(NO)` on session start. (IMAP STARTTLS is not yet implemented in the codebase; the upgrade site has no handler to instrument.)
    - JMAP middleware: `inc_tls_session(NO)`. JMAP's TLS termination happens upstream of axum (rustls-axum or a fronting proxy) so the application layer can't reliably distinguish — the doc-comment in `metrics_middleware` flags this for future work.
    - Implicit-TLS listeners (smtps:465, imaps:993) are not wired in `rusmes-server` yet; when added, the listener-side wrapper should call `inc_tls_session(YES)` on each accepted stream.
  - **Tests:** `test_tls_session_counter_labels` verifies all three labels accumulate independently and the exposition format matches `rusmes_tls_sessions_total{tls="..."} N`.
- [x] Per-domain message counters (landed 2026-05-03)
  - **Goal:** Receives data from Cluster 4's `MailQueue::queue_stats_per_domain()`; exposes as a Prometheus counter keyed by recipient domain.
  - **Implementation:** `MetricsCollector::set_domain_stats_source(Arc<dyn Fn() -> HashMap<String,u64> + Send + Sync>)` lets `rusmes-server` register `Arc::new(move || queue.queue_stats_per_domain())` at boot. The metrics layer holds no direct dep on rusmes-core (avoids the `metrics ↔ core` cycle). On every scrape, `export_prometheus()` calls `refresh_domain_stats_now()` which pulls the snapshot through the callback and mirrors it into `Arc<DashMap<String, Arc<AtomicU64>>>`. A `spawn_domain_stats_refresher(period)` background task is provided for embedders that prefer a periodic push (e.g. when the source is expensive to call per-scrape).
  - **Exposition:** `rusmes_messages_per_domain_total{domain="example.com"}`. Label values run through `escape_label_value` (handles `\`, `"`, `\n` per the Prometheus exposition spec).
  - **Tests:** `test_messages_per_domain_from_callback_source` registers a stub source, scrapes, and asserts both labels surface with the right counts. `test_escape_label_value_quotes_and_backslash` covers the escaping path.

## Cluster 7 deviation notes (2026-05-03)

1. **JMAP `session.rs` is not a connection-lifecycle file.** The cluster plan said to add inc/dec to `crates/rusmes-jmap/src/session.rs`, but that file defines the RFC 8620 `Session` response object — there is no accept/close lifecycle to hook. The instrumentation lives in `crates/rusmes-jmap/src/api.rs` as an axum middleware (`metrics_middleware`) layered on `JmapServer::routes` and `routes_with_auth`. JMAP "session" here means "in-flight HTTP request", which is the operationally meaningful gauge.
2. **IMAP instrumentation is in `server.rs::handle_connection()` rather than `session.rs`.** IMAP's `session.rs` only defines the `ImapSession` state machine; the actual per-connection handler is `server.rs::handle_connection()`. Same gauge semantics, more honest location.
3. **POP3 is out of scope** per the task brief ("NOT pop3 — Cluster 5 is handling that file"). When Cluster 5 lands, it should add `let _g = rusmes_metrics::global_metrics().connection_guard("pop3");` and an `inc_tls_session` call alongside its maildrop locking work.
4. **Implicit-TLS listeners (smtps:465, imaps:993) not yet wired.** When `rusmes-server` adds those listener variants, they should call `inc_tls_session(rusmes_metrics::tls_label::YES)` on accept. The `tls_label::YES` constant exists for this purpose.
5. **Per-domain via callback, not direct dep.** A direct `rusmes-metrics → rusmes-core` dep would create a cycle (rusmes-core already depends on rusmes-metrics for its `MailProcessorRouter::new(metrics)` constructor). The callback approach (`set_domain_stats_source`) puts the cycle break in the embedder (`rusmes-server`), which is the only crate that depends on both.
