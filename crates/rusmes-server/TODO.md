# rusmes-server TODO

## Implemented ✅
### Protocol Servers
- [x] Start SMTP, IMAP, POP3, JMAP servers in parallel
- [x] Graceful shutdown on SIGTERM/SIGINT
- [x] `tokio::select!` for signal handling
- [x] Hot-reload on SIGHUP
- [x] Connection limiter (per-IP + global, auto-reaper, 745 lines)
- [x] Structured logging with session UUID (558 lines)

### Configuration
- [x] Load config from TOML/YAML
- [x] Processor router construction from config
- [x] Startup banner with version info

### Observability
- [x] Metrics HTTP endpoint startup
- [x] Health check endpoint

## Remaining
### Critical
- [x] **Auth backend integration**: LDAP/SQL/OAuth2 all fall back to `DummyAuthBackend` — only file-based works (landed 2026-05-03)
  - **Goal.** Remove `DummyAuthBackend` construction in `main`; call `AuthBackendKind::from(cfg.auth).build().await?` (from rusmes-auth Cluster 1A) to obtain a real `Arc<dyn AuthBackend>` and clone it into every protocol server constructor (SMTP, IMAP, JMAP, POP3).
  - **Files.** `crates/rusmes-server/src/main.rs` (or `bootstrap.rs`) — replace dummy construction; `crates/rusmes-server/src/run.rs` (wherever protocol servers spawn) — pass `Arc<dyn AuthBackend>` into each constructor.
  - **Prerequisites.** Cluster 1A (`AuthBackendKind::build()` + `AuthBackend` trait); Cluster 3 (`build_storage` factory).
  - **Tests.** Integration `tests/bootstrap_auth_variants.rs`: start server with each `[auth]` variant (file via tempdir; SQL via SQLite tempfile; LDAP gated behind feature; OAuth2 via mockito), issue a JMAP `Session` request, confirm the principal binds.
  - **Risk.** DummyAuthBackend removal is a compile-time forcing function — any missed call site becomes a build error, which is a safety net. Wire all four protocol servers before removing the dummy to keep compilation green throughout.

- [x] **PostgreSQL backend initialization from config** (landed 2026-05-03)
  - **Goal.** Server bootstrap calls Cluster 3's `build_storage(&cfg.storage).await?` for the Postgres variant, which constructs a `PgPool` from the config DSN and runs `sqlx::migrate!()` against it. No direct `sqlx` calls remain in `rusmes-server/main.rs`.
  - **Files.** `crates/rusmes-server/src/main.rs` (or `bootstrap.rs`) — replace any inline pool construction with `build_storage`; `crates/rusmes-config/src/lib.rs` — verify `[storage]` schema includes DSN and backend-kind fields (no changes expected; add if missing).
  - **Prerequisites.** Cluster 3 (`build_storage` factory + `sqlx::migrate!` runner).
  - **Tests.** Part of `tests/bootstrap_auth_variants.rs` above: SQL via SQLite tempfile verifies the factory path end-to-end. Postgres path gated behind `#[cfg(feature = "postgres-integration")]` or uses a mock pool.
  - **Risk.** Migration failures at startup must surface a clear error (not a panic). Ensure `build_storage` propagates `sqlx::Error` with a descriptive context.

- [x] AmateRS backend initialization from config — `amaters-sdk-rust v0.2.0` wired in rusmes-storage (landed 2026-05-05); `AmatersBackend::connect_real` available via `amaters-backend` feature

### Important
- [x] **Run storage migrations on startup** (landed 2026-05-03)
  - **Goal.** On server boot, `build_storage` (Cluster 3) internally calls `sqlx::migrate!().run(&pool)` for each SQL backend (SQLite and Postgres). The server does not need to call any migration API explicitly — it falls out of the factory call above. This item tracks that the wiring is exercised and verified from the server's perspective.
  - **Files.** `crates/rusmes-server/src/main.rs` (or `bootstrap.rs`) — no dedicated migration call; ensured by `build_storage`. `crates/rusmes-storage/migrations/` — migration files must exist (Cluster 3 owns their content).
  - **Prerequisites.** Cluster 3 (`build_storage` with embedded `sqlx::migrate!`).
  - **Tests.** Spin up an empty SQLite tempfile, start the server via `bootstrap.rs` entry point, assert `messages` table exists after startup.
  - **Risk.** If Cluster 3 lands with migrations not embedded, server startup will fail silently at query time rather than at boot. Subagent must assert the table exists in the integration test to catch this early.

- [x] **`--check-config` flag** (validate and exit, without starting) (landed 2026-05-03)
  - **Goal.** New CLI flag `--check-config`: load the config file, validate all fields (including `[auth]` backend reachability and `[storage]` DSN parse), print diagnostics to stderr, then exit 0 on success or 1 on any error. No server sockets are opened.
  - **Files.** `crates/rusmes-server/src/main.rs` (or `cli.rs`) — clap derive adds `#[arg(long = "check-config")] check_config: bool`; bootstrap path branches before `tokio::spawn` calls.
  - **Prerequisites.** Cluster 1A (`AuthBackendKind` for config validation); Cluster 3 (`StorageConfig` validation path).
  - **Tests.** CLI: `--check-config valid.toml` exits 0; `--check-config bad.toml` (missing required field) exits 1 and prints a human-readable error.
  - **Risk.** "Validate backend reachability" without actually starting the backend (e.g., no DB connection on `--check-config`) is an intentional scope limit — config-schema validation only, not network-level health.

- [x] **`-c` / `--config` flag** (instead of positional argument) (landed 2026-05-03)
  - **Goal.** CLI gains `-c/--config <path>` as a named flag. The existing positional argument is preserved for one release as a deprecated fallback (emits a warning on stderr). Clap derive: `#[arg(short = 'c', long = "config")] config: Option<PathBuf>`.
  - **Files.** `crates/rusmes-server/src/main.rs` (or `cli.rs`) — clap struct update; deprecation warning path.
  - **Prerequisites.** None within rusmes-server (pure CLI change).
  - **Tests.** CLI: `-c valid.toml` starts; positional `valid.toml` starts (with deprecation warning on stderr); missing config path exits with a clear error.
  - **Risk.** Technically a breaking change for any scripts passing the config positionally. The one-release back-compat fallback mitigates. Subagent must not remove the positional until the follow-up release.

### Security
- [x] **PID file creation for process management** (landed 2026-05-03)
  - **Goal.** On startup, write the process PID to `<runtime_dir>/rusmes.pid` via `tokio::fs::write`. Register a tokio shutdown handler (attached to `SIGTERM` and `ctrl_c()`) that removes the file. If a stale PID file exists from a crashed process, overwrite it (no startup failure).
  - **Files.** `crates/rusmes-server/src/main.rs` (or `bootstrap.rs`) — PID write + shutdown cleanup; `crates/rusmes-config/src/lib.rs` — verify `[server.runtime_dir]` is present (add if missing).
  - **Prerequisites.** None (pure tokio/fs — no Cluster 1A or 3 dependency).
  - **Tests.** CLI: PID file written on startup; PID file removed after clean SIGINT.
  - **Risk.** PID file path must not assume a chroot root (chroot support is deferred — see Proposed follow-ups). Use the `runtime_dir` from config, which remains an absolute path until chroot lands.

## Proposed follow-ups

- [x] **Drop privileges after binding ports (setuid)** (landed 2026-05-06)
  - **What landed.** New `src/privileges.rs` module with `PrivilegeDrop { chroot_dir, uid, gid }`, `apply()` (Linux: real syscalls; other platforms: `tracing::warn!` + `Ok(())`), and `resolve_uid` / `resolve_gid` helpers. Config fields `run_as_user`, `run_as_group`, `chroot` added to `ServerConfig`. `nix = "0.31.2"` added to workspace. Six unit tests pass on macOS/Linux. Privilege-drop wired in `main.rs` before first `tokio::spawn`.
  - **Known limitation (pre-bind refactor required).** The current architecture binds all sockets *inside* `tokio::spawn` closures, so privileged ports (<1024) will fail to bind after `apply()` runs. Operators must use ports ≥1024 or `CAP_NET_BIND_SERVICE` until the listener-pre-bind refactor lands. Tracked below.
  - **Deferred.** CLI `--run-as <uid>:<gid>` flag; `test_privilege_drop_full_e2e` (requires root); listener-pre-bind refactor.

- [x] **Chroot support** (landed 2026-05-06)
  - **What landed.** `PrivilegeDrop.chroot_dir` field + `chroot(dir); chdir("/")` in `apply()` on Linux. Config field `chroot = true/false`. Same bind-ordering caveat as setuid item above.
  - **Deferred.** Pre-staging of `/etc/resolv.conf` and NSS files under `runtime_dir`; `prctl(PR_SET_NO_NEW_PRIVS)` + seccomp filter as post-drop hardening.

- [ ] **Listener pre-bind refactor** (follow-up to privilege-drop)
  - **Goal.** Hoist all `TcpListener::bind` calls (SMTP, IMAP, POP3, JMAP, metrics) above the first `tokio::spawn` in `main.rs`, so that privilege drop runs *after* binding and *before* spawning. Required for privileged-port operation with `run_as_user` / `chroot`.
  - **Files.** `crates/rusmes-server/src/main.rs` — move socket setup before spawn; thread `TcpListener` handles into each protocol server constructor.
  - **Risk.** Each protocol server's constructor signature must accept a pre-bound `TcpListener` rather than a `String` bind address. May require minor changes in SMTP/IMAP/POP3/JMAP crates.
