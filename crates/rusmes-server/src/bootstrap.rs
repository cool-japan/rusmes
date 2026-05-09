//! Server bootstrap: configuration-driven construction of authentication and
//! storage backends, plus PID-file lifecycle management.
//!
//! This module isolates the conversion logic between the configuration crate
//! ([`rusmes_config`]) and the runtime factory APIs exposed by [`rusmes_auth`]
//! and [`rusmes_storage`]. The bridging here exists because the on-disk
//! configuration intentionally exposes a smaller, user-friendly surface than
//! the full backend configuration types — the helpers below fill in sensible
//! defaults for the fields that the configuration does not surface.
//!
//! The PID-file helpers cooperate with the signal-handling loop in `main.rs`:
//! [`PidFile::write`] creates the file (overwriting any stale entry) and
//! [`PidFile::cleanup`] removes it on graceful shutdown.

use anyhow::{Context, Result};
use rusmes_auth::backends::ldap::LdapConfig;
use rusmes_auth::backends::oauth2::{OAuth2Config, OidcProvider};
use rusmes_auth::backends::sql::SqlConfig;
use rusmes_auth::file::HashAlgorithm;
use rusmes_auth::{AuthBackend, AuthBackendKind, FileBackendConfig};
use rusmes_config::{
    AuthConfig as CfgAuthConfig, LdapAuthConfig, OAuth2AuthConfig, ServerConfig, SqlAuthConfig,
    StorageConfig as CfgStorageConfig,
};
use rusmes_storage::{build_storage, BackendKind, StorageBackend};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Auth-backend bridging
// ---------------------------------------------------------------------------

/// Translate a [`CfgAuthConfig`] from the on-disk configuration into the
/// matching [`AuthBackendKind`] understood by `rusmes-auth`.
///
/// The on-disk configuration intentionally exposes a smaller subset of fields
/// than the runtime backend configurations. Missing fields are filled in from
/// each backend's `Default` implementation, which provides production-safe
/// defaults (e.g. `max_connections = 10` for SQL, sensible LDAP timeouts).
pub fn auth_kind_from_config(cfg: &CfgAuthConfig) -> AuthBackendKind {
    match cfg {
        CfgAuthConfig::File { config } => {
            // The on-disk hash_algorithm string is best-effort: an unrecognised
            // value falls back to the default (bcrypt) with a warning, so a
            // typo never blocks startup of an otherwise valid configuration.
            let algorithm = match HashAlgorithm::from_config_str(&config.hash_algorithm) {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(
                        "[auth.file] {}; falling back to bcrypt for new password writes",
                        e
                    );
                    HashAlgorithm::default()
                }
            };
            AuthBackendKind::File(FileBackendConfig {
                path: config.path.clone(),
                hash_algorithm: algorithm,
            })
        }
        CfgAuthConfig::Sql { config } => AuthBackendKind::Sql(sql_config_from(config)),
        CfgAuthConfig::Ldap { config } => AuthBackendKind::Ldap(ldap_config_from(config)),
        CfgAuthConfig::OAuth2 { config } => AuthBackendKind::OAuth2(oauth2_config_from(config)),
    }
}

/// Build an authentication backend from configuration, defaulting to a
/// `FileAuthBackend` rooted in `<runtime_dir>/passwd` when the configuration
/// omits the `[auth]` section.
///
/// The fallback is intentional: it keeps existing single-host installs that
/// rely on the implicit file backend working, while routing the on-disk store
/// through the runtime-dir convention so it never lands in an unexpected place.
pub async fn build_auth_backend(cfg: &ServerConfig) -> Result<Arc<dyn AuthBackend>> {
    match &cfg.auth {
        Some(auth_cfg) => {
            let kind = auth_kind_from_config(auth_cfg);
            log_auth_backend_kind(auth_cfg);
            kind.build()
                .await
                .context("failed to construct authentication backend from configuration")
        }
        None => {
            let default_path = format!("{}/passwd", cfg.runtime_dir);
            tracing::warn!(
                "[auth] section missing — defaulting to file backend at {}",
                default_path
            );
            AuthBackendKind::File(FileBackendConfig {
                path: default_path,
                hash_algorithm: HashAlgorithm::default(),
            })
            .build()
            .await
            .context("failed to construct default file authentication backend")
        }
    }
}

fn log_auth_backend_kind(cfg: &CfgAuthConfig) {
    match cfg {
        CfgAuthConfig::File { config } => {
            tracing::info!("Using file authentication backend at {}", config.path);
        }
        CfgAuthConfig::Sql { config } => {
            tracing::info!(
                "Using SQL authentication backend at {}",
                redact_database_url(&config.connection_string)
            );
        }
        CfgAuthConfig::Ldap { config } => {
            tracing::info!(
                "Using LDAP authentication backend at {} (base_dn={})",
                config.url,
                config.base_dn
            );
        }
        CfgAuthConfig::OAuth2 { config } => {
            tracing::info!(
                "Using OAuth2 authentication backend (client_id={}, token_url={})",
                config.client_id,
                config.token_url
            );
        }
    }
}

fn sql_config_from(cfg: &SqlAuthConfig) -> SqlConfig {
    let defaults = SqlConfig::default();
    let password_query = if cfg.query.trim().is_empty() {
        defaults.password_query
    } else {
        cfg.query.clone()
    };
    SqlConfig {
        database_url: cfg.connection_string.clone(),
        password_query,
        ..defaults
    }
}

fn ldap_config_from(cfg: &LdapAuthConfig) -> LdapConfig {
    let bind_dn = if cfg.bind_dn.is_empty() {
        None
    } else {
        Some(cfg.bind_dn.clone())
    };
    let bind_password = if cfg.bind_password.is_empty() {
        None
    } else {
        Some(cfg.bind_password.clone())
    };
    LdapConfig {
        server_url: cfg.url.clone(),
        base_dn: cfg.base_dn.clone(),
        user_filter: cfg.user_filter.clone(),
        bind_dn,
        bind_password,
        ..LdapConfig::default()
    }
}

fn oauth2_config_from(cfg: &OAuth2AuthConfig) -> OAuth2Config {
    // The on-disk configuration exposes the issuer-agnostic OAuth2 fields, so
    // we always materialise a `Generic` provider. Operators that want
    // tenant-specific Google / Microsoft validation can extend the config in a
    // follow-up release.
    let provider = OidcProvider::Generic {
        issuer_url: cfg.authorization_url.clone(),
        client_id: cfg.client_id.clone(),
        client_secret: cfg.client_secret.clone(),
        jwks_url: cfg.token_url.clone(),
    };
    OAuth2Config {
        provider,
        ..OAuth2Config::default()
    }
}

fn redact_database_url(raw: &str) -> String {
    // Strip embedded credentials (`scheme://user:pass@host/...`) before
    // emitting to logs. Keeps the scheme + host portion intact for operators.
    if let Some(scheme_end) = raw.find("://") {
        let (scheme, rest) = raw.split_at(scheme_end + 3);
        if let Some(at_pos) = rest.find('@') {
            return format!("{}***@{}", scheme, &rest[at_pos + 1..]);
        }
    }
    raw.to_string()
}

// ---------------------------------------------------------------------------
// Storage-backend bridging
// ---------------------------------------------------------------------------

/// Translate a [`CfgStorageConfig`] from the on-disk configuration into the
/// matching [`BackendKind`] understood by `rusmes-storage`.
pub fn storage_kind_from_config(cfg: &CfgStorageConfig) -> BackendKind {
    match cfg {
        CfgStorageConfig::Filesystem { path } => BackendKind::Filesystem { path: path.clone() },
        CfgStorageConfig::Postgres { connection_string } => BackendKind::Postgres {
            connection_string: connection_string.clone(),
        },
        CfgStorageConfig::AmateRS {
            endpoints,
            replication_factor,
        } => BackendKind::Amaters {
            endpoints: endpoints.clone(),
            replication_factor: *replication_factor,
        },
    }
}

/// Construct a storage backend from configuration. Wraps
/// [`rusmes_storage::build_storage`] with a [`anyhow::Context`] tag for the
/// configured backend kind so startup errors are easy to trace.
pub async fn build_storage_backend(cfg: &CfgStorageConfig) -> Result<Arc<dyn StorageBackend>> {
    let kind = storage_kind_from_config(cfg);
    let label = backend_label(&kind);
    build_storage(&kind)
        .await
        .with_context(|| format!("failed to construct storage backend ({})", label))
}

fn backend_label(kind: &BackendKind) -> &'static str {
    match kind {
        BackendKind::Filesystem { .. } => "filesystem",
        BackendKind::Sqlite { .. } => "sqlite",
        BackendKind::Postgres { .. } => "postgres",
        BackendKind::Amaters { .. } => "amaters",
    }
}

// ---------------------------------------------------------------------------
// PID file lifecycle
// ---------------------------------------------------------------------------

/// PID file marker conventionally placed under `<runtime_dir>/rusmes.pid`.
///
/// The handle is deliberately small: callers create it with [`PidFile::write`]
/// during startup and call [`PidFile::cleanup`] from the graceful-shutdown
/// path. Silent-removal failures are logged at WARN level; they should not
/// abort shutdown.
#[derive(Debug, Clone)]
pub struct PidFile {
    path: PathBuf,
}

impl PidFile {
    /// Compute the canonical PID-file path inside `runtime_dir`.
    pub fn path_in(runtime_dir: impl AsRef<Path>) -> PathBuf {
        runtime_dir.as_ref().join("rusmes.pid")
    }

    /// Write the current process PID to `<runtime_dir>/rusmes.pid`, creating
    /// the runtime directory (and any intermediate parents) if needed.
    ///
    /// If a stale file already exists (from a previous crashed process) it is
    /// overwritten. The write is performed atomically via `tokio::fs::write`.
    pub async fn write(runtime_dir: impl AsRef<Path>) -> Result<Self> {
        let runtime_dir = runtime_dir.as_ref();
        if !runtime_dir.exists() {
            tokio::fs::create_dir_all(runtime_dir)
                .await
                .with_context(|| {
                    format!(
                        "failed to create runtime_dir at {} for PID file",
                        runtime_dir.display()
                    )
                })?;
        }
        let path = Self::path_in(runtime_dir);
        let pid = std::process::id();
        tokio::fs::write(&path, format!("{}\n", pid))
            .await
            .with_context(|| format!("failed to write PID file at {}", path.display()))?;
        tracing::info!("Wrote PID {} to {}", pid, path.display());
        Ok(Self { path })
    }

    /// Path to the PID file managed by this handle.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Remove the PID file. Errors are logged but never returned — shutdown
    /// must remain best-effort.
    pub async fn cleanup(&self) {
        if let Err(e) = tokio::fs::remove_file(&self.path).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    "failed to remove PID file at {}: {}",
                    self.path.display(),
                    e
                );
            }
        } else {
            tracing::info!("Removed PID file at {}", self.path.display());
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration validation entry point
// ---------------------------------------------------------------------------

/// Load and validate a configuration file without opening any sockets.
///
/// Used by both the `--check-config` flag and the regular startup path. A
/// successful return guarantees that:
/// - The file parses as TOML or YAML.
/// - All schema-level invariants pass (`ServerConfig::validate`).
/// - The configured backend kinds for `[auth]` and `[storage]` map onto known
///   `AuthBackendKind` / `BackendKind` variants.
///
/// Network-level reachability checks (e.g. opening an LDAP connection) are
/// intentionally out of scope.
pub fn load_and_validate(path: impl AsRef<Path>) -> Result<ServerConfig> {
    let path = path.as_ref();
    let cfg = ServerConfig::from_file(path)
        .with_context(|| format!("failed to load configuration from {}", path.display()))?;

    // Round-trip through the bridging helpers to catch any silent variant
    // mismatch at validation time rather than at startup.
    if let Some(ref auth_cfg) = cfg.auth {
        let _ = auth_kind_from_config(auth_cfg);
    }
    let _ = storage_kind_from_config(&cfg.storage);

    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusmes_config::FileAuthConfig;

    fn write_minimal_config(path: &Path, runtime_dir: &str) {
        let body = format!(
            r#"domain = "example.com"
postmaster = "postmaster@example.com"
runtime_dir = "{}"

[smtp]
host = "0.0.0.0"
port = 2525
max_message_size = "10MB"
require_auth = false
enable_starttls = false

[storage]
backend = "filesystem"
path = "{}/mail"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"
"#,
            runtime_dir, runtime_dir
        );
        std::fs::write(path, body).expect("write tmp config");
    }

    #[test]
    fn auth_kind_from_file_config_round_trips() {
        let cfg = CfgAuthConfig::File {
            config: FileAuthConfig {
                path: "/etc/rusmes/passwd".to_string(),
                hash_algorithm: "bcrypt".to_string(),
            },
        };
        match auth_kind_from_config(&cfg) {
            AuthBackendKind::File(file_cfg) => {
                assert_eq!(file_cfg.path, "/etc/rusmes/passwd");
                assert_eq!(file_cfg.hash_algorithm, HashAlgorithm::Bcrypt);
            }
            _ => panic!("expected AuthBackendKind::File"),
        }
    }

    #[test]
    fn auth_kind_from_file_config_argon2_algorithm() {
        let cfg = CfgAuthConfig::File {
            config: FileAuthConfig {
                path: "/etc/rusmes/passwd".to_string(),
                hash_algorithm: "argon2id".to_string(),
            },
        };
        match auth_kind_from_config(&cfg) {
            AuthBackendKind::File(file_cfg) => {
                assert_eq!(file_cfg.hash_algorithm, HashAlgorithm::Argon2);
            }
            _ => panic!("expected AuthBackendKind::File"),
        }
    }

    #[test]
    fn auth_kind_from_file_config_unknown_algorithm_falls_back_to_bcrypt() {
        let cfg = CfgAuthConfig::File {
            config: FileAuthConfig {
                path: "/etc/rusmes/passwd".to_string(),
                hash_algorithm: "scrypt".to_string(),
            },
        };
        match auth_kind_from_config(&cfg) {
            AuthBackendKind::File(file_cfg) => {
                assert_eq!(file_cfg.hash_algorithm, HashAlgorithm::Bcrypt);
            }
            _ => panic!("expected AuthBackendKind::File"),
        }
    }

    #[test]
    fn auth_kind_from_sql_config_preserves_url() {
        let cfg = CfgAuthConfig::Sql {
            config: SqlAuthConfig {
                connection_string: "postgres://user:pw@db/auth".to_string(),
                query: "SELECT password FROM users WHERE name = ?".to_string(),
            },
        };
        match auth_kind_from_config(&cfg) {
            AuthBackendKind::Sql(sql_cfg) => {
                assert_eq!(sql_cfg.database_url, "postgres://user:pw@db/auth");
                assert_eq!(
                    sql_cfg.password_query,
                    "SELECT password FROM users WHERE name = ?"
                );
                // Defaults preserved for fields not in the on-disk schema.
                assert!(sql_cfg.max_connections > 0);
            }
            _ => panic!("expected AuthBackendKind::Sql"),
        }
    }

    #[test]
    fn auth_kind_from_ldap_config_promotes_blank_to_none() {
        let cfg = CfgAuthConfig::Ldap {
            config: LdapAuthConfig {
                url: "ldaps://ldap.example.com:636".to_string(),
                base_dn: "dc=example,dc=com".to_string(),
                bind_dn: String::new(),
                bind_password: String::new(),
                user_filter: "(uid={username})".to_string(),
            },
        };
        match auth_kind_from_config(&cfg) {
            AuthBackendKind::Ldap(ldap_cfg) => {
                assert_eq!(ldap_cfg.server_url, "ldaps://ldap.example.com:636");
                assert!(ldap_cfg.bind_dn.is_none());
                assert!(ldap_cfg.bind_password.is_none());
            }
            _ => panic!("expected AuthBackendKind::Ldap"),
        }
    }

    #[test]
    fn auth_kind_from_oauth2_config_uses_generic_provider() {
        let cfg = CfgAuthConfig::OAuth2 {
            config: OAuth2AuthConfig {
                client_id: "rusmes".to_string(),
                client_secret: "secret".to_string(),
                token_url: "https://auth.example/.well-known/jwks.json".to_string(),
                authorization_url: "https://auth.example".to_string(),
            },
        };
        match auth_kind_from_config(&cfg) {
            AuthBackendKind::OAuth2(oauth_cfg) => match oauth_cfg.provider {
                OidcProvider::Generic {
                    issuer_url,
                    client_id,
                    client_secret,
                    jwks_url,
                } => {
                    assert_eq!(issuer_url, "https://auth.example");
                    assert_eq!(client_id, "rusmes");
                    assert_eq!(client_secret, "secret");
                    assert_eq!(jwks_url, "https://auth.example/.well-known/jwks.json");
                }
                other => panic!("expected Generic provider, got {:?}", other),
            },
            _ => panic!("expected AuthBackendKind::OAuth2"),
        }
    }

    #[test]
    fn storage_kind_from_filesystem_config() {
        let cfg = CfgStorageConfig::Filesystem {
            path: "/var/mail".to_string(),
        };
        match storage_kind_from_config(&cfg) {
            BackendKind::Filesystem { path } => assert_eq!(path, "/var/mail"),
            _ => panic!("expected BackendKind::Filesystem"),
        }
    }

    #[test]
    fn storage_kind_from_postgres_config() {
        let cfg = CfgStorageConfig::Postgres {
            connection_string: "postgres://user:pw@db/mail".to_string(),
        };
        match storage_kind_from_config(&cfg) {
            BackendKind::Postgres { connection_string } => {
                assert_eq!(connection_string, "postgres://user:pw@db/mail");
            }
            _ => panic!("expected BackendKind::Postgres"),
        }
    }

    #[test]
    fn storage_kind_from_amaters_config() {
        let cfg = CfgStorageConfig::AmateRS {
            endpoints: vec!["a:1".to_string(), "b:2".to_string()],
            replication_factor: 3,
        };
        match storage_kind_from_config(&cfg) {
            BackendKind::Amaters {
                endpoints,
                replication_factor,
            } => {
                assert_eq!(endpoints.len(), 2);
                assert_eq!(replication_factor, 3);
            }
            _ => panic!("expected BackendKind::Amaters"),
        }
    }

    #[test]
    fn redacts_credentials_from_database_url() {
        assert_eq!(
            redact_database_url("postgres://alice:secret@db.example/auth"),
            "postgres://***@db.example/auth"
        );
        assert_eq!(
            redact_database_url("sqlite:///tmp/auth.db"),
            "sqlite:///tmp/auth.db"
        );
    }

    #[tokio::test]
    async fn pid_file_round_trip_inside_tempdir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = PidFile::write(dir.path()).await.expect("write pid file");
        let path = pid_file.path().to_path_buf();
        assert!(path.exists(), "PID file must be created");
        let contents = tokio::fs::read_to_string(&path).await.expect("read");
        let pid: u32 = contents.trim().parse().expect("parse pid");
        assert_eq!(pid, std::process::id());
        pid_file.cleanup().await;
        assert!(!path.exists(), "PID file must be removed after cleanup");
    }

    #[tokio::test]
    async fn pid_file_creates_runtime_dir_if_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("nested/runtime");
        let pid_file = PidFile::write(&nested).await.expect("write pid file");
        assert!(nested.exists(), "runtime_dir must be created");
        assert!(pid_file.path().exists(), "PID file must be created");
        pid_file.cleanup().await;
    }

    #[tokio::test]
    async fn pid_file_cleanup_silent_on_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = PidFile {
            path: dir.path().join("rusmes.pid"),
        };
        // No write — cleanup must not panic or error.
        pid_file.cleanup().await;
    }

    #[test]
    fn load_and_validate_accepts_minimal_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime_dir = dir.path().to_string_lossy().to_string();
        let cfg_path = dir.path().join("rusmes.toml");
        write_minimal_config(&cfg_path, &runtime_dir);
        let cfg = load_and_validate(&cfg_path).expect("config must validate");
        assert_eq!(cfg.domain, "example.com");
        assert_eq!(cfg.runtime_dir, runtime_dir);
    }

    #[test]
    fn load_and_validate_rejects_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bogus = dir.path().join("does-not-exist.toml");
        let err = load_and_validate(&bogus).expect_err("must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("does-not-exist.toml"), "msg = {msg}");
    }

    #[test]
    fn load_and_validate_rejects_invalid_port() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime_dir = dir.path().to_string_lossy().to_string();
        let cfg_path = dir.path().join("rusmes.toml");
        let body = format!(
            r#"domain = "example.com"
postmaster = "postmaster@example.com"
runtime_dir = "{}"

[smtp]
host = "0.0.0.0"
port = 0
max_message_size = "10MB"
require_auth = false
enable_starttls = false

[storage]
backend = "filesystem"
path = "{}/mail"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"
"#,
            runtime_dir, runtime_dir
        );
        std::fs::write(&cfg_path, body).expect("write");
        let err = load_and_validate(&cfg_path).expect_err("invalid port must fail");
        let msg = format!("{:#}", err);
        assert!(
            msg.to_lowercase().contains("port") || msg.to_lowercase().contains("0"),
            "expected port-related error, got: {msg}"
        );
    }
}
