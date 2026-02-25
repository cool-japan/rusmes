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
- [-] **Auth backend integration**: LDAP/SQL/OAuth2 all fall back to `DummyAuthBackend` — only file-based works
- [ ] PostgreSQL backend initialization from config
- [ ] AmateRS backend initialization from config

### Important
- [ ] Run storage migrations on startup
- [ ] `--check-config` flag (validate and exit, without starting)
- [ ] `-c` / `--config` flag (instead of positional argument)

### Security
- [ ] Drop privileges after binding ports (setuid)
- [ ] Chroot support
- [ ] PID file creation for process management