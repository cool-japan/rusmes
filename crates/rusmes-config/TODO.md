# rusmes-config TODO

## Implemented ✅
- [x] TOML/YAML configuration loading (auto-detect, 1,639 lines)
- [x] All config sections: SMTP, IMAP, JMAP, POP3, Storage, Auth, Queue, Security, Metrics, Tracing, Connection Limits
- [x] Environment variable overrides (30+ `RUSMES_*` variables)
- [x] Configuration validation on load (domain, email, port, paths, processor names)
- [x] Hot-reload on SIGHUP
- [x] Size string parser ("50MB", "1GB"), duration parser ("60s", "30m", "1h")
- [x] Log rotation config (daily/hourly/size-based, JSON/Text format)

## Completed (2026-05-05)
- [x] `[performance]` section (worker threads, pool sizes, buffer sizes) (implemented 2026-05-05)
  - `PerformanceConfig` in `crates/rusmes-config/src/performance.rs` (new submodule).
  - `ServerConfig.performance: PerformanceConfig` with `#[serde(default)]`.
  - Validation in `PerformanceConfig::validate()` (plugged into `ServerConfig::validate`).
  - Tests: `tests/performance_config.rs` (8 tests — defaults, explicit, partial, validate_ok/err, effective_worker_threads).
- [x] TLS certificate paths per protocol (per-protocol override with fallback to default) (implemented 2026-05-05)
  - `TlsEndpointConfig`, `TlsConfig`, `ProtocolKind` in `crates/rusmes-config/src/tls.rs` (new submodule).
  - `ServerConfig.tls: Option<TlsConfig>` with `#[serde(default)]`; no-TLS configs continue to work.
  - `ServerConfig::tls_for_protocol(proto) -> Option<&TlsEndpointConfig>` helper.
  - `TlsConfig::tls_for_protocol(proto) -> &TlsEndpointConfig` (fallback logic).
  - `#[serde(alias = "cert_path")] / #[serde(alias = "key_path")]` for backward compat.
  - Tests: `tests/tls_config.rs` (7 tests — fallback, imap override, all overrides, no section, validate ok/err).
- [x] Default value documentation in struct fields (implemented 2026-05-05)
  - All public fields in `ServerConfig`, `SmtpServerConfig`, `ImapServerConfig`, `JmapServerConfig`, `Pop3ServerConfig`, `RelayConfig`, `ProcessorConfig`, `MailetConfig`, `RateLimitConfig`, `LoggingConfig`, `LogFileConfig`, `QueueConfig`, `SecurityConfig`, `DomainsConfig`, `MetricsConfig`, `TracingConfig`, `ConnectionLimitsConfig` now have `///` doc comments with defaults and descriptions.
  - `RUSTDOCFLAGS="-D warnings" cargo doc -p rusmes-config --all-features --no-deps` exits 0.
- [x] Warn on unknown configuration keys (implemented 2026-05-05, nested detection added 2026-05-05)
  - Two-phase TOML parsing in `ServerConfig::from_file`: first parse as `toml::Value`, collect unknown keys via `collect_unknown_toml_keys`, then deserialize the struct.
  - `ServerConfig::extra: Vec<String>` (`#[serde(skip)]`) holds unknown key names.
  - `ServerConfig::warn_unknown_keys()` emits `tracing::warn!` per unknown key; called by `from_file` automatically.
  - `KNOWN_TOP_LEVEL_KEYS` constant lists all 20 recognized top-level TOML keys.
  - `KNOWN_SECTION_KEYS` constant maps each non-tagged-enum section to its known field names.
  - `collect_unknown_toml_keys` walks root table AND all known nested section tables; unknown nested keys formatted as `"section.key"` (e.g. `"smtp.prot"`).
  - Tagged-enum sections (`storage`, `auth`, `processors`) intentionally excluded from nested walk (serde rejects those on its own).
  - Tests: `tests/unknown_keys.rs` (9 tests — empty extra, top-level capture, multiple top-level, known keys not in extra, no panic, noop, serialization, warn-runs-without-panic, nested key captured).