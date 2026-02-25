# rusmes-server

Main server binary for RusMES. Composes all crates into a running mail server: loads configuration, initializes storage, builds the mailet processing pipeline, and starts protocol servers.

## Status

All four protocol servers (SMTP, IMAP, POP3, JMAP) start in parallel. File-based
authentication is fully integrated. LDAP/SQL/OAuth2 auth backends require
additional wiring to replace the DummyAuthBackend fallback.

## Binary

```bash
cargo build --bin rusmes-server
```

## Usage

```bash
# With configuration file
rusmes-server rusmes.toml

# With default configuration (localhost:2525)
rusmes-server
```

## Startup Sequence

1. Initialize tracing/logging
2. Load configuration from TOML file (or use defaults)
3. Initialize storage backend (filesystem or PostgreSQL)
4. Create metrics collector
5. Build mailet processor router from config:
   - Parse processor state names
   - Create matchers and mailets via factory
   - Assemble processing chains
6. Initialize authentication backend
7. Start SMTP server on configured address

## Configuration

Reads `rusmes.toml` (or path from first CLI argument). Falls back to defaults:
- Domain: `localhost`
- SMTP: `0.0.0.0:2525`
- Storage: filesystem at `/tmp/rusmes`
- Processors: single "root" processor with `LocalDelivery`

## Dependencies

Depends on nearly all workspace crates:
- `rusmes-proto` - core types
- `rusmes-core` - mailet engine, factory, processor router
- `rusmes-storage` - storage backend
- `rusmes-smtp` - SMTP server
- `rusmes-imap` - IMAP server (declared, not yet started)
- `rusmes-jmap` - JMAP server (declared, not yet started)
- `rusmes-config` - configuration loading
- `rusmes-metrics` - metrics collector
- `rusmes-auth` - authentication backend
- `tokio` - async runtime
- `tracing` / `tracing-subscriber` - logging
