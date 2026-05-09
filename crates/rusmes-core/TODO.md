# rusmes-core TODO

## Implemented ✅
### Mailet Engine
- [x] `Mailet` trait (async init/service/destroy), `MailetAction` enum
- [x] `Matcher` trait + `MatchResult`
- [x] Processor chain with fork/split support (partial match handling)
- [x] Mail processor router with state-based routing (depth limit = 100)

### Mailets (16)
- [x] AddHeader, Bounce, DkimVerify (962 lines), DmarcVerify, DNSBL
- [x] Forward (1,121 lines), Greylist, Legalis (1,040 lines), LocalDelivery
- [x] OxiFY (1,411 lines), RemoteDelivery, RemoveMimeHeader, Sieve
- [x] SpamAssassin (spamd protocol), SpfCheck (972 lines), VirusScan (ClamAV clamd)

### Matchers (11)
- [x] All, None, RecipientIsLocal, SenderIs, HasAttachment
- [x] SizeGreaterThan, HeaderContains, RemoteAddress (CIDR)
- [x] IsInWhitelist, IsInBlacklist, Composite (And/Or/Not)

### Other
- [x] DSN bounce message generation (RFC 3464)
- [x] Rate limiter (per-IP, hot-reload capable)
- [x] Persistent queue + dead letter queue + priority queue
- [x] Sieve scripting engine (RFC 5228 parser + interpreter)

## Remaining
- [x] Mailet execution timeout (configurable per-mailet) (planned 2026-05-03)
  - **Goal:** Mailet pipeline survives slow/hostile mailets; each mailet gets an optional `timeout_ms` budget.
  - **Files:** `crates/rusmes-core/src/mailet.rs` (add `timeout_ms: Option<u64>` to `MailetConfig`); `crates/rusmes-core/src/processor.rs` (wrap `service(envelope)` in `tokio::time::timeout`, emit `MailetError::Timeout` on elapse).
  - **Tests:** A mailet that sleeps 200 ms with a 50 ms timeout produces `MailetError::Timeout`.
  - **Risk:** Default disabled — no impact on existing pipelines without `timeout_ms` set.
- [x] Mailet error handling policy (skip, abort, retry) (planned 2026-05-03)
  - **Goal:** Router has a configurable contract on `Err(_)` / `Err(Timeout)`: continue the pipeline, abort it (4xx/5xx upstream), or re-enqueue with bounded retries.
  - **Files:** `crates/rusmes-core/src/mailet.rs` (new enum `MailetErrorPolicy { Skip, Abort, Retry { max: u32, backoff: Duration } }` stored on `MailetConfig`); `crates/rusmes-core/src/processor.rs` (consult policy on error).
  - **Tests:** `Skip` continues pipeline; `Abort` propagates; `Retry { max: 2 }` re-enqueues twice then aborts.
  - **Risk:** Retry path re-uses the existing queue — no new queue infrastructure required.
- [x] Queue priority levels (currently flat) (planned 2026-05-03)
  - **Goal:** Three priority levels (High / Normal / Low); bounce/notification jobs default High; outbound delivery Normal; reindex Low. Configurable per submission site.
  - **Files:** `crates/rusmes-core/src/queue.rs` (replace single `tokio::sync::mpsc` with `BinaryHeap<PriorityEnvelope>` behind `Mutex` + `Notify`).
  - **Tests:** Enqueue 1× High, 2× Low, 1× Normal — assert dequeue order is High, Normal, Low, Low.
  - **Risk:** BinaryHeap requires `Ord` on `PriorityEnvelope`; implement carefully to avoid priority inversion.
- [x] Queue statistics per destination domain (planned 2026-05-03)
  - **Goal:** `pub fn queue_stats_per_domain() -> HashMap<String, u64>` exposed for Prometheus consumption by rusmes-metrics (Cluster 7).
  - **Files:** `crates/rusmes-core/src/queue.rs` (add `Arc<DashMap<String, AtomicU64>>` updated on every queue insert); `crates/rusmes-core/Cargo.toml` (add `dashmap` if not present).
  - **Tests:** Enqueue 5 messages to `example.com`, 3 to `example.org`, assert counts.
  - **Risk:** `DashMap` + `AtomicU64` are already common in the workspace; no new policy conflicts.
- [x] Per-sender rate limiting (currently per-IP only) (planned 2026-05-03)
  - **Goal:** Rate limiter enforces both per-IP and per-MAIL-FROM axes simultaneously; SMTP session passes `MAIL FROM` to the limiter after RCPT TO.
  - **Files:** `crates/rusmes-core/src/rate_limit.rs` (new enum `RateLimitKey { Ip(IpAddr), Sender(MailAddress), IpAndSender(IpAddr, MailAddress) }`; extend `RateLimiter` to key by all three variants).
  - **Tests:** 6 messages from `spammer@x.com` over 1 s with limit 5 → 6th rejected.
  - **Risk:** SMTP session call pattern changes (limiter now sees envelope post-RCPT TO); subagent updates only the call site, not the session structure.
- [x] Persistent rate limit state (survive restarts) (planned 2026-05-03)
  - **Goal:** `RateLimiter` snapshots bucket state to `<runtime_dir>/ratelimit.json` every N seconds via a tokio task; re-loads on startup. Uses `serde_json` (human-debuggable; not a hot path). Per COOLJAPAN policy: no `bincode`; JSON preferred here.
  - **Files:** `crates/rusmes-core/src/rate_limit.rs` (snapshot task + load-on-init).
  - **Tests:** Snapshot, drop limiter, reload, assert bucket state preserved.
  - **Risk:** File I/O on the rate-limit hot path is avoided by snapshotting on a background interval, not on every request.

## Proposed follow-ups

- **Async mailet loading from shared libraries (plugin system / WASM)** — A full WASM mailet runtime is a multi-week project with significant design scope (sandboxing, ABI stability, host function surface); deferred to 0.3.0 where it can be a first-class branch effort.
