# rusmes-pop3 TODO

## Implemented ✅
### Core Commands (RFC 1939)
- [x] USER / PASS authentication
- [x] APOP authentication (MD5 challenge-response)
- [x] STAT, LIST, RETR, DELE, NOOP, RSET, QUIT
- [x] TOP (headers + N lines), UIDL (unique ID listing)

### Extensions
- [x] STLS (RFC 2595) — STARTTLS upgrade
- [x] CAPA (RFC 2449) — capability negotiation

### Server
- [x] TCP listener with session dispatch
- [x] Session state machine (Authorization → Transaction → Update)
- [x] Command parser + response builder
- [x] Integration with rusmes-storage

## Remaining
- [x] Maildrop locking (prevent concurrent POP3 sessions for same user) (landed 2026-05-03)
  - **Goal.** POP3 maildrop is exclusively locked during the Transaction state per RFC 1939 §3; a second concurrent session for the same user is rejected with `-ERR maildrop locked`.
  - **Design.** Implemented as `MaildropLockManager` (`crates/rusmes-pop3/src/maildrop_lock.rs`): per-user `Arc<tokio::sync::Mutex<()>>` map keyed by username, acquired non-blockingly via `try_lock_owned()`. The returned `MaildropGuard` is RAII (`OwnedMutexGuard<()>`) and is released on `QUIT`, on session drop, or on panic. Wired into `Pop3Session` via `transition_to_transaction()` which is called from `PASS`, `APOP`, and the SASL AUTH success path.
  - **Files.**
    - `crates/rusmes-pop3/src/maildrop_lock.rs` — new module: `MaildropLockManager` + `MaildropGuard`.
    - `crates/rusmes-pop3/src/session.rs` — `Pop3Session::transition_to_transaction` acquires lock; `release_maildrop_lock()` proactively drops it after Update state; PASS / APOP route through the helper.
    - `crates/rusmes-pop3/src/server.rs` — server holds a single shared `MaildropLockManager` per listener.
    - `crates/rusmes-pop3/src/lib.rs` — re-exports `MaildropLockManager` + `MaildropGuard`.

- [x] AUTH (RFC 1734) — SASL authentication for POP3 (landed 2026-05-03)
  - **Goal.** POP3 server supports the `AUTH <mechanism>` command (RFC 1734 + RFC 5034) in addition to the legacy USER/PASS path. On success, transitions directly to Transaction state with the authenticated principal (and acquires the maildrop lock).
  - **Design.** New `Pop3Command::Auth { mechanism, initial_response }` variant; parser added in `parser.rs`. New `Pop3Status::Continue` and `Pop3Response::cont(...)` for the `+ <base64>` continuation. `Pop3Session` delegates to `rusmes_auth::sasl::SaslServer::create_mechanism(...)` and drives the resulting `SaslMechanism` step-by-step. Multi-step exchanges park the in-flight mechanism in `pending_sasl: Option<Box<dyn SaslMechanism>>`; subsequent client lines are routed through `handle_sasl_continuation`. RFC 5034 `*` (client abort) and `=` (empty initial-response) tokens are honored. Mechanisms: PLAIN, LOGIN, CRAM-MD5, SCRAM-SHA-256 — exactly what `rusmes-auth::sasl` exposes.
  - **Files.**
    - `crates/rusmes-pop3/src/command.rs` — `Auth` variant + Display + tests.
    - `crates/rusmes-pop3/src/parser.rs` — `auth_command` + 6 parser tests.
    - `crates/rusmes-pop3/src/response.rs` — `Pop3Status::Continue`, `Pop3Response::cont(...)`, wire-format tests.
    - `crates/rusmes-pop3/src/session.rs` — `handle_auth`, `drive_pending_sasl`, `handle_sasl_continuation`; CAPA now advertises `SASL <mechanisms>`.
    - `crates/rusmes-pop3/Cargo.toml` — added `base64` workspace dep + `tempfile` dev-dep.
  - **Tests.**
    - Unit: 6 maildrop_lock + 5 response wire-format + 6 parser AUTH tests + 1 command Display.
    - Integration (`tests/sasl_and_locking.rs`): 7 tests — concurrent maildrop locking, lock release, PLAIN with IR, PLAIN two-step, PLAIN bad password, unknown mechanism, client abort.
  - **Notes.** SCRAM-SHA-256 round-trip would require Cluster 1A's `set_scram_credentials` migration tooling and is therefore not exercised end-to-end here; the SCRAM mechanism is wired and parsed, and will work as soon as the credential bundle is populated.

- [x] **POP3 metrics integration** — Connection guard (RAII) for active_connections wired (landed 2026-05-05)
  - `Pop3Session::new` and `Pop3Session::handle` carry `// TODO(metrics)` placeholders for `metrics::active_connections().with_label_values(&["pop3"]).inc()`/`.dec()`. Cluster 7 (rusmes-metrics) owns adding the `active_connections()` API; once it lands the placeholder lines should be flipped on.

## Proposed follow-ups

No additional items deferred from Cluster 5 — both pending items are captured above. See the global plan (`~/.claude/plans/cached-riding-torvalds.md`) for cross-crate follow-ups (e.g. per-protocol TLS metrics from Cluster 7, active-connections gauge integration).
