# rusmes-server

Main server binary for RusMES. Composes all crates into a running mail server: loads configuration,
initializes storage, builds the mailet processing pipeline, starts protocol servers, and exposes a
Prometheus metrics endpoint.

## Status

All four protocol servers (SMTP, IMAP, POP3, JMAP) start in parallel. Auth backends — file-based,
LDAP, SQL, and OAuth2 — are fully wired (landed 2026-05-03). PostgreSQL and SQLite storage backends
initialize and run migrations on startup. PID file management, structured session logging,
connection limiting, hot-reload via SIGHUP, and privilege drop (chroot + setuid/setgid) are all in production.

## Binary

```bash
cargo build --bin rusmes-server
```

## Usage

```bash
# With named config flag (recommended)
rusmes-server -c rusmes.toml

# With long form
rusmes-server --config rusmes.toml

# Validate config without starting (exits 0/1)
rusmes-server --check-config -c rusmes.toml

# With default configuration (localhost:2525)
rusmes-server
```

## Startup Sequence

1. Parse CLI arguments (`-c/--config`, `--check-config`)
2. Initialize tracing/logging with structured session UUIDs
3. Load and validate configuration from TOML/YAML file (or use defaults)
4. Write PID file to `<runtime_dir>/rusmes.pid`
5. Initialize storage backend (filesystem, PostgreSQL, or AmateRS) and run migrations
6. Create Prometheus metrics collector and start metrics HTTP endpoint
7. Build mailet processor router from config:
   - Parse processor state names
   - Create matchers and mailets via factory
   - Assemble processing chains
8. Initialize authentication backend (file, LDAP, SQL, or OAuth2)
9. Start SMTP, IMAP, POP3, and JMAP servers concurrently via `tokio::select!`
10. Drop privileges via `PrivilegeDrop` (chroot + setuid/setgid, Linux-only; configured via `[server] run_as_user`, `run_as_group`, `chroot`)
11. Register SIGTERM/SIGINT/SIGHUP handlers for graceful shutdown and hot-reload

## Configuration

Reads `rusmes.toml` (or path from `-c`/`--config`). Falls back to defaults:
- Domain: `localhost`
- SMTP: `0.0.0.0:2525`
- Storage: filesystem at `/tmp/rusmes`
- Processors: single "root" processor with `LocalDelivery`

Use `--check-config` to validate the configuration file without starting any server sockets.

## Dependencies

Depends on nearly all workspace crates:
- `rusmes-proto` - core types
- `rusmes-core` - mailet engine, factory, processor router
- `rusmes-storage` - storage backend
- `rusmes-smtp` - SMTP server
- `rusmes-imap` - IMAP server
- `rusmes-jmap` - JMAP server
- `rusmes-pop3` - POP3 server
- `rusmes-config` - configuration loading
- `rusmes-metrics` - Prometheus metrics collector
- `rusmes-auth` - authentication backend (file, LDAP, SQL, OAuth2)
- `tokio` - async runtime
- `tracing` / `tracing-subscriber` - logging

## Tests

```bash
cargo nextest run -p rusmes-server --all-features
```

47 tests run: 47 passed, 0 skipped.

Coverage: bootstrap (auth backend wiring, storage kind mapping, PID file round-trip,
credential redaction, config validation), CLI flag parsing (`-c`, `--check-config`,
positional fallback with deprecation warning), structured session logging, and privilege drop.
