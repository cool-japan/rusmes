# rusmes-cli

Command-line management tool for RusMES. Provides server initialization, user management, mailbox management, queue operations, and metrics viewing.

## Status

CLI structure is complete with clap-based argument parsing. Commands currently print placeholder output; real storage/server integration is needed.

## Binary

```bash
cargo build --bin rusmes
```

## Commands

```
rusmes <COMMAND>

Commands:
  init      Initialize a new RusMES installation
  start     Start the RusMES server
  stop      Stop the RusMES server
  user      User management
  mailbox   Mailbox management
  queue     Queue management
  metrics   Show server metrics
```

### Server Management
```bash
rusmes init --domain example.com    # Create config and directories
rusmes start -c rusmes.toml         # Start the server
rusmes stop                         # Stop the server
```

### User Management
```bash
rusmes user add user@example.com --password secret
rusmes user list
rusmes user delete user@example.com
```

### Mailbox Management
```bash
rusmes mailbox list user@example.com
rusmes mailbox create user@example.com --name Archive
rusmes mailbox delete user@example.com --name Archive
```

### Queue Management
```bash
rusmes queue list                    # List queued messages
rusmes queue flush                   # Force delivery attempt
rusmes queue inspect <message-id>    # Show message details
```

### Metrics
```bash
rusmes metrics                       # Show server metrics
```

## Dependencies
- `rusmes-proto` - mail types
- `rusmes-config` - configuration loading
- `clap` - argument parsing
- `tokio` - async runtime
