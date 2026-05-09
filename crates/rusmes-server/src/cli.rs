//! Command-line interface for `rusmes-server`.
//!
//! The CLI uses `clap` derive to expose:
//! - `-c/--config <PATH>` — explicit config flag (preferred).
//! - A positional `[CONFIG]` fallback — preserved for one release for backwards
//!   compatibility with older invocations such as `rusmes-server rusmes.toml`.
//!   The fallback emits a deprecation warning to stderr when used.
//! - `--check-config` — load + validate the configuration, then exit. No
//!   sockets are opened, no servers are spawned.
//!
//! Resolution rules:
//! - If `--config` is provided, it wins (even if a positional argument is also
//!   present — the positional is silently ignored to keep the meaning of the
//!   flag unambiguous).
//! - Otherwise, the positional is used (with a deprecation warning).
//! - Otherwise, the default `rusmes.toml` in the current directory is used.

use clap::Parser;
use std::path::PathBuf;

/// `rusmes-server` — RusMES mail-server orchestrator.
#[derive(Debug, Clone, Parser)]
#[command(name = "rusmes-server", version, about, long_about = None)]
pub struct Cli {
    /// Path to the configuration file (TOML or YAML).
    #[arg(short = 'c', long = "config", value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Validate the configuration file and exit without starting any servers.
    ///
    /// Exits 0 on success, 1 on any validation error. Diagnostics are printed
    /// to stderr.
    #[arg(long = "check-config", default_value_t = false)]
    pub check_config: bool,

    /// Deprecated: positional path to the configuration file. Use `--config`
    /// instead. Will be removed in the next minor release.
    #[arg(value_name = "CONFIG")]
    pub positional_config: Option<PathBuf>,
}

impl Cli {
    /// Resolve the effective configuration file path according to the
    /// precedence rules documented at the module level.
    ///
    /// Returns `(path, used_positional_fallback)`. Callers should emit a
    /// deprecation warning if `used_positional_fallback` is `true`.
    pub fn resolve_config_path(&self) -> (PathBuf, bool) {
        if let Some(ref path) = self.config {
            return (path.clone(), false);
        }
        if let Some(ref path) = self.positional_config {
            return (path.clone(), true);
        }
        (PathBuf::from("rusmes.toml"), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_explicit_config_flag() {
        let cli = Cli::parse_from(["rusmes-server", "-c", "/etc/rusmes/rusmes.toml"]);
        let (path, fallback) = cli.resolve_config_path();
        assert_eq!(path, PathBuf::from("/etc/rusmes/rusmes.toml"));
        assert!(!fallback);
        assert!(!cli.check_config);
    }

    #[test]
    fn resolves_long_form_config_flag() {
        let cli = Cli::parse_from(["rusmes-server", "--config", "x.toml"]);
        let (path, fallback) = cli.resolve_config_path();
        assert_eq!(path, PathBuf::from("x.toml"));
        assert!(!fallback);
    }

    #[test]
    fn resolves_positional_fallback_with_deprecation_flag() {
        let cli = Cli::parse_from(["rusmes-server", "rusmes.toml"]);
        let (path, fallback) = cli.resolve_config_path();
        assert_eq!(path, PathBuf::from("rusmes.toml"));
        assert!(fallback);
    }

    #[test]
    fn flag_takes_precedence_over_positional() {
        let cli = Cli::parse_from(["rusmes-server", "-c", "flag.toml", "positional.toml"]);
        let (path, fallback) = cli.resolve_config_path();
        assert_eq!(path, PathBuf::from("flag.toml"));
        assert!(!fallback);
    }

    #[test]
    fn resolves_default_when_nothing_supplied() {
        let cli = Cli::parse_from(["rusmes-server"]);
        let (path, fallback) = cli.resolve_config_path();
        assert_eq!(path, PathBuf::from("rusmes.toml"));
        assert!(!fallback);
    }

    #[test]
    fn check_config_flag_parses() {
        let cli = Cli::parse_from(["rusmes-server", "--check-config", "-c", "x.toml"]);
        assert!(cli.check_config);
    }
}
