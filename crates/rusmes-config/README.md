# rusmes-config

Configuration management for RusMES. Loads and validates TOML and YAML configuration files, providing strongly-typed config structs to all server components.

## Status

Complete. Supports loading `ServerConfig` from both TOML and YAML files with all major configuration sections.

## Configuration Structs

| Struct | Description |
|--------|-------------|
| `ServerConfig` | Top-level: domain, postmaster, sub-configs |
| `SmtpServerConfig` | SMTP host, port, TLS, auth, message size |
| `ImapServerConfig` | IMAP host, port, TLS |
| `JmapServerConfig` | JMAP host, port, base URL |
| `StorageConfig` | Backend selection (filesystem / postgres / amaters) |
| `ProcessorConfig` | Mailet processor chain definition |
| `MailetConfig` | Individual mailet with matcher, name, and params |

## Storage Backend Variants

```rust
pub enum StorageConfig {
    Filesystem { path: String },
    Postgres { connection_string: String },
    AmateRS { endpoints: Vec<String>, replication_factor: usize },
}
```

## Size Parser

Parses human-readable sizes: `"50MB"`, `"1GB"`, `"1024KB"`, `"1024"` (bytes).

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

See `examples/` directory for minimal, full reference, and production configurations in both TOML and YAML formats:
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
cargo test -p rusmes-config   # 13 tests
```

Tests cover: size string parsing, duration string parsing, TOML configuration deserialization, YAML configuration deserialization, and TOML/YAML equivalence.
