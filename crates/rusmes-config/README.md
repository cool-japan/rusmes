# rusmes-config

Configuration management for RusMES. Loads and validates TOML and YAML configuration files,
providing strongly-typed config structs to all server components. Supports environment variable
overrides, hot-reload on SIGHUP, and structured validation with descriptive errors.

## Status

Complete. Supports loading `ServerConfig` from both TOML and YAML files with all major
configuration sections. Environment variable overrides, hot-reload, log rotation, and validation
are all production-ready.

## Configuration Structs

| Struct | Description |
|--------|-------------|
| `ServerConfig` | Top-level: domain, postmaster, sub-configs |
| `SmtpServerConfig` | SMTP host, port, TLS, auth, message size |
| `ImapServerConfig` | IMAP host, port, TLS |
| `JmapServerConfig` | JMAP host, port, base URL |
| `Pop3ServerConfig` | POP3 host, port, TLS |
| `AuthConfig` | Auth backend kind (file, LDAP, SQL, OAuth2), credentials |
| `QueueConfig` | Retry intervals, max attempts, bounce policy |
| `SecurityConfig` | TLS versions, cipher suites, DKIM, SPF, DMARC settings |
| `TracingConfig` | Trace level, exporter endpoint, sampling rate |
| `ConnectionLimitsConfig` | Per-IP and global connection caps, timeout, auto-reaper |
| `StorageConfig` | Backend selection (filesystem / postgres / amaters) |
| `ProcessorConfig` | Mailet processor chain definition |
| `MailetConfig` | Individual mailet with matcher, name, and params |
| `LogConfig` | Log level, format (JSON/Text), rotation policy |
| `MetricsConfig` | Prometheus endpoint host and port |

## Storage Backend Variants

```rust
pub enum StorageConfig {
    Filesystem { path: String },
    Postgres { connection_string: String },
    AmateRS { endpoints: Vec<String>, replication_factor: usize },
}
```

## Environment Variable Overrides

All major settings can be overridden at runtime via `RUSMES_*` environment variables (30+ supported).
Examples:

```bash
RUSMES_SMTP_PORT=2525
RUSMES_SMTP_HOST=0.0.0.0
RUSMES_IMAP_PORT=143
RUSMES_DOMAIN=mail.example.com
RUSMES_STORAGE_BACKEND=postgres
RUSMES_STORAGE_PATH=/var/mail
RUSMES_LOG_LEVEL=debug
RUSMES_METRICS_PORT=9090
```

Environment variables take precedence over file-based configuration values.

## Hot-Reload

The server reloads configuration on `SIGHUP` without restarting. Config structs are re-parsed
from the same file path and validated before replacing the live configuration. If validation
fails, the reload is aborted and the server continues with the previous configuration.

## Validation

`ServerConfig::from_file()` validates all fields on load and returns descriptive errors:
- Domain and postmaster address format
- Port ranges (1-65535)
- File paths (existence checks where applicable)
- Processor name uniqueness
- Auth backend required fields (e.g., LDAP URL, SQL DSN, OAuth2 client ID/secret)

## Size and Duration Parsers

```rust
// Size strings
"50MB"   -> 52_428_800 bytes
"1GB"    -> 1_073_741_824 bytes
"1024KB" -> 1_048_576 bytes
"1024"   -> 1024 bytes (bare integer = bytes)

// Duration strings
"60s"  -> 60 seconds
"30m"  -> 1800 seconds
"1h"   -> 3600 seconds
```

## Log Rotation

```toml
[log]
level = "info"
format = "json"          # "json" | "text"

[log.rotation]
policy = "daily"         # "daily" | "hourly" | "size"
max_size_bytes = 104857600  # used when policy = "size"
keep_files = 7
```

## Usage

```rust
use rusmes_config::ServerConfig;

// Load TOML configuration
let config = ServerConfig::from_file("rusmes.toml")?;
println!("Domain: {}", config.domain);
println!("SMTP port: {}", config.smtp.port);
println!("Max message size: {} bytes", config.smtp.max_message_size_bytes()?);

// Load YAML configuration
let config = ServerConfig::from_file("rusmes.yaml")?;
// ... same API
```

The format is auto-detected based on file extension:
- `.toml` files are parsed as TOML
- `.yaml` or `.yml` files are parsed as YAML

## Example Configuration

See `examples/` directory for minimal, full reference, and production configurations in both
TOML and YAML formats:
- `rusmes-minimal.toml` / `rusmes-minimal.yaml`
- `rusmes-full.toml` / `rusmes-full.yaml`
- `rusmes-production.toml`

## Dependencies
- `rusmes-proto` - `MailAddress` type
- `serde` - deserialization
- `toml` - TOML parsing
- `serde_yaml` - YAML parsing
- `anyhow` - error handling

## Tests

```bash
cargo nextest run -p rusmes-config --all-features
```

57 tests run: 57 passed, 0 skipped.

Tests cover: size string parsing, duration string parsing, log config (rotation policy,
level parsing, module-level filters), TOML configuration deserialization for all sections
(Auth, Queue, Security, Metrics, Logging, Rate Limits), YAML configuration deserialization,
TOML/YAML equivalence, and field validation (domain, email, port).
