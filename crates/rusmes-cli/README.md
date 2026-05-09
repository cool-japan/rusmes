# rusmes-cli

Command-line management tool for RusMES. Provides server initialization, user management, mailbox management, queue operations, migration, backup/restore, and live metrics viewing.

## Status

Alpha. All core commands are implemented with real storage and server integration. Placeholder output has been replaced with live data from the storage backend and the Prometheus `/metrics` endpoint.

## Binary

```bash
cargo build --bin rusmes
```

## Commands

```
rusmes <COMMAND>

Commands:
  init          Initialize a new RusMES installation
  start         Start the RusMES server
  stop          Stop the RusMES server
  check-config  Validate configuration file without starting
  status        Show live server status (PID, uptime, active connections, message counts)
  user          User management
  mailbox       Mailbox management
  queue         Queue management
  backup        Back up mailboxes and configuration
  restore       Restore from a backup archive
  migrate       Migrate storage between backends (filesystem ↔ sqlite ↔ postgres ↔ amaters)
  metrics       Show live server metrics
```

### Server Management
```bash
rusmes init --domain example.com    # Create config and directories
rusmes start -c rusmes.toml         # Start the server
rusmes stop                         # Stop the server
rusmes check-config -c rusmes.toml  # Validate config without starting
rusmes status                       # Show PID, uptime, active connections fetched from /metrics
```

### User Management
```bash
rusmes user add user@example.com --password secret
rusmes user list                      # Enumerates users via backend.list_all_users()
rusmes user delete user@example.com
rusmes user passwd user@example.com   # Change password
```

### Mailbox Management
```bash
rusmes mailbox list user@example.com
rusmes mailbox create user@example.com --name Archive
rusmes mailbox delete user@example.com --name Archive
rusmes mailbox rename user@example.com --from Archive --to Old
rusmes mailbox repair user@example.com            # Validate index vs on-disk state
rusmes mailbox repair user@example.com --vacuum   # Also calls compact_expunged() and reports count
```

### Queue Management
```bash
rusmes queue list                    # List queued messages
rusmes queue flush                   # Force delivery attempt
rusmes queue inspect <message-id>    # Show message details
rusmes queue delete <message-id>     # Remove message from queue
rusmes queue retry <message-id>      # Retry delivery immediately
```

### Backup & Migration
```bash
rusmes backup --output backup.tar.gz
rusmes restore --input backup.tar.gz
rusmes migrate --from filesystem --to amaters --data-dir /var/mail
```

### Metrics
```bash
rusmes metrics   # Fetch and display live Prometheus metrics from running server
```

## Key Implementation Notes

- `status` fetches active connections per protocol from the `/metrics` endpoint (port 9090) and falls back gracefully when the server is offline.
- `mailbox repair --vacuum` calls `compact_expunged()` on the storage backend and prints the number of expunged entries reclaimed.
- `migrate` supports the AmateRS backend as both source and destination, in addition to filesystem, sqlite, and postgres.
- `user list` uses `backend.list_all_users().await` rather than a static stub.

## Dependencies
- `rusmes-proto` - mail types
- `rusmes-storage` - storage backends (filesystem, sqlite, postgres, amaters)
- `rusmes-config` - configuration loading
- `rusmes-metrics` - metrics endpoint client
- `clap` - argument parsing
- `tokio` - async runtime

## Tests

```bash
cargo nextest run -p rusmes-cli --all-features   # 121 tests
```
