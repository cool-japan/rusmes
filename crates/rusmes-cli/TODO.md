# rusmes-cli TODO

## Implemented ✅
### Server Commands
- [x] `init` — generate default `rusmes.toml`, create data directories
- [x] `start` — start the server (delegates to `rusmes-server`)
- [x] `stop` — send shutdown signal
- [x] `check-config` — validate configuration file without starting
- [-] `status` — partially placeholder (some fields hardcoded)

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
- [-] `migrate` command — storage migration between backends (1,217 lines, partially placeholder)

### Metrics
- [x] `metrics` — fetch and display from running server

## Remaining
- [ ] `mailbox repair` — check and fix mailbox consistency
- [ ] Colored terminal output
- [ ] JSON output mode (`--json`)
- [ ] Tab completion generation (clap_complete)
- [ ] Man page generation
- [ ] `--watch` flag for continuous metrics display