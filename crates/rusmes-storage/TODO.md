# rusmes-storage TODO

## Implemented ✅
### Abstraction Layer
- [x] `MailboxStore`, `MessageStore`, `MetadataStore`, `QuotaStore` traits
- [x] `StorageEvent` types for notifications
- [x] MODSEQ tracking for CONDSTORE/QRESYNC (463 lines)
- [x] Storage metrics — Prometheus-compatible (1,000 lines)

### Filesystem Backend (1,582 lines)
- [x] Maildir format with proper flags encoding (`:2,DFPRST`)
- [x] Atomic message delivery (write to `tmp/`, rename to `new/`)
- [x] Quota enforcement
- [x] Mailbox subscriptions (IMAP LSUB)
- [x] Message expunge (permanent deletion)
- [x] `get_message_thread_id()` — returns thread ID for RFC 5256 threading (thread_ops.rs / threading.rs)

### PostgreSQL Backend (2,592 lines)
- [x] All `MailboxStore`, `MessageStore`, `MetadataStore` methods
- [x] Connection pool (sqlx)
- [x] Full-text search index on message body/headers
- [x] Quota enforcement
- [x] MODSEQ tracking

### AmateRS Backend (1,390 lines)
- [-] **Mock implementation** — no real distributed system client
- [x] All trait methods implemented (with placeholder values)
- [x] `AmatersConfig::from_url()` — URL parser for AmateRS connection strings

## Remaining
### Filesystem
- [x] Directory locking for concurrent access safety (landed 2026-05-05)
  - **Goal.** Wrap maildir per-folder mutating operations (deliver, expunge, copy) in `fs2::FileExt::try_lock_exclusive()` on a hidden lockfile (`.rusmes.lock`) inside the folder. On contention, retry with backoff (50 ms, 100 ms, 250 ms, 500 ms, 1 s); abort with `StorageError::ConcurrencyConflict` after 2 s total.
  - **Implementation.** `locking.rs` provides `acquire_dir_lock()`. Wired into `append_message` (lock hoisted above fs2 call; in-process per-MailboxId `TokioMutex` gates entry first), `set_flags`, and `delete_messages` (lock acquired once per mailbox dir, not per file). `copy_messages` is transitively covered via `append_message`. `FilesystemMessageStore::per_mailbox_mutex()` helper lazily creates and returns the in-process mutex for any MailboxId, preventing all N in-process tasks from racing against the same `fs2` lockfile simultaneously.
  - **Tests.** `test_concurrent_deliver_no_duplicate_uids` — 16 **truly concurrent** (no staggering) deliveries; all 16 UIDs are unique. The in-process Tokio mutex serialises the tasks before they touch the filesystem lock.

### PostgreSQL
- [x] Database migration tooling (landed 2026-05-05)
  - **Goal.** Adopt `sqlx::migrate!`. Place migrations under `crates/rusmes-storage/migrations/` with `0001_initial.sql`. New `SqliteBackend` uses `sqlx::migrate!` automatically. PostgreSQL continues using hand-rolled idempotent `init_schema()` for backwards compatibility.
  - **Files.** `src/backends/sqlite.rs` (new), `migrations/0001_initial.sql` (new), `src/backends/mod.rs` (updated).
  - **Tests.** `test_migrations_sqlite_creates_tables` — `SqliteBackend::new(tempfile)` → all 5 tables exist.
- [x] Vacuum/maintenance scheduling (landed 2026-05-05)
  - **Goal.** Background `tokio::spawn` task started by `PostgresBackend::with_config_and_vacuum`; runs `VACUUM (ANALYZE)` once per configurable interval (default 24 h). Cancellable via `watch::Sender<bool>` shutdown channel on the backend.
  - **Files.** `src/backends/postgres.rs` — added `shutdown_tx: watch::Sender<bool>` field, `shutdown()` method, `with_config_and_vacuum()` constructor.
  - **Tests.** `test_vacuum_loop_shutdown_pattern` (renamed from `test_vacuum_task_shutdown`) — tests the `tokio::select!` / `watch::channel` shutdown machinery in isolation (does not require a live PostgreSQL instance). A full end-to-end test of `PostgresBackend::with_config_and_vacuum` + `shutdown()` would require `DATABASE_URL` and is left under `#[cfg(feature = "postgres-tests")]`.

### AmateRS
- [x] Real distributed system client integration (landed 2026-05-05)
  - **Goal:** Replace in-memory HashMap mock `AmatersClient` with real `amaters-sdk-rust v0.2.0`. Feature-gate as `amaters-backend`.
  - **Design:** `AmatersClient` is now an enum with `Mock` (HashMap) and `Real` (SDK gRPC) variants. `AmatersBackend::new` keeps mock path; `AmatersBackend::connect_real` (cfg-gated) uses the SDK. `list_prefix` maps to SDK `range` via `prefix_upper_bound` helper. SDK has C transitive deps (aws-lc-rs, ring); kept opt-in. Mock path (and all existing tests) unchanged.
  - **Files:** `Cargo.toml` (workspace dep), `crates/rusmes-storage/Cargo.toml` (feature + dep), `crates/rusmes-storage/src/backends/amaters.rs` (enum AmatersClient)
  - **Tests:** `test_amaters_set_get_roundtrip`, `test_amaters_delete`, `test_amaters_list_prefix` (all `#[ignore]` — require `AMATERS_TEST_ENDPOINT` env var)
- [x] **AmateRS Real-path correctness — body serialization, UID allocation, counter writes** (landed 2026-05-05)
  - **Goal.** Replace three silent-corruption stubs in the Real-path branches of `AmatersMessageStore` and `AmatersMetadataStore`: (a) `MessageBlob.body` is stored/retrieved as canonical RFC 5322 on-wire bytes instead of `vec![]`; (b) per-mailbox UIDs are allocated via a stored `nextuid` counter (with in-process Mutex fallback if SDK lacks CAS); (c) all `MetadataStore` counter writes (`nextuid`, `uidvalidity`, `messages_count`, etc.) persist to amaters keys. Mock-path behavior unchanged.
  - **Design.** Files post-split (plan block #1 prerequisite): `amaters/messages.rs` (items a+b), `amaters/metadata.rs` (item c), `amaters/client.rs` (CAS wrapper if SDK exposes it). UID counter key format: `meta:nextuid:{account}:{mailbox}` as `u32.to_be_bytes()`. Write UID FIRST (claim before storing body) to avoid reuse on body-write failure. Large message (>SDK max) → `StorageError::MessageTooLargeForBackend`.
  - **Prerequisites.** amaters.rs split (plan block #1) must land first.
  - **Concurrency fix (2026-05-05).** The per-mailbox `Arc<Mutex<u32>>` guard is now held for the *entire* `append_message` body (UID reservation → blob write → metadata write → counter RMW), not just the UID allocation step. This eliminates the counter race where two concurrent appends could both observe `exists=N` and write `exists=N+1`. `delete_messages` acquires the same mutex before each per-mailbox counter decrement. Helper functions refactored: `get_or_create_mailbox_mutex` (map-only lookup) + `sync_and_advance_uid` (called under guard, no re-entrant lock).
  - **Tests.** `test_amaters_real_message_body_roundtrip`, `test_amaters_real_uid_monotonic`, `test_amaters_real_uid_concurrent` (also checks `counters.exists == 16`), `test_amaters_real_uidvalidity_persistence`, `test_amaters_real_messages_count` — all `#[ignore]` unless `AMATERS_TEST_ENDPOINT` is set.
  - **Risk.** Counter write atomicity: write UID first. CAS availability: subagent reads SDK source and chooses strategy A (CAS) or B (in-process Mutex) accordingly.
- [x] **Replication factor and consistency level configuration** (landed 2026-05-05) — Path B2: emit `tracing::warn!` for config fields ignored by amaters-sdk-rust v0.2.0
  - **Goal.** When `AmatersConfig.replication_factor` or `consistency_level` differ from defaults, emit a clear `tracing::warn!` in `new_real` so operators know their config is accepted for forward compat but has no effect in SDK v0.2.0.
  - **Design.** `AmatersConfig::DEFAULT_REPLICATION_FACTOR: usize = 3`. After SDK client construction in `new_real`, compare configured values against defaults and emit structured warn events targeting `rusmes::storage::amaters`. `ConsistencyLevel` gained `#[derive(Default)]` with `#[default]` on `Quorum`. Rustdoc on `AmatersConfig` notes forward-compat status. Files: `amaters/client.rs` (warn blocks), `amaters/config.rs` (const + derive + rustdoc).
  - **Tests.** `test_amaters_config_default_replication_factor_const`, `test_consistency_level_default_is_quorum`, `test_amaters_config_default_consistency_levels`. (Warn-emission tests omitted: `tracing_test` not in workspace.)
- [x] **Failover and retry logic** (landed 2026-05-05) — Path A: thread retry fields through `new_real` into SDK `RetryConfig`. Multi-endpoint failover deferred (requires upstream amaters-sdk-rust PR).
  - **Goal.** New `AmatersConfig` fields `initial_backoff_ms: u64` (default 100) and `max_backoff_ms: u64` (default 5 000) join the existing `max_retries: usize` field; all three are threaded through `new_real` into `SdkRetryConfig`. Mock path unaffected.
  - **Design.** In `new_real`, build `SdkRetryConfig { max_retries, initial_backoff: Duration::from_millis, max_backoff: Duration::from_millis, backoff_multiplier: 2.0, jitter: true }` and apply via `SdkClientConfig::with_retry_config(...)`. `AmatersConfig::from_url` parses `?max_retries=N&initial_backoff_ms=M&max_backoff_ms=K` from the URL query string. Files: `amaters/client.rs` (SDK wiring), `amaters/config.rs` (fields + from_url).
  - **Tests.** `test_amaters_config_retry_defaults`, `test_amaters_config_from_url_with_retry_params`, `test_amaters_config_from_url_all_retry_params`, `test_amaters_config_from_url_invalid_retry_param_errors`, `test_amaters_config_retry_field_assignment`.
- [x] **AmateRS initial-connect endpoint cycling** (landed 2026-05-06)
  - **Goal.** `AmatersClient::new_real` currently uses only `cluster_endpoints.first()`. Add sequential cycling through all endpoints in the Vec so that the server bootstraps successfully as long as any endpoint is reachable.
  - **Design.** Replace the single-endpoint block at `client.rs:102-105` with a `for endpoint in &config.cluster_endpoints` loop; on connect success break; on failure emit `tracing::warn!` and try the next. Return `Err` only after exhausting all endpoints. Extract `fn ensure_scheme(endpoint: &str) -> String` for the existing scheme-prepend logic. Per-op runtime failover deferred (requires upstream amaters-sdk-rust PR).
  - **Files.** `src/backends/amaters/client.rs`.
  - **Tests.** `test_amaters_initial_connect_first_endpoint_works`, `test_amaters_initial_connect_falls_back_to_second`, `test_amaters_initial_connect_all_fail` — all `#[ignore]` unless `AMATERS_TEST_ENDPOINT` is set.

### General
- [x] Storage backend factory from configuration enum (landed 2026-05-05)
  - **Goal.** `pub async fn build_storage(kind: &BackendKind) -> Result<Arc<dyn StorageBackend>>`. Match on `Filesystem | Sqlite | Postgres | Amaters`; each arm constructs and initializes the concrete backend (migrations run automatically for SQL backends). rusmes-server bootstrap calls this exactly once.
  - **Files.** `src/lib.rs` — `BackendKind` now includes `Sqlite` variant; factory covers all four arms.
  - **Tests.** `test_build_storage_filesystem` — round-trips a single message through the factory.
- [x] Backup/restore API (landed 2026-05-05)
  - **Goal.** `pub async fn backup(backend: &dyn StorageBackend, dest: &Path)` and `pub async fn restore(backend: &dyn StorageBackend, src: &Path)` in `src/lib.rs` (re-exported from `src/backup.rs`). Uses OxiARC ZIP exclusively.
  - **Files.** `src/backup.rs`, `src/backends/filesystem/backup.rs` (OxiARC ZIP implementation).
  - **Tests.** `test_backup_restore_roundtrip_full` — 5 messages, backup → restore → count matches. `test_backup_restore_roundtrip` in `filesystem/backup.rs` — file-level raw roundtrip.
- [x] Compaction/cleanup for deleted messages (landed 2026-05-05)
  - **Goal.** `pub async fn compact_expunged(backend: &dyn StorageBackend, older_than: Duration)`. For filesystem: walks `.Trash` and removes files mtime older than threshold. For postgres: `DELETE FROM messages WHERE expunged_at < NOW() - $1`. Exposed via storage trait method `compact(...)`.
  - **Design.** Also add `MessageStored` event hook (NEW — owned by Cluster 3, consumed by Cluster 9 search incremental indexing): add a `tokio::sync::broadcast::Sender<StorageEvent>` to the `StorageBackend` trait via `fn event_stream(&self) -> broadcast::Receiver<StorageEvent>` (default impl returns an empty receiver). `enum StorageEvent { MessageStored { account: String, mailbox: String, uid: u32, message_id: MessageId }, MessageExpunged { account: String, mailbox: String, uid: u32 } }`. Each backend's `deliver()` / `append()` / `expunge()` paths fire the event after the storage write commits. Cluster 9 subscribes to drive incremental indexing.
  - **Files.** `crates/rusmes-storage/src/lib.rs` — add `compact_expunged`, `StorageEvent` enum, trait `event_stream()` default; re-export.
  - **Tests.** Deliver+expunge, advance time (use `tokio::time::pause`), call `compact_expunged`, assert removal. Event hook: subscribe to `event_stream()`, deliver one message, assert `MessageStored` arrives within 100 ms; expunge it, assert `MessageExpunged` follows.
  - **Risk.** Broadcast channel capacity: start at 256; if the subscriber lags, the channel drops oldest events — that's acceptable for the search use case (full reindex catches up).

## Proposed follow-ups

- [x] Replication factor and consistency level configuration — landed 2026-05-05 (Path B2: warn on non-default values; SDK v0.2.0 does not expose connect-time knobs)
- [x] Failover and retry logic — landed 2026-05-05 (Path A: retry fields wired into SDK RetryConfig; multi-endpoint failover deferred — needs upstream amaters-sdk-rust PR)
