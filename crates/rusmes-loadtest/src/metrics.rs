//! Load test metrics collection and reporting

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Latency statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    pub min: Duration,
    pub max: Duration,
    pub mean: Duration,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub p999: Duration,
}

impl Default for LatencyStats {
    fn default() -> Self {
        Self {
            min: Duration::from_millis(0),
            max: Duration::from_millis(0),
            mean: Duration::from_millis(0),
            p50: Duration::from_millis(0),
            p95: Duration::from_millis(0),
            p99: Duration::from_millis(0),
            p999: Duration::from_millis(0),
        }
    }
}

/// Load test metrics
#[derive(Debug, Clone)]
pub struct LoadTestMetrics {
    /// Total requests sent
    pub total_requests: u64,

    /// Successful requests
    pub successful_requests: u64,

    /// Failed requests
    pub failed_requests: u64,

    /// Total bytes sent
    pub bytes_sent: u64,

    /// Total bytes received
    pub bytes_received: u64,

    /// Start time
    pub start_time: Option<Instant>,

    /// End time
    pub end_time: Option<Instant>,

    /// Latency histogram (in microseconds)
    latency_histogram: Histogram<u64>,

    /// Error messages
    pub errors: Vec<String>,
}

impl LoadTestMetrics {
    /// Create new metrics
    pub fn new() -> Self {
        Self {
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            bytes_sent: 0,
            bytes_received: 0,
            start_time: None,
            end_time: None,
            latency_histogram: Histogram::new(3).expect("Failed to create histogram"),
            errors: Vec::new(),
        }
    }

    /// Record a successful request
    pub fn record_success(&mut self, latency: Duration, bytes_sent: usize, bytes_received: usize) {
        self.total_requests += 1;
        self.successful_requests += 1;
        self.bytes_sent += bytes_sent as u64;
        self.bytes_received += bytes_received as u64;

        let latency_us = latency.as_micros() as u64;
        let _ = self.latency_histogram.record(latency_us);
    }

    /// Record a failed request
    pub fn record_failure(&mut self, error: String) {
        self.total_requests += 1;
        self.failed_requests += 1;
        if self.errors.len() < 100 {
            self.errors.push(error);
        }
    }

    /// Mark test as started
    pub fn mark_started(&mut self) {
        self.start_time = Some(Instant::now());
    }

    /// Mark test as completed
    pub fn mark_completed(&mut self) {
        self.end_time = Some(Instant::now());
    }

    /// Get test duration
    pub fn duration(&self) -> Option<Duration> {
        match (self.start_time, self.end_time) {
            (Some(start), Some(end)) => Some(end.duration_since(start)),
            (Some(start), None) => Some(Instant::now().duration_since(start)),
            _ => None,
        }
    }

    /// Get requests per second
    pub fn requests_per_second(&self) -> f64 {
        if let Some(duration) = self.duration() {
            let secs = duration.as_secs_f64();
            if secs > 0.0 {
                return self.total_requests as f64 / secs;
            }
        }
        0.0
    }

    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_requests > 0 {
            self.successful_requests as f64 / self.total_requests as f64
        } else {
            0.0
        }
    }

    /// Get latency statistics
    pub fn latency_stats(&self) -> LatencyStats {
        if self.latency_histogram.is_empty() {
            return LatencyStats::default();
        }

        LatencyStats {
            min: Duration::from_micros(self.latency_histogram.min()),
            max: Duration::from_micros(self.latency_histogram.max()),
            mean: Duration::from_micros(self.latency_histogram.mean() as u64),
            p50: Duration::from_micros(self.latency_histogram.value_at_quantile(0.50)),
            p95: Duration::from_micros(self.latency_histogram.value_at_quantile(0.95)),
            p99: Duration::from_micros(self.latency_histogram.value_at_quantile(0.99)),
            p999: Duration::from_micros(self.latency_histogram.value_at_quantile(0.999)),
        }
    }

    /// Print summary report
    pub fn print_summary(&self) {
        println!("\n=== Load Test Results ===\n");

        if let Some(duration) = self.duration() {
            println!("Duration: {:.2}s", duration.as_secs_f64());
        }

        println!("Total Requests: {}", self.total_requests);
        println!("Successful: {}", self.successful_requests);
        println!("Failed: {}", self.failed_requests);
        println!("Success Rate: {:.2}%", self.success_rate() * 100.0);
        println!("Throughput: {:.2} req/s", self.requests_per_second());

        println!("\nData Transfer:");
        println!(
            "  Sent: {} bytes ({:.2} MB)",
            self.bytes_sent,
            self.bytes_sent as f64 / 1_000_000.0
        );
        println!(
            "  Received: {} bytes ({:.2} MB)",
            self.bytes_received,
            self.bytes_received as f64 / 1_000_000.0
        );

        let stats = self.latency_stats();
        println!("\nLatency:");
        println!("  Min: {:?}", stats.min);
        println!("  Mean: {:?}", stats.mean);
        println!("  Max: {:?}", stats.max);
        println!("  p50: {:?}", stats.p50);
        println!("  p95: {:?}", stats.p95);
        println!("  p99: {:?}", stats.p99);
        println!("  p99.9: {:?}", stats.p999);

        if !self.errors.is_empty() {
            println!("\nFirst {} Errors:", self.errors.len().min(10));
            for (i, error) in self.errors.iter().take(10).enumerate() {
                println!("  {}: {}", i + 1, error);
            }
        }
    }
}

impl Default for LoadTestMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_metrics() {
        let metrics = LoadTestMetrics::new();
        assert_eq!(metrics.total_requests, 0);
        assert_eq!(metrics.successful_requests, 0);
        assert_eq!(metrics.failed_requests, 0);
    }

    #[test]
    fn test_record_success() {
        let mut metrics = LoadTestMetrics::new();
        metrics.record_success(Duration::from_millis(10), 100, 50);

        assert_eq!(metrics.total_requests, 1);
        assert_eq!(metrics.successful_requests, 1);
        assert_eq!(metrics.bytes_sent, 100);
        assert_eq!(metrics.bytes_received, 50);
    }

    #[test]
    fn test_record_failure() {
        let mut metrics = LoadTestMetrics::new();
        metrics.record_failure("Test error".to_string());

        assert_eq!(metrics.total_requests, 1);
        assert_eq!(metrics.failed_requests, 1);
        assert_eq!(metrics.errors.len(), 1);
    }

    #[test]
    fn test_success_rate() {
        let mut metrics = LoadTestMetrics::new();
        metrics.record_success(Duration::from_millis(10), 100, 50);
        metrics.record_success(Duration::from_millis(10), 100, 50);
        metrics.record_failure("Error".to_string());

        assert_eq!(metrics.success_rate(), 2.0 / 3.0);
    }

    #[test]
    fn test_latency_stats() {
        let mut metrics = LoadTestMetrics::new();
        metrics.record_success(Duration::from_millis(10), 100, 50);
        metrics.record_success(Duration::from_millis(20), 100, 50);
        metrics.record_success(Duration::from_millis(30), 100, 50);

        let stats = metrics.latency_stats();
        assert!(stats.min.as_millis() >= 10);
        assert!(stats.max.as_millis() >= 30);
    }
}
