//! Performance tuning configuration for RusMES.
//!
//! The `[performance]` TOML section controls thread counts and per-connection
//! buffer sizes. All fields are optional — omitting the entire section (or
//! individual fields) uses the `Default` values documented below.
//!
//! ## Example
//!
//! ```toml
//! [performance]
//! worker_threads = 4
//! imap_pool_size = 128
//! smtp_pool_size = 128
//! read_buffer_kb = 32
//! write_buffer_kb = 32
//! ```

use serde::{Deserialize, Serialize};

/// Runtime performance tuning parameters.
///
/// Exposed via the `[performance]` TOML section. Defaults are conservative
/// values that work well on typical single-node deployments. Increase pool
/// sizes and buffer sizes on high-traffic installations with ample RAM.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PerformanceConfig {
    /// Default: `None` (use Tokio's default, typically the number of logical
    /// CPUs). Set to override the number of Tokio worker threads at runtime
    /// startup. Values of `0` are rejected by validation.
    #[serde(default)]
    pub worker_threads: Option<usize>,

    /// Default: `64`. Maximum number of concurrent IMAP connections the server
    /// will accept per listener socket before new connections are queued.
    #[serde(default = "default_pool_size")]
    pub imap_pool_size: usize,

    /// Default: `64`. Maximum number of concurrent SMTP connections the server
    /// will accept per listener socket before new connections are queued.
    #[serde(default = "default_pool_size")]
    pub smtp_pool_size: usize,

    /// Default: `64`. Read buffer size in kibibytes allocated per connection.
    /// Larger values reduce system-call overhead on bulk transfers at the cost
    /// of memory.
    #[serde(default = "default_buffer_kb")]
    pub read_buffer_kb: usize,

    /// Default: `64`. Write buffer size in kibibytes allocated per connection.
    /// Larger values reduce system-call overhead on bulk transfers at the cost
    /// of memory.
    #[serde(default = "default_buffer_kb")]
    pub write_buffer_kb: usize,
}

fn default_pool_size() -> usize {
    64
}

fn default_buffer_kb() -> usize {
    64
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            worker_threads: None,
            imap_pool_size: default_pool_size(),
            smtp_pool_size: default_pool_size(),
            read_buffer_kb: default_buffer_kb(),
            write_buffer_kb: default_buffer_kb(),
        }
    }
}

impl PerformanceConfig {
    /// Validate performance configuration values.
    ///
    /// Returns an error if any pool size or buffer size is zero, or if
    /// `worker_threads` is explicitly set to zero.
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(threads) = self.worker_threads {
            if threads == 0 {
                anyhow::bail!("performance.worker_threads must be greater than 0");
            }
        }
        if self.imap_pool_size == 0 {
            anyhow::bail!("performance.imap_pool_size must be greater than 0");
        }
        if self.smtp_pool_size == 0 {
            anyhow::bail!("performance.smtp_pool_size must be greater than 0");
        }
        if self.read_buffer_kb == 0 {
            anyhow::bail!("performance.read_buffer_kb must be greater than 0");
        }
        if self.write_buffer_kb == 0 {
            anyhow::bail!("performance.write_buffer_kb must be greater than 0");
        }
        Ok(())
    }

    /// Return `worker_threads` or a sensible fallback based on the number of
    /// logical CPUs (minimum 1).
    pub fn effective_worker_threads(&self) -> usize {
        self.worker_threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        })
    }
}
