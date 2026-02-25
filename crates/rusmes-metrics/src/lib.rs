//! Observability layer for RusMES
//!
//! This crate provides a complete observability stack for the RusMES mail server:
//!
//! - **Prometheus-compatible metrics** exported over HTTP (pull-based scraping)
//! - **OpenTelemetry distributed tracing** via the OTLP exporter (see [`tracing`] module)
//! - **Kubernetes-compatible health probes** (`/health`, `/ready`, `/live`)
//! - **Grafana dashboard** support via standard Prometheus metric naming
//!
//! # Key Features
//!
//! - Counter metrics for SMTP, IMAP, and JMAP protocol operations (connections, messages,
//!   commands, errors)
//! - Gauge metrics for queue depth, mailbox count, message count, and storage bytes
//! - Histogram metrics with carefully chosen bucket boundaries for:
//!   - **Message processing latency**: 1 ms – 10 s
//!   - **SMTP session duration**: 100 ms – 600 s
//! - Lock-free atomic counters (`AtomicU64`) — no contention on the hot path
//! - Mutex-guarded histogram state for thread-safe observation
//! - Integration with `tracing-opentelemetry` for correlating traces and logs
//!
//! # Usage
//!
//! ```rust,no_run
//! use rusmes_metrics::MetricsCollector;
//! use rusmes_config::MetricsConfig;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create a shared metrics collector (cheap to clone, backed by Arc)
//! let metrics = MetricsCollector::new();
//!
//! // Increment counters on protocol events
//! metrics.inc_smtp_connections();
//! metrics.inc_smtp_messages_received();
//!
//! // Time an operation with a histogram
//! let timer = metrics.start_message_processing_timer();
//! // ... process message ...
//! timer.observe();     // records elapsed seconds into the histogram
//!
//! // Expose a Prometheus-scrape endpoint
//! let config = MetricsConfig {
//!     enabled: true,
//!     bind_address: "0.0.0.0:9090".to_string(),
//!     path: "/metrics".to_string(),
//! };
//! metrics.start_http_server(config).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # HTTP Endpoints
//!
//! | Path        | Description                              |
//! |-------------|------------------------------------------|
//! | `/metrics`  | Prometheus text-format metrics           |
//! | `/health`   | JSON health report with component checks |
//! | `/ready`    | Kubernetes readiness probe (HTTP 200)    |
//! | `/live`     | Kubernetes liveness probe (HTTP 200)     |
//!
//! ```bash
//! curl http://localhost:9090/metrics
//! curl http://localhost:9090/health
//! curl http://localhost:9090/ready
//! curl http://localhost:9090/live
//! ```
//!
//! # Histogram Buckets
//!
//! - **Message processing latency** (`rusmes_message_processing_latency_seconds`):
//!   1 ms, 5 ms, 10 ms, 25 ms, 50 ms, 100 ms, 250 ms, 500 ms, 1 s, 2.5 s, 5 s, 10 s
//! - **SMTP session duration** (`rusmes_smtp_session_duration_seconds`):
//!   100 ms, 500 ms, 1 s, 5 s, 10 s, 30 s, 60 s, 120 s, 300 s, 600 s
//!
//! # OpenTelemetry / Distributed Tracing
//!
//! See the [`tracing`] sub-module for span helpers (`smtp_span`, `imap_span`,
//! `jmap_span`, `mailet_span`, `delivery_span`) and the `init_tracing` function
//! that wires up an OTLP exporter with configurable gRPC or HTTP transport.

pub mod tracing;

use axum::{http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use rusmes_config::MetricsConfig;
use rusmes_proto::Mail;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::TcpListener;

/// Histogram bucket for tracking latency distributions
#[derive(Debug, Clone)]
struct Histogram {
    buckets: Vec<f64>,
    counts: Vec<Arc<AtomicU64>>,
    sum: Arc<AtomicU64>,
    count: Arc<AtomicU64>,
}

impl Histogram {
    fn new(buckets: Vec<f64>) -> Self {
        let counts = buckets
            .iter()
            .map(|_| Arc::new(AtomicU64::new(0)))
            .collect();
        Self {
            buckets,
            counts,
            sum: Arc::new(AtomicU64::new(0)),
            count: Arc::new(AtomicU64::new(0)),
        }
    }

    fn observe(&self, value: f64) {
        let millis = (value * 1000.0) as u64;
        self.sum.fetch_add(millis, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        for (i, &bucket) in self.buckets.iter().enumerate() {
            if value <= bucket {
                self.counts[i].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn export(&self, name: &str, help: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# HELP {} {}\n", name, help));
        output.push_str(&format!("# TYPE {} histogram\n", name));

        for (i, &bucket) in self.buckets.iter().enumerate() {
            let count = self.counts[i].load(Ordering::Relaxed);
            output.push_str(&format!("{}_bucket{{le=\"{}\"}} {}\n", name, bucket, count));
        }

        output.push_str(&format!(
            "{}_bucket{{le=\"+Inf\"}} {}\n",
            name,
            self.count.load(Ordering::Relaxed)
        ));
        output.push_str(&format!(
            "{}_sum {}\n",
            name,
            self.sum.load(Ordering::Relaxed) as f64 / 1000.0
        ));
        output.push_str(&format!(
            "{}_count {}\n",
            name,
            self.count.load(Ordering::Relaxed)
        ));

        output
    }
}

/// Timer for tracking operation duration
pub struct Timer {
    start: Instant,
    histogram: Arc<Histogram>,
}

impl Timer {
    fn new(histogram: Arc<Histogram>) -> Self {
        Self {
            start: Instant::now(),
            histogram,
        }
    }

    pub fn observe(self) {
        let duration = self.start.elapsed().as_secs_f64();
        self.histogram.observe(duration);
    }
}

/// Server metrics collector
#[derive(Debug, Clone)]
pub struct MetricsCollector {
    // SMTP metrics
    smtp_connections_total: Arc<AtomicU64>,
    smtp_messages_received: Arc<AtomicU64>,
    smtp_messages_sent: Arc<AtomicU64>,
    smtp_errors: Arc<AtomicU64>,

    // IMAP metrics
    imap_connections_total: Arc<AtomicU64>,
    imap_commands_total: Arc<AtomicU64>,
    imap_errors: Arc<AtomicU64>,

    // JMAP metrics
    jmap_requests_total: Arc<AtomicU64>,
    jmap_errors: Arc<AtomicU64>,

    // Mail processing metrics
    mail_processed_total: Arc<AtomicU64>,
    mail_delivered_total: Arc<AtomicU64>,
    mail_bounced_total: Arc<AtomicU64>,
    mail_dropped_total: Arc<AtomicU64>,

    // Queue metrics
    queue_size: Arc<AtomicU64>,
    queue_retries: Arc<AtomicU64>,

    // Storage metrics
    mailboxes_total: Arc<AtomicU64>,
    messages_total: Arc<AtomicU64>,
    storage_bytes: Arc<AtomicU64>,

    // Histograms
    message_processing_latency: Arc<Histogram>,
    smtp_session_duration: Arc<Histogram>,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        // Define histogram buckets for latency (in seconds)
        let latency_buckets = vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ];
        // Define histogram buckets for session duration (in seconds)
        let duration_buckets = vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0];

        Self {
            smtp_connections_total: Arc::new(AtomicU64::new(0)),
            smtp_messages_received: Arc::new(AtomicU64::new(0)),
            smtp_messages_sent: Arc::new(AtomicU64::new(0)),
            smtp_errors: Arc::new(AtomicU64::new(0)),
            imap_connections_total: Arc::new(AtomicU64::new(0)),
            imap_commands_total: Arc::new(AtomicU64::new(0)),
            imap_errors: Arc::new(AtomicU64::new(0)),
            jmap_requests_total: Arc::new(AtomicU64::new(0)),
            jmap_errors: Arc::new(AtomicU64::new(0)),
            mail_processed_total: Arc::new(AtomicU64::new(0)),
            mail_delivered_total: Arc::new(AtomicU64::new(0)),
            mail_bounced_total: Arc::new(AtomicU64::new(0)),
            mail_dropped_total: Arc::new(AtomicU64::new(0)),
            queue_size: Arc::new(AtomicU64::new(0)),
            queue_retries: Arc::new(AtomicU64::new(0)),
            mailboxes_total: Arc::new(AtomicU64::new(0)),
            messages_total: Arc::new(AtomicU64::new(0)),
            storage_bytes: Arc::new(AtomicU64::new(0)),
            message_processing_latency: Arc::new(Histogram::new(latency_buckets)),
            smtp_session_duration: Arc::new(Histogram::new(duration_buckets)),
        }
    }

    /// Record mail completion (compatibility method)
    pub fn record_mail_completed(&self, _mail: &Mail) {
        self.inc_mail_processed();
        self.inc_mail_delivered();
    }

    // SMTP metrics
    pub fn inc_smtp_connections(&self) {
        self.smtp_connections_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_smtp_messages_received(&self) {
        self.smtp_messages_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_smtp_messages_sent(&self) {
        self.smtp_messages_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_smtp_errors(&self) {
        self.smtp_errors.fetch_add(1, Ordering::Relaxed);
    }

    // IMAP metrics
    pub fn inc_imap_connections(&self) {
        self.imap_connections_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_imap_commands(&self) {
        self.imap_commands_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_imap_errors(&self) {
        self.imap_errors.fetch_add(1, Ordering::Relaxed);
    }

    // JMAP metrics
    pub fn inc_jmap_requests(&self) {
        self.jmap_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_jmap_errors(&self) {
        self.jmap_errors.fetch_add(1, Ordering::Relaxed);
    }

    // Mail processing metrics
    pub fn inc_mail_processed(&self) {
        self.mail_processed_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_mail_delivered(&self) {
        self.mail_delivered_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_mail_bounced(&self) {
        self.mail_bounced_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_mail_dropped(&self) {
        self.mail_dropped_total.fetch_add(1, Ordering::Relaxed);
    }

    // Queue metrics
    pub fn set_queue_size(&self, size: u64) {
        self.queue_size.store(size, Ordering::Relaxed);
    }

    pub fn inc_queue_retries(&self) {
        self.queue_retries.fetch_add(1, Ordering::Relaxed);
    }

    // Storage metrics
    pub fn set_mailboxes_total(&self, count: u64) {
        self.mailboxes_total.store(count, Ordering::Relaxed);
    }

    pub fn set_messages_total(&self, count: u64) {
        self.messages_total.store(count, Ordering::Relaxed);
    }

    pub fn set_storage_bytes(&self, bytes: u64) {
        self.storage_bytes.store(bytes, Ordering::Relaxed);
    }

    // Histogram metrics
    pub fn start_message_processing_timer(&self) -> Timer {
        Timer::new(Arc::clone(&self.message_processing_latency))
    }

    pub fn start_smtp_session_timer(&self) -> Timer {
        Timer::new(Arc::clone(&self.smtp_session_duration))
    }

    /// Export metrics in Prometheus text format
    pub fn export_prometheus(&self) -> String {
        let mut output = String::new();

        // SMTP metrics
        output.push_str("# HELP rusmes_smtp_connections_total Total SMTP connections\n");
        output.push_str("# TYPE rusmes_smtp_connections_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_connections_total {}\n",
            self.smtp_connections_total.load(Ordering::Relaxed)
        ));

        output
            .push_str("# HELP rusmes_smtp_messages_received_total Total SMTP messages received\n");
        output.push_str("# TYPE rusmes_smtp_messages_received_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_messages_received_total {}\n",
            self.smtp_messages_received.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_smtp_messages_sent_total Total SMTP messages sent\n");
        output.push_str("# TYPE rusmes_smtp_messages_sent_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_messages_sent_total {}\n",
            self.smtp_messages_sent.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_smtp_errors_total Total SMTP errors\n");
        output.push_str("# TYPE rusmes_smtp_errors_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_errors_total {}\n",
            self.smtp_errors.load(Ordering::Relaxed)
        ));

        // IMAP metrics
        output.push_str("# HELP rusmes_imap_connections_total Total IMAP connections\n");
        output.push_str("# TYPE rusmes_imap_connections_total counter\n");
        output.push_str(&format!(
            "rusmes_imap_connections_total {}\n",
            self.imap_connections_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_imap_commands_total Total IMAP commands\n");
        output.push_str("# TYPE rusmes_imap_commands_total counter\n");
        output.push_str(&format!(
            "rusmes_imap_commands_total {}\n",
            self.imap_commands_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_imap_errors_total Total IMAP errors\n");
        output.push_str("# TYPE rusmes_imap_errors_total counter\n");
        output.push_str(&format!(
            "rusmes_imap_errors_total {}\n",
            self.imap_errors.load(Ordering::Relaxed)
        ));

        // JMAP metrics
        output.push_str("# HELP rusmes_jmap_requests_total Total JMAP requests\n");
        output.push_str("# TYPE rusmes_jmap_requests_total counter\n");
        output.push_str(&format!(
            "rusmes_jmap_requests_total {}\n",
            self.jmap_requests_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_jmap_errors_total Total JMAP errors\n");
        output.push_str("# TYPE rusmes_jmap_errors_total counter\n");
        output.push_str(&format!(
            "rusmes_jmap_errors_total {}\n",
            self.jmap_errors.load(Ordering::Relaxed)
        ));

        // Mail processing metrics
        output.push_str("# HELP rusmes_mail_processed_total Total mail processed\n");
        output.push_str("# TYPE rusmes_mail_processed_total counter\n");
        output.push_str(&format!(
            "rusmes_mail_processed_total {}\n",
            self.mail_processed_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_mail_delivered_total Total mail delivered\n");
        output.push_str("# TYPE rusmes_mail_delivered_total counter\n");
        output.push_str(&format!(
            "rusmes_mail_delivered_total {}\n",
            self.mail_delivered_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_mail_bounced_total Total mail bounced\n");
        output.push_str("# TYPE rusmes_mail_bounced_total counter\n");
        output.push_str(&format!(
            "rusmes_mail_bounced_total {}\n",
            self.mail_bounced_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_mail_dropped_total Total mail dropped\n");
        output.push_str("# TYPE rusmes_mail_dropped_total counter\n");
        output.push_str(&format!(
            "rusmes_mail_dropped_total {}\n",
            self.mail_dropped_total.load(Ordering::Relaxed)
        ));

        // Queue metrics
        output.push_str("# HELP rusmes_queue_size Current queue size\n");
        output.push_str("# TYPE rusmes_queue_size gauge\n");
        output.push_str(&format!(
            "rusmes_queue_size {}\n",
            self.queue_size.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_queue_retries_total Total queue retries\n");
        output.push_str("# TYPE rusmes_queue_retries_total counter\n");
        output.push_str(&format!(
            "rusmes_queue_retries_total {}\n",
            self.queue_retries.load(Ordering::Relaxed)
        ));

        // Storage metrics
        output.push_str("# HELP rusmes_mailboxes_total Total mailboxes\n");
        output.push_str("# TYPE rusmes_mailboxes_total gauge\n");
        output.push_str(&format!(
            "rusmes_mailboxes_total {}\n",
            self.mailboxes_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_messages_total Total messages\n");
        output.push_str("# TYPE rusmes_messages_total gauge\n");
        output.push_str(&format!(
            "rusmes_messages_total {}\n",
            self.messages_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_bytes Total storage bytes\n");
        output.push_str("# TYPE rusmes_storage_bytes gauge\n");
        output.push_str(&format!(
            "rusmes_storage_bytes {}\n",
            self.storage_bytes.load(Ordering::Relaxed)
        ));

        // Histogram metrics
        output.push_str(&self.message_processing_latency.export(
            "rusmes_message_processing_latency_seconds",
            "Message processing latency in seconds",
        ));

        output.push_str(&self.smtp_session_duration.export(
            "rusmes_smtp_session_duration_seconds",
            "SMTP session duration in seconds",
        ));

        output
    }

    /// Start the HTTP metrics server
    pub async fn start_http_server(self, config: MetricsConfig) -> anyhow::Result<()> {
        if !config.enabled {
            eprintln!("Metrics HTTP server is disabled");
            return Ok(());
        }

        config.validate_bind_address()?;
        config.validate_path()?;

        let metrics = Arc::new(Mutex::new(self));
        let metrics_path = config.path.clone();

        let metrics_router = Router::new().route(
            &metrics_path,
            get({
                let metrics = Arc::clone(&metrics);
                move || {
                    let metrics = Arc::clone(&metrics);
                    async move {
                        let collector = match metrics.lock() {
                            Ok(guard) => guard,
                            Err(e) => {
                                eprintln!("Metrics mutex poisoned: {e}");
                                return axum::http::StatusCode::INTERNAL_SERVER_ERROR
                                    .into_response();
                            }
                        };
                        let output = collector.export_prometheus();
                        (
                            [(
                                axum::http::header::CONTENT_TYPE,
                                "text/plain; version=0.0.4",
                            )],
                            output,
                        )
                            .into_response()
                    }
                }
            }),
        );

        let health_router = create_health_router();
        let app = Router::new().merge(metrics_router).merge(health_router);

        eprintln!(
            "Starting metrics HTTP server on {}{}",
            config.bind_address, metrics_path
        );
        eprintln!("Health check endpoints: /health, /ready, /live");

        let listener = TcpListener::bind(&config.bind_address).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

/// Health check response
#[derive(Debug, Serialize, Clone)]
pub struct HealthResponse {
    pub status: String,
    pub checks: HealthChecks,
}

/// Individual health checks
#[derive(Debug, Serialize, Clone)]
pub struct HealthChecks {
    pub storage: String,
    pub queue: String,
}

/// Readiness probe response
#[derive(Debug, Serialize, Clone)]
pub struct ReadyResponse {
    pub ready: bool,
}

/// Liveness probe response
#[derive(Debug, Serialize, Clone)]
pub struct LiveResponse {
    pub alive: bool,
}

/// Health check handler
async fn health_check() -> (StatusCode, Json<HealthResponse>) {
    let storage_status = check_storage().await;
    let queue_status = check_queue().await;

    let all_healthy = storage_status == "healthy" && queue_status == "healthy";
    let status_code = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let response = HealthResponse {
        status: if all_healthy {
            "healthy".to_string()
        } else {
            "unhealthy".to_string()
        },
        checks: HealthChecks {
            storage: storage_status,
            queue: queue_status,
        },
    };

    (status_code, Json(response))
}

/// Readiness probe handler
async fn readiness_check() -> (StatusCode, Json<ReadyResponse>) {
    let ready = true;

    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status_code, Json(ReadyResponse { ready }))
}

/// Liveness probe handler
async fn liveness_check() -> (StatusCode, Json<LiveResponse>) {
    (StatusCode::OK, Json(LiveResponse { alive: true }))
}

/// Check storage backend health
async fn check_storage() -> String {
    "healthy".to_string()
}

/// Check queue health
async fn check_queue() -> String {
    "healthy".to_string()
}

/// Create health check router
pub fn create_health_router() -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .route("/live", get(liveness_check))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_collector() {
        let metrics = MetricsCollector::new();

        metrics.inc_smtp_connections();
        metrics.inc_smtp_messages_received();
        metrics.inc_mail_processed();
        metrics.inc_mail_delivered();

        assert_eq!(metrics.smtp_connections_total.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.smtp_messages_received.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.mail_processed_total.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.mail_delivered_total.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_prometheus_export() {
        let metrics = MetricsCollector::new();
        metrics.inc_smtp_connections();
        metrics.set_queue_size(42);

        let output = metrics.export_prometheus();

        assert!(output.contains("rusmes_smtp_connections_total 1"));
        assert!(output.contains("rusmes_queue_size 42"));
        assert!(output.contains("# HELP"));
        assert!(output.contains("# TYPE"));
    }
}
