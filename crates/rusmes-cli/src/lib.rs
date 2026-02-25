//! # rusmes-cli
//!
//! Administrative command-line interface for the **RusMES** Rust Mail Enterprise Server.
//!
//! This crate provides the `rusmes` binary, a comprehensive management tool for
//! interacting with a running RusMES instance over its REST API, as well as
//! performing offline operations (backup, restore, migration, config validation).
//!
//! ## Features
//!
//! - **User management** — add, list, delete, change passwords, adjust quotas, enable/disable accounts
//! - **Mailbox management** — create, delete, rename, repair, subscribe/unsubscribe mailboxes
//! - **Queue management** — list, inspect, flush, retry, purge queued messages
//! - **Backup** — full and incremental backups with optional AES-256-GCM encryption,
//!   gzip/zstd compression, and S3-compatible upload
//! - **Restore** — full, user-scoped, and point-in-time restore with optional decryption
//! - **Migration** — online migration between storage backends (filesystem ↔ PostgreSQL)
//! - **Config validation** — parse and validate `rusmes.toml` configuration files
//! - **Server status** — inspect process health and listening ports
//! - **Shell completions** — generate completions for bash, zsh, fish, etc.
//!
//! ## Usage
//!
//! ```text
//! rusmes --help
//! rusmes user add alice@example.com --password s3cr3t
//! rusmes backup full --output /var/backups/rusmes.tar.zst --compression zstd
//! rusmes restore restore --backup /var/backups/rusmes.tar.zst
//! rusmes migrate --from filesystem --to postgres
//! rusmes check-config --config /etc/rusmes/rusmes.toml
//! ```

pub mod client;
pub mod commands;
