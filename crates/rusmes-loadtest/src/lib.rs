//! # rusmes-loadtest
//!
//! Multi-protocol load testing tool for the **RusMES** Rust Mail Enterprise Server.
//!
//! This crate provides the `rusmes-loadtest` binary, capable of generating
//! realistic, high-throughput email traffic across SMTP, IMAP, JMAP, and POP3
//! protocols simultaneously, while collecting detailed latency and throughput metrics.
//!
//! ## Features
//!
//! - **Multi-protocol** — SMTP, IMAP, JMAP, POP3, and configurable mixed-protocol scenarios
//! - **Configurable scenarios** — SMTP throughput, concurrent connections, mixed protocol,
//!   and sustained-load test patterns
//! - **Flexible workload patterns** — steady, spike, ramp-up, stress, and wave patterns
//! - **HDR histogram latency** — precise percentile reporting (p50, p95, p99, p99.9)
//!   via [`hdrhistogram`]
//! - **Report generation** — JSON, HTML, and CSV output; Prometheus metrics export
//! - **Realistic message generation** — random sizes, template-based, and real-world content types
//!
//! ## Example
//!
//! ```text
//! # 60-second SMTP throughput test at 200 msg/s with 20 workers
//! rusmes-loadtest --host mail.example.com --port 25 \
//!     --protocol smtp --scenario smtp-throughput \
//!     --duration 60 --concurrency 20 --rate 200 \
//!     --output-json /tmp/results.json --output-html /tmp/results.html
//! ```
//!
//! ## Architecture
//!
//! The main entry point is [`LoadTester`], which takes a [`LoadTestConfig`] and
//! orchestrates concurrent workers via Tokio tasks.  Each worker calls into a
//! protocol-specific client ([`protocols::SmtpClient`], [`protocols::ImapClient`], …)
//! and records results into a shared [`LoadTestMetrics`] protected by an
//! `Arc<RwLock<…>>`.

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod config;
pub mod generators;
pub mod metrics;
pub mod protocols;
pub mod reporter;
pub mod scenarios;
pub mod workload;

pub use config::LoadTestConfig;
pub use metrics::{LatencyStats, LoadTestMetrics};
pub use scenarios::{LoadTestScenario, ScenarioRunner};

/// Main load test coordinator
pub struct LoadTester {
    config: LoadTestConfig,
    metrics: Arc<RwLock<LoadTestMetrics>>,
}

impl LoadTester {
    /// Create a new load tester with the given configuration
    pub fn new(config: LoadTestConfig) -> Self {
        Self {
            config,
            metrics: Arc::new(RwLock::new(LoadTestMetrics::new())),
        }
    }

    /// Run the load test
    pub async fn run(&self) -> Result<LoadTestMetrics> {
        tracing::info!("Starting load test with config: {:?}", self.config);

        let scenario = self.config.scenario.create_runner(&self.config);
        scenario.run(self.metrics.clone()).await?;

        let final_metrics = self.metrics.read().await.clone();
        Ok(final_metrics)
    }

    /// Get current metrics
    pub async fn get_metrics(&self) -> LoadTestMetrics {
        self.metrics.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loadtester_creation() {
        let config = LoadTestConfig::default();
        let tester = LoadTester::new(config);
        assert!(Arc::strong_count(&tester.metrics) >= 1);
    }

    #[tokio::test]
    async fn test_loadtester_metrics() {
        let config = LoadTestConfig::default();
        let tester = LoadTester::new(config);
        let metrics = tester.get_metrics().await;
        assert_eq!(metrics.total_requests, 0);
    }
}
