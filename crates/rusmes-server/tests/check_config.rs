//! Integration tests for the `rusmes-server` `--check-config` CLI flag.
//!
//! These tests build a tempdir-backed configuration file, invoke the binary
//! with `assert_cmd`, and verify the documented exit codes:
//! - exit 0 on a valid configuration
//! - exit 1 on a malformed or invalid configuration
//!
//! No sockets are opened during these tests because `--check-config` is the
//! validation-only path.

use assert_cmd::Command;
use std::fs;

fn write_valid_config(path: &std::path::Path, runtime_dir: &str) {
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
    fs::write(path, body).expect("write valid config");
}

fn write_invalid_config(path: &std::path::Path) {
    // Domain is empty → `validate_domain` fails.
    let body = r#"domain = ""
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 2525
max_message_size = "10MB"
require_auth = false
enable_starttls = false

[storage]
backend = "filesystem"
path = "/tmp/rusmes-bad"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"
"#;
    fs::write(path, body).expect("write invalid config");
}

#[test]
fn check_config_succeeds_on_valid_config() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let runtime_dir = tmp.path().to_string_lossy().to_string();
    let cfg_path = tmp.path().join("rusmes.toml");
    write_valid_config(&cfg_path, &runtime_dir);

    let mut cmd = Command::cargo_bin("rusmes-server").expect("locate built binary");
    cmd.arg("--check-config").arg("-c").arg(&cfg_path);

    let output = cmd.output().expect("run binary");
    assert!(
        output.status.success(),
        "expected exit 0 on valid config, got {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn check_config_fails_on_invalid_config() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("rusmes.toml");
    write_invalid_config(&cfg_path);

    let mut cmd = Command::cargo_bin("rusmes-server").expect("locate built binary");
    cmd.arg("--check-config").arg("-c").arg(&cfg_path);

    let output = cmd.output().expect("run binary");
    assert!(
        !output.status.success(),
        "expected non-zero exit on invalid config, got success\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1, got {:?}",
        output.status.code()
    );
}

#[test]
fn check_config_fails_on_missing_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cfg_path = tmp.path().join("does-not-exist.toml");

    let mut cmd = Command::cargo_bin("rusmes-server").expect("locate built binary");
    cmd.arg("--check-config").arg("-c").arg(&cfg_path);

    let output = cmd.output().expect("run binary");
    assert!(
        !output.status.success(),
        "expected non-zero exit on missing config, got success"
    );
}
