//! Tests for unknown configuration key warnings (Item 4).
//!
//! `ServerConfig::from_file` uses two-phase TOML parsing:
//!
//! 1. Parse the raw `toml::Value` table to collect unknown top-level keys.
//! 2. Deserialize into the typed `ServerConfig` struct.
//!
//! Unknown keys are stored in `ServerConfig::extra: Vec<String>` and
//! `warn_unknown_keys()` emits one `tracing::warn!` per entry.
//!
//! These tests assert on `config.extra` directly, which is deterministic and
//! does not depend on tracing subscriber capture.

use rusmes_config::ServerConfig;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQUE_ID: AtomicU64 = AtomicU64::new(0);

/// Create a unique writable temporary directory for one test's storage path.
///
/// Isolation ensures parallel tests don't collide on the `.rusmes_write_test`
/// file that `validate_storage_path` creates and removes.
fn unique_storage_dir() -> std::path::PathBuf {
    let id = UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "rusmes_storage_{pid}_{id}",
        pid = std::process::id()
    ));
    std::fs::create_dir_all(&p).expect("failed to create isolated storage dir");
    p
}

/// Build a minimal valid TOML string that includes `extra_toml` BEFORE the
/// `[[processors]]` array-of-tables section.
///
/// IMPORTANT: In TOML, key-value pairs after `[[processors]]` are scoped to
/// that table entry, not the root table. Unknown keys must appear before the
/// first `[[processors]]` header to be captured as top-level unknowns.
///
/// `storage_dir` must be a writable directory so `validate_storage_path` passes.
fn minimal_toml(extra_toml: &str, storage_dir: &std::path::Path) -> String {
    let storage_str = storage_dir.to_string_lossy();
    // NOTE: In TOML, bare key-value pairs after a `[section]` header are
    // scoped to that section, not the root table. `extra_toml` must appear
    // at the top (before the first `[section]`) to be root-level keys.
    format!(
        r#"
domain = "example.com"
postmaster = "postmaster@example.com"
{extra_toml}

[smtp]
host = "0.0.0.0"
port = 25
max_message_size = "50MB"

[storage]
backend = "filesystem"
path = "{storage_str}"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"
"#
    )
}

/// Write `content` to a uniquely-named temp `.toml` file, load it via
/// [`ServerConfig::from_file`], clean up the file, and return the config.
fn load_toml(content: &str) -> anyhow::Result<ServerConfig> {
    let id = UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "rusmes_cfg_{pid}_{id}.toml",
        pid = std::process::id()
    ));
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        f.write_all(content.as_bytes())?;
    }
    let result = ServerConfig::from_file(&path);
    let _ = std::fs::remove_file(&path);
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_valid_config_has_empty_extra() {
    let storage = unique_storage_dir();
    let config = load_toml(&minimal_toml("", &storage)).expect("valid config should load");
    let _ = std::fs::remove_dir_all(&storage);

    assert!(
        config.extra.is_empty(),
        "expected no unknown keys, got: {:?}",
        config.extra
    );
}

#[test]
fn test_unknown_key_captured_in_extra() {
    let storage = unique_storage_dir();
    let config = load_toml(&minimal_toml("foo_bar = \"baz\"", &storage))
        .expect("config with unknown key should still load");
    let _ = std::fs::remove_dir_all(&storage);

    assert!(
        config.extra.contains(&"foo_bar".to_string()),
        "expected 'foo_bar' in extra, got: {:?}",
        config.extra
    );
}

#[test]
fn test_multiple_unknown_keys_all_captured() {
    let storage = unique_storage_dir();
    let config = load_toml(&minimal_toml("alpha = 1\nbeta = 2", &storage))
        .expect("config with multiple unknown keys should still load");
    let _ = std::fs::remove_dir_all(&storage);

    assert!(
        config.extra.contains(&"alpha".to_string()),
        "expected 'alpha' in extra; got: {:?}",
        config.extra
    );
    assert!(
        config.extra.contains(&"beta".to_string()),
        "expected 'beta' in extra; got: {:?}",
        config.extra
    );
}

#[test]
fn test_known_keys_not_in_extra() {
    let storage = unique_storage_dir();
    let config = load_toml(&minimal_toml("", &storage)).expect("valid config should load");
    let _ = std::fs::remove_dir_all(&storage);

    for key in &[
        "domain",
        "smtp",
        "storage",
        "processors",
        "performance",
        "tls",
    ] {
        assert!(
            !config.extra.contains(&(*key).to_string()),
            "'{key}' must not appear in extra; it is a known field"
        );
    }
}

#[test]
fn test_warn_unknown_keys_does_not_panic() {
    let storage = unique_storage_dir();
    let config = load_toml(&minimal_toml("should_warn_key = \"value\"", &storage))
        .expect("config with unknown key should still load");
    let _ = std::fs::remove_dir_all(&storage);

    // Must not panic regardless of tracing subscriber state.
    config.warn_unknown_keys();
}

#[test]
fn test_warn_unknown_keys_is_noop_on_valid_config() {
    let storage = unique_storage_dir();
    let config = load_toml(&minimal_toml("", &storage)).expect("valid config should load");
    let _ = std::fs::remove_dir_all(&storage);

    assert!(config.extra.is_empty());
    config.warn_unknown_keys(); // must not panic
}

#[test]
fn test_extra_not_in_serialized_json() {
    let storage = unique_storage_dir();
    let config = load_toml(&minimal_toml(
        "unknown_sentinel_xyz = \"should_not_appear\"",
        &storage,
    ))
    .expect("config should load");
    let _ = std::fs::remove_dir_all(&storage);

    let json = serde_json::to_string(&config).unwrap();
    assert!(
        !json.contains("unknown_sentinel_xyz"),
        "unknown keys must not appear in serialized output; found in JSON snippet: {}",
        &json[..json.len().min(400)]
    );
}

/// This test verifies that:
/// 1. `extra` is populated with the unknown key (authoritative assertion on struct state).
/// 2. `warn_unknown_keys()` runs without panicking when warnings would be emitted.
///
/// Tracing's callsite-interest caching means that `tracing::subscriber::with_default`
/// does not reliably intercept `tracing::warn!` calls in integration tests; subscriber
/// capture is therefore best-effort and NOT asserted here. The `config.extra` field is
/// the ground truth for unknown-key detection.
#[test]
fn test_unknown_key_stored_in_extra_and_warn_does_not_panic() {
    let storage = unique_storage_dir();
    let config = load_toml(&minimal_toml("foo_bar_sentinel = \"baz\"", &storage))
        .expect("config with unknown key should load");
    let _ = std::fs::remove_dir_all(&storage);

    // The key must be recorded in `extra`.
    assert!(
        config.extra.contains(&"foo_bar_sentinel".to_string()),
        "expected 'foo_bar_sentinel' in extra; got: {:?}",
        config.extra
    );

    // warn_unknown_keys() must not panic when extra is non-empty.
    config.warn_unknown_keys();
}

#[test]
fn test_nested_unknown_key_captured() {
    // A typo inside [smtp] (e.g. `prot` instead of `port`) should appear in
    // `config.extra` as "smtp.prot".
    let storage = unique_storage_dir();
    let storage_str = storage.to_str().expect("utf-8 storage path");
    let toml = format!(
        r#"
domain = "example.com"
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 25
max_message_size = "50MB"
unknown_smtp_key = "x"

[storage]
backend = "filesystem"
path = "{storage_str}"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"
"#
    );
    let config = load_toml(&toml).expect("config with nested unknown key should load");
    let _ = std::fs::remove_dir_all(&storage);

    assert!(
        config.extra.contains(&"smtp.unknown_smtp_key".to_string()),
        "expected 'smtp.unknown_smtp_key' in extra; got: {:?}",
        config.extra
    );
    // Known smtp fields must not appear as unknown.
    assert!(
        !config.extra.contains(&"smtp.host".to_string()),
        "'smtp.host' is a known key and must not appear in extra; got: {:?}",
        config.extra
    );
}
