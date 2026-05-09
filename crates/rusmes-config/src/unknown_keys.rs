//! Detection of unknown top-level and section keys in TOML configuration files.
//!
//! This module provides the static key tables and the [`collect_unknown_toml_keys`]
//! helper used by [`crate::ServerConfig::from_file`] to emit actionable
//! `tracing::warn!` messages when operators introduce typos or stale keys.

/// All recognized top-level keys in a [`crate::ServerConfig`] TOML file.
///
/// Used by the two-phase unknown-key detection in [`crate::ServerConfig::from_file`].
pub(crate) const KNOWN_TOP_LEVEL_KEYS: &[&str] = &[
    "domain",
    "postmaster",
    "smtp",
    "imap",
    "jmap",
    "pop3",
    "storage",
    "processors",
    "runtime_dir",
    "relay",
    "auth",
    "logging",
    "queue",
    "security",
    "domains",
    "metrics",
    "tracing",
    "connection_limits",
    "performance",
    "tls",
    "chroot",
    "run_as_user",
    "run_as_group",
];

/// Known keys for each nested (non-tagged-enum) section table.
///
/// Entries are `(section_name, &[known_keys])`.  Tagged-enum sections
/// (`storage`, `auth`, `processors`) are intentionally excluded because
/// their contents depend on the active variant and cannot be statically
/// enumerated here — TOML's typed deserializer will reject unknown keys
/// inside those sections on its own.
pub(crate) const KNOWN_SECTION_KEYS: &[(&str, &[&str])] = &[
    (
        "smtp",
        &[
            "host",
            "port",
            "tls_port",
            "max_message_size",
            "require_auth",
            "enable_starttls",
            "rate_limit",
        ],
    ),
    ("imap", &["host", "port", "tls_port"]),
    ("jmap", &["host", "port", "base_url"]),
    (
        "pop3",
        &["host", "port", "tls_port", "timeout_seconds", "enable_stls"],
    ),
    (
        "relay",
        &["host", "port", "username", "password", "use_tls"],
    ),
    ("logging", &["level", "format", "output", "file"]),
    (
        "queue",
        &[
            "initial_delay",
            "max_delay",
            "backoff_multiplier",
            "max_attempts",
            "worker_threads",
            "batch_size",
        ],
    ),
    (
        "security",
        &[
            "relay_networks",
            "blocked_ips",
            "check_recipient_exists",
            "reject_unknown_recipients",
        ],
    ),
    ("domains", &["local_domains", "aliases"]),
    (
        "metrics",
        &["enabled", "bind_address", "path", "basic_auth"],
    ),
    (
        "tracing",
        &[
            "enabled",
            "endpoint",
            "protocol",
            "service_name",
            "sample_ratio",
        ],
    ),
    (
        "connection_limits",
        &[
            "max_connections_per_ip",
            "max_total_connections",
            "idle_timeout",
            "reaper_interval",
        ],
    ),
    (
        "performance",
        &[
            "worker_threads",
            "imap_pool_size",
            "smtp_pool_size",
            "read_buffer_kb",
            "write_buffer_kb",
        ],
    ),
    // [tls] top-level keys; per-protocol sub-tables each have cert_path/key_path.
    ("tls", &["default", "smtp", "imap", "pop3", "jmap"]),
];

/// Extract unknown keys from a raw TOML value, walking both the top-level
/// table and every known nested section table.
///
/// Returns a `Vec<String>` of unknown key names in `"key"` (top-level) or
/// `"section.key"` (nested) form.  Tagged-enum sections (`storage`, `auth`,
/// `processors`) are skipped because their valid keys vary by variant.
pub(crate) fn collect_unknown_toml_keys(raw: &toml::Value) -> Vec<String> {
    let mut unknown = Vec::new();

    let root = match raw {
        toml::Value::Table(t) => t,
        _ => return unknown,
    };

    // --- Top-level unknown keys ---
    let known_top: std::collections::HashSet<&str> = KNOWN_TOP_LEVEL_KEYS.iter().copied().collect();
    for key in root.keys() {
        if !known_top.contains(key.as_str()) {
            unknown.push(key.clone());
        }
    }

    // --- Nested section unknown keys ---
    for &(section, known_keys) in KNOWN_SECTION_KEYS {
        let section_val = match root.get(section) {
            Some(v) => v,
            None => continue,
        };
        let section_table = match section_val {
            toml::Value::Table(t) => t,
            _ => continue,
        };
        let known_set: std::collections::HashSet<&str> = known_keys.iter().copied().collect();
        for key in section_table.keys() {
            if !known_set.contains(key.as_str()) {
                unknown.push(format!("{}.{}", section, key));
            }
        }
    }

    unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_unknown_toml_keys() {
        let toml_str = "domain = \"example.com\"\nfoo_bar = \"baz\"\nalpha = 1";
        let raw: toml::Value = toml::from_str(toml_str).unwrap();
        let unknown = collect_unknown_toml_keys(&raw);
        assert!(
            unknown.contains(&"foo_bar".to_string()),
            "got: {:?}",
            unknown
        );
        assert!(unknown.contains(&"alpha".to_string()), "got: {:?}", unknown);
        assert!(
            !unknown.contains(&"domain".to_string()),
            "got: {:?}",
            unknown
        );
    }

    #[test]
    fn test_collect_unknown_toml_keys_nested() {
        // A typo inside [smtp] should surface as "smtp.prot".
        let toml_str = r#"
domain = "example.com"
[smtp]
host = "0.0.0.0"
prot = 25
"#;
        let raw: toml::Value = toml::from_str(toml_str).unwrap();
        let unknown = collect_unknown_toml_keys(&raw);
        assert!(
            unknown.contains(&"smtp.prot".to_string()),
            "expected 'smtp.prot' in unknown; got: {:?}",
            unknown
        );
        assert!(
            !unknown.contains(&"smtp.host".to_string()),
            "'smtp.host' is a known field and must not appear; got: {:?}",
            unknown
        );
    }
}
