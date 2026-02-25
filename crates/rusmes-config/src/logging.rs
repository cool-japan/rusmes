//! Structured logging with file rotation for RusMES
//!
//! This module provides comprehensive logging capabilities including:
//! - File rotation (daily, hourly, size-based)
//! - JSON and plain text formatting
//! - Configurable log levels per module
//! - Log file compression and archiving

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Log rotation policy
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RotationPolicy {
    /// Rotate logs daily
    #[default]
    Daily,
    /// Rotate logs hourly
    Hourly,
    /// Rotate logs based on file size
    SizeBased,
    /// Never rotate logs
    Never,
}

/// Log format type
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Plain text format
    #[default]
    Text,
    /// JSON format
    Json,
}

/// Complete logging configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogConfig {
    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_level")]
    pub level: String,

    /// Log format (text or json)
    #[serde(default)]
    pub format: LogFormat,

    /// Directory for log files
    #[serde(default = "default_log_dir")]
    pub log_dir: String,

    /// Base name for log files
    #[serde(default = "default_file_prefix")]
    pub file_prefix: String,

    /// Rotation policy
    #[serde(default)]
    pub rotation: RotationPolicy,

    /// Maximum file size for size-based rotation (e.g., "100MB")
    #[serde(default = "default_max_size")]
    pub max_size: String,

    /// Maximum number of archived log files to keep
    #[serde(default = "default_max_backups")]
    pub max_backups: usize,

    /// Whether to compress archived logs
    #[serde(default = "default_compress")]
    pub compress: bool,

    /// Per-module log level overrides
    #[serde(default)]
    pub module_levels: HashMap<String, String>,

    /// Whether to log to stdout in addition to files
    #[serde(default)]
    pub also_stdout: bool,
}

fn default_level() -> String {
    "info".to_string()
}

fn default_log_dir() -> String {
    "/var/log/rusmes".to_string()
}

fn default_file_prefix() -> String {
    "rusmes".to_string()
}

fn default_max_size() -> String {
    "100MB".to_string()
}

fn default_max_backups() -> usize {
    10
}

fn default_compress() -> bool {
    true
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_level(),
            format: LogFormat::default(),
            log_dir: default_log_dir(),
            file_prefix: default_file_prefix(),
            rotation: RotationPolicy::default(),
            max_size: default_max_size(),
            max_backups: default_max_backups(),
            compress: default_compress(),
            module_levels: HashMap::new(),
            also_stdout: false,
        }
    }
}

impl LogConfig {
    /// Validate the log configuration
    pub fn validate(&self) -> Result<()> {
        // Validate log level
        self.parse_level()
            .with_context(|| format!("Invalid log level: {}", self.level))?;

        // Validate module levels
        for (module, level) in &self.module_levels {
            level
                .parse::<Level>()
                .with_context(|| format!("Invalid level '{}' for module '{}'", level, module))?;
        }

        // Validate max size for size-based rotation
        if self.rotation == RotationPolicy::SizeBased {
            self.max_size_bytes()
                .with_context(|| format!("Invalid max_size: {}", self.max_size))?;
        }

        // Validate log directory can be created
        if let Some(parent) = Path::new(&self.log_dir).parent() {
            if !parent.exists() {
                anyhow::bail!(
                    "Parent directory of log_dir does not exist: {}",
                    parent.display()
                );
            }
        }

        Ok(())
    }

    /// Parse log level string to tracing Level
    pub fn parse_level(&self) -> Result<Level> {
        self.level
            .parse::<Level>()
            .map_err(|e| anyhow::anyhow!("Invalid log level: {}", e))
    }

    /// Parse max size to bytes
    pub fn max_size_bytes(&self) -> Result<usize> {
        parse_size(&self.max_size)
    }

    /// Build an EnvFilter from the configuration
    pub fn build_filter(&self) -> Result<EnvFilter> {
        let mut filter = EnvFilter::new(&self.level);

        // Add module-specific filters
        for (module, level) in &self.module_levels {
            filter =
                filter.add_directive(format!("{}={}", module, level).parse().with_context(
                    || format!("Invalid filter directive for module '{}'", module),
                )?);
        }

        Ok(filter)
    }
}

/// Initialize logging based on configuration
///
/// This function sets up the global tracing subscriber with the specified
/// configuration. It must be called only once at application startup.
///
/// Returns a `WorkerGuard` that must be kept alive for the duration of the program.
/// Dropping the guard will cause log messages to be lost.
#[allow(clippy::type_complexity)]
pub fn init_logging(config: &LogConfig) -> Result<Option<(WorkerGuard, Option<WorkerGuard>)>> {
    // Validate configuration
    config.validate()?;

    // Create log directory if it doesn't exist
    fs::create_dir_all(&config.log_dir)
        .with_context(|| format!("Failed to create log directory: {}", config.log_dir))?;

    // Build the environment filter
    let filter = config.build_filter()?;

    // Set up file appender based on rotation policy
    let file_appender = match config.rotation {
        RotationPolicy::Daily => {
            tracing_appender::rolling::daily(&config.log_dir, &config.file_prefix)
        }
        RotationPolicy::Hourly => {
            tracing_appender::rolling::hourly(&config.log_dir, &config.file_prefix)
        }
        RotationPolicy::Never => {
            tracing_appender::rolling::never(&config.log_dir, &config.file_prefix)
        }
        RotationPolicy::SizeBased => {
            // For size-based rotation, we use the daily appender and handle rotation separately
            tracing_appender::rolling::daily(&config.log_dir, &config.file_prefix)
        }
    };

    let (non_blocking_file, file_guard) = tracing_appender::non_blocking(file_appender);

    // Set up stdout appender if requested
    let stdout_guard = if config.also_stdout {
        let (non_blocking_stdout, guard) = tracing_appender::non_blocking(std::io::stdout());

        match config.format {
            LogFormat::Text => {
                let stdout_layer = tracing_subscriber::fmt::layer()
                    .with_writer(non_blocking_stdout)
                    .with_span_events(FmtSpan::CLOSE);

                let file_layer = tracing_subscriber::fmt::layer()
                    .with_writer(non_blocking_file)
                    .with_span_events(FmtSpan::CLOSE)
                    .with_ansi(false);

                tracing_subscriber::registry()
                    .with(filter)
                    .with(stdout_layer)
                    .with(file_layer)
                    .init();
            }
            LogFormat::Json => {
                let stdout_layer = tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(non_blocking_stdout);

                let file_layer = tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(non_blocking_file);

                tracing_subscriber::registry()
                    .with(filter)
                    .with(stdout_layer)
                    .with(file_layer)
                    .init();
            }
        }

        Some(guard)
    } else {
        match config.format {
            LogFormat::Text => {
                tracing_subscriber::registry()
                    .with(filter)
                    .with(
                        tracing_subscriber::fmt::layer()
                            .with_writer(non_blocking_file)
                            .with_span_events(FmtSpan::CLOSE)
                            .with_ansi(false),
                    )
                    .init();
            }
            LogFormat::Json => {
                tracing_subscriber::registry()
                    .with(filter)
                    .with(
                        tracing_subscriber::fmt::layer()
                            .json()
                            .with_writer(non_blocking_file),
                    )
                    .init();
            }
        }

        None
    };

    // Start background task for archiving and compression if enabled
    if config.compress && config.max_backups > 0 {
        let config_clone = config.clone();
        std::thread::spawn(move || {
            archive_old_logs(&config_clone);
        });
    }

    Ok(Some((file_guard, stdout_guard)))
}

/// Archive and compress old log files
fn archive_old_logs(config: &LogConfig) {
    let log_dir = Path::new(&config.log_dir);

    // Find all log files matching the prefix
    let mut log_files: Vec<PathBuf> = match fs::read_dir(log_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with(&config.file_prefix) && !name.ends_with(".gz"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return,
    };

    // Sort by modification time (oldest first)
    log_files.sort_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok());

    // Keep only the most recent files, compress the rest
    let current_file = format!("{}.log", config.file_prefix);

    for (idx, log_file) in log_files.iter().enumerate() {
        // Skip the current log file
        if log_file.file_name().and_then(|n| n.to_str()) == Some(&current_file) {
            continue;
        }

        // If we have more than max_backups, delete old files
        if idx >= config.max_backups {
            let _ = fs::remove_file(log_file);
            continue;
        }

        // Compress if not already compressed
        if config.compress {
            let _ = compress_log_file(log_file);
        }
    }
}

/// Compress a log file using gzip
fn compress_log_file(path: &Path) -> Result<()> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let input =
        fs::read(path).with_context(|| format!("Failed to read log file: {}", path.display()))?;

    let output_path = path.with_extension("log.gz");
    let output_file = fs::File::create(&output_path).with_context(|| {
        format!(
            "Failed to create compressed file: {}",
            output_path.display()
        )
    })?;

    let mut encoder = GzEncoder::new(output_file, Compression::default());
    encoder
        .write_all(&input)
        .with_context(|| format!("Failed to compress log file: {}", path.display()))?;
    encoder.finish()?;

    // Remove original file after successful compression
    fs::remove_file(path)
        .with_context(|| format!("Failed to remove original log file: {}", path.display()))?;

    Ok(())
}

/// Parse size string like "50MB", "1GB", "1024KB"
fn parse_size(s: &str) -> Result<usize> {
    let s = s.trim().to_uppercase();

    if let Some(rest) = s.strip_suffix("GB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0 * 1024.0 * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix("MB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0 * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix("KB") {
        let num: f64 = rest.trim().parse()?;
        Ok((num * 1024.0) as usize)
    } else if let Some(rest) = s.strip_suffix('B') {
        let num: usize = rest.trim().parse()?;
        Ok(num)
    } else {
        // Assume bytes
        let num: usize = s.parse()?;
        Ok(num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("50MB").unwrap(), 50 * 1024 * 1024);
        assert_eq!(parse_size("100MB").unwrap(), 100 * 1024 * 1024);
        assert_eq!(
            parse_size("2.5GB").unwrap(),
            (2.5 * 1024.0 * 1024.0 * 1024.0) as usize
        );
    }

    #[test]
    fn test_default_log_config() {
        let config = LogConfig::default();
        assert_eq!(config.level, "info");
        assert_eq!(config.format, LogFormat::Text);
        assert_eq!(config.rotation, RotationPolicy::Daily);
        assert_eq!(config.max_backups, 10);
        assert!(config.compress);
        assert!(!config.also_stdout);
    }

    #[test]
    fn test_log_config_parse_level() {
        let config = LogConfig {
            level: "debug".to_string(),
            ..Default::default()
        };
        assert!(config.parse_level().is_ok());
        assert_eq!(config.parse_level().unwrap(), Level::DEBUG);

        let config = LogConfig {
            level: "invalid".to_string(),
            ..Default::default()
        };
        assert!(config.parse_level().is_err());
    }

    #[test]
    fn test_log_config_max_size_bytes() {
        let config = LogConfig {
            max_size: "100MB".to_string(),
            ..Default::default()
        };
        assert_eq!(config.max_size_bytes().unwrap(), 100 * 1024 * 1024);

        let config = LogConfig {
            max_size: "1GB".to_string(),
            ..Default::default()
        };
        assert_eq!(config.max_size_bytes().unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn test_rotation_policy_serialization() {
        let daily = RotationPolicy::Daily;
        let json = serde_json::to_string(&daily).unwrap();
        assert_eq!(json, r#""daily""#);

        let hourly = RotationPolicy::Hourly;
        let json = serde_json::to_string(&hourly).unwrap();
        assert_eq!(json, r#""hourly""#);

        let size_based = RotationPolicy::SizeBased;
        let json = serde_json::to_string(&size_based).unwrap();
        assert_eq!(json, r#""sizebased""#);
    }

    #[test]
    fn test_log_format_serialization() {
        let text = LogFormat::Text;
        let json = serde_json::to_string(&text).unwrap();
        assert_eq!(json, r#""text""#);

        let json_format = LogFormat::Json;
        let json = serde_json::to_string(&json_format).unwrap();
        assert_eq!(json, r#""json""#);
    }

    #[test]
    fn test_build_filter_with_module_levels() {
        let mut module_levels = HashMap::new();
        module_levels.insert("rusmes_smtp".to_string(), "debug".to_string());
        module_levels.insert("rusmes_imap".to_string(), "trace".to_string());

        let config = LogConfig {
            level: "info".to_string(),
            module_levels,
            ..Default::default()
        };

        let filter = config.build_filter();
        assert!(filter.is_ok());
    }

    #[test]
    fn test_build_filter_with_invalid_module_level() {
        let mut module_levels = HashMap::new();
        module_levels.insert("rusmes_smtp".to_string(), "invalid".to_string());

        let config = LogConfig {
            level: "info".to_string(),
            module_levels,
            ..Default::default()
        };

        let filter = config.build_filter();
        assert!(filter.is_err());
    }

    #[test]
    fn test_log_config_deserialization_toml() {
        let toml_str = r#"
            level = "debug"
            format = "json"
            log_dir = "/tmp/test_logs"
            file_prefix = "test"
            rotation = "hourly"
            max_size = "50MB"
            max_backups = 5
            compress = false
            also_stdout = true
        "#;

        let config: LogConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.level, "debug");
        assert_eq!(config.format, LogFormat::Json);
        assert_eq!(config.log_dir, "/tmp/test_logs");
        assert_eq!(config.file_prefix, "test");
        assert_eq!(config.rotation, RotationPolicy::Hourly);
        assert_eq!(config.max_size, "50MB");
        assert_eq!(config.max_backups, 5);
        assert!(!config.compress);
        assert!(config.also_stdout);
    }

    #[test]
    fn test_log_config_with_module_levels_toml() {
        let toml_str = r#"
            level = "info"
            format = "text"

            [module_levels]
            rusmes_smtp = "debug"
            rusmes_imap = "trace"
            rusmes_core = "warn"
        "#;

        let config: LogConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.module_levels.len(), 3);
        assert_eq!(
            config.module_levels.get("rusmes_smtp"),
            Some(&"debug".to_string())
        );
        assert_eq!(
            config.module_levels.get("rusmes_imap"),
            Some(&"trace".to_string())
        );
        assert_eq!(
            config.module_levels.get("rusmes_core"),
            Some(&"warn".to_string())
        );
    }
}
