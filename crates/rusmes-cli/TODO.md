# rusmes-cli TODO

## Implemented ✅
### Server Commands
- [x] `init` — generate default `rusmes.toml`, create data directories
- [x] `start` — start the server (delegates to `rusmes-server`)
- [x] `stop` — send shutdown signal
- [x] `check-config` — validate configuration file without starting
- [x] `status` — fetches active connections from `/metrics` endpoint (port 9090); graceful fallback when server is offline (landed 2026-05-05)
    - **Goal:** rusmes CLI `status` feels like a production sysadmin tool with real live data
    - **Design:** read PID file (Cluster 1B) for uptime; query `/metrics` for connection counts; query storage backend for message counts; graceful fallback when server is not running; support `--json` flag for machine-readable output
    - **Files:** `crates/rusmes-cli/src/commands/status.rs`
    - **Tests:** `rusmes status --json` parses as `serde_json::Value`; with `NO_COLOR=1` output contains no ANSI escapes; offline fallback returns exit 0 with a clear "server not running" message
    - **Prerequisites:** Cluster 1B (PID file), Cluster 3 (storage counts)
    - **Risk:** status uptime and migrate backup/restore wiring get stub implementations in Wave A; follow-up edit in Wave B once Clusters 1B/3 land

### User Management
- [x] `user add` — create user with password hashing
- [x] `user list` — enumerate users from backend
- [x] `user delete` — remove user
- [x] `user passwd` — change password

### Mailbox Management
- [x] `mailbox list`, `mailbox create`, `mailbox delete`, `mailbox rename`

### Queue Management
- [x] `queue list`, `queue flush`, `queue inspect`, `queue delete`, `queue retry`

### Backup & Migration
- [x] `backup` command (1,099 lines)
- [x] `restore` command (943 lines)
- [x] `migrate` command — AmateRS backend wired as source/destination; full backend pairs (file↔sqlite↔postgres↔amaters); progress via `indicatif` (landed 2026-05-05)
    - **Goal:** complete storage migration between all backend pairs rather than a partial placeholder
    - **Design:** implement every combination of file↔sqlite↔postgres; use Cluster 3's `backup`/`restore` library API where the simplest path; progress reporting via `indicatif` (already in workspace deps); support `--json` flag for machine-readable status
    - **Files:** `crates/rusmes-cli/src/commands/migrate.rs`
    - **Tests:** `rusmes migrate --json` parses as `serde_json::Value`; smoke test for each backend pair round-trip in a tempdir
    - **Prerequisites:** Cluster 3 (backup/restore library API)
    - **Risk:** stub implementation (current 1,217 lines) lands in Wave A; real backend wiring follows in Wave B after Cluster 3 returns

### Metrics
- [x] `metrics` — fetch and display from running server

## Remaining
- [x] `mailbox repair` — validates index vs on-disk state; `--vacuum` calls `compact_expunged()` and reports count (landed 2026-05-05)
    - **Goal:** new subcommand connecting to the backend, walking every mailbox, validating on-disk state matches the metadata index, rebuilding missing index entries, and reporting orphans
    - **Design:** walk every mailbox via the storage backend; validate on-disk state vs metadata index; rebuild missing index entries; report orphans; reuse Cluster 3's `compact_expunged` for cleanup when `--vacuum` is passed
    - **Files:** `crates/rusmes-cli/src/commands/mailbox.rs`
    - **Tests:** deliver a message, manually corrupt the index, assert `mailbox repair` restores it
    - **Risk:** depends on having an in-process server fixture or using an `--offline` flag for unit tests; commands that depend on the running server must spin up a tempdir-backed fixture
- [x] Colored terminal output (landed 2026-05-05)
    - **Goal:** color-aware output on status, mailbox list, and error messages; respects `--color {auto,always,never}` and `NO_COLOR` env var
    - **Design:** use `colored = "3.1"` (latest); global `--color {auto,always,never}` flag with default `auto` (TTY detect via `std::io::IsTerminal`); `should_color(choice, is_tty) -> bool` extracted into `cli_def.rs` for testability; `apply_color` removed from `main.rs` in favour of the pure `should_color` helper
    - **Files:** `crates/rusmes-cli/src/cli_def.rs` (enum + `should_color`), `crates/rusmes-cli/src/main.rs` (wiring), `crates/rusmes-cli/src/lib.rs` (re-export)
    - **Tests:** `test_should_color_always`, `test_should_color_never`, `test_should_color_auto_tty`, `test_should_color_auto_no_tty`, `test_no_color_env_logic`, `color_disabled_when_no_color_env`
- [x] JSON output mode (`--json`) (landed 2026-05-05)
    - **Goal:** machine-readable JSON output for every structured-data subcommand
    - **Design:** global `--json` flag in `CliApp`; `ServerStatus` derives `Serialize`; `render(&dir, json)` returns JSON string when `json == true`; `OutputMode` implicitly represented by the `json: bool` parameter
    - **Files:** `crates/rusmes-cli/src/cli_def.rs`, `crates/rusmes-cli/src/commands/status.rs`
    - **Tests:** `json_output_parses_as_json` — verifies `serde_json::from_str::<serde_json::Value>` succeeds on status output
- [x] Tab completion generation (clap_complete) (landed 2026-05-05)
    - **Goal:** `rusmes completions <bash|zsh|fish|elvish|powershell>` subcommand for piping into shell init files
    - **Design:** `Commands::Completions { shell: Shell }` variant; `clap_complete::generate(shell, &mut cmd, name, &mut stdout)`
    - **Files:** `crates/rusmes-cli/src/commands/completions.rs`, `crates/rusmes-cli/src/cli_def.rs`, `crates/rusmes-cli/src/main.rs`
    - **Tests:** `completions_bash_contains_all_subcommands` — verifies non-empty output with every top-level subcommand name
- [x] Man page generation (landed 2026-05-05)
    - **Goal:** `rusmes man` subcommand emitting roff to stdout
    - **Design:** `Commands::Man` variant; `clap_mangen::Man::new(cmd).render(&mut stdout)`
    - **Files:** `crates/rusmes-cli/src/commands/man.rs`, `crates/rusmes-cli/src/cli_def.rs`, `crates/rusmes-cli/src/main.rs`
    - **Tests:** `man_page_produces_valid_roff` — verifies `.TH` macro is present in output
- [x] `--watch` flag for continuous status display (landed 2026-05-05)
    - **Goal:** live, in-place redrawing of `status` subcommand on a configurable interval; exits cleanly on SIGINT
    - **Design:** `run_watch(interval_ms, render_fn, cancel: Option<oneshot::Receiver<()>>)` with `tokio::select!` over sleep / ctrl_c / cancel channel; `run_watch_secs` convenience wrapper for production use; interval in seconds via `--watch <SECS>` on `status` command
    - **Files:** `crates/rusmes-cli/src/commands/watch.rs`, `crates/rusmes-cli/src/main.rs`
    - **Tests:** `test_watch_exits_on_signal` — pre-fires cancel channel, verifies loop exits without panic or error
    - **Note:** `rusmes metrics --watch` deferred — no `Metrics` command in the CLI yet; `--watch` implemented for `status`

## Proposed follow-ups

The following `[ ]` items from this crate are **not** covered by Cluster 6 of the current `/ultra` plan and are deferred to a future pass:

*(None at this time — all six Remaining items are covered by Cluster 6.)*

Additional items deferred by the broader plan (not originated in this TODO but relevant to this crate):
- `status` uptime field and `migrate` backend wiring require Clusters 1B and 3 (Wave B dependencies); stub implementations land in Wave A and are completed in Wave B.
