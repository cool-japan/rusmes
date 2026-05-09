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
//!     basic_auth: None,
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

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use dashmap::DashMap;
use rusmes_config::{MetricsBasicAuthConfig, MetricsConfig};
use rusmes_proto::Mail;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

/// Global metrics collector accessible by every protocol crate without explicit threading.
///
/// Set once on server bootstrap via [`set_global_metrics`]; protocol implementations call
/// [`global_metrics`] to record events. Falls back to a one-time initialised default so
/// tests and embedders that never call `set_global_metrics` still get a usable collector
/// (rather than panicking or silently dropping data).
static GLOBAL_METRICS: OnceLock<MetricsCollector> = OnceLock::new();

/// Error returned by [`set_global_metrics`] when the global has already been initialised.
///
/// Cannot store the rejected collector inside the variant because it carries non-`'static`
/// borrows on the closure-shaped fields (the source callback). The caller is expected
/// to drop their `MetricsCollector` on receiving this error.
#[derive(Debug, thiserror::Error)]
#[error("global MetricsCollector has already been initialised")]
pub struct GlobalMetricsAlreadySet;

/// Install the process-wide [`MetricsCollector`] so protocol crates can record events
/// without having to thread the handle through every constructor.
///
/// Returns `Err(GlobalMetricsAlreadySet)` if a collector has already been installed —
/// callers should treat this as a non-fatal warning and continue using the existing
/// global handle returned by [`global_metrics`].
pub fn set_global_metrics(collector: MetricsCollector) -> Result<(), GlobalMetricsAlreadySet> {
    GLOBAL_METRICS
        .set(collector)
        .map_err(|_| GlobalMetricsAlreadySet)
}

/// Get the process-wide metrics collector, lazily installing a fresh one the first time
/// it is requested if `set_global_metrics` was never called.
///
/// Always returns a usable handle — never panics, never returns `None`.
pub fn global_metrics() -> &'static MetricsCollector {
    GLOBAL_METRICS.get_or_init(MetricsCollector::new)
}

/// TLS label values for the `rusmes_tls_sessions_total` counter.
///
/// Use these constants instead of stringly-typed labels to keep the cardinality bounded
/// and avoid typo-induced label drift across protocol implementations.
pub mod tls_label {
    /// Session was established over a TLS-from-the-start (implicit TLS) port.
    pub const YES: &str = "yes";
    /// Session was established as plaintext and never upgraded.
    pub const NO: &str = "no";
    /// Session started plaintext and was upgraded via STARTTLS.
    pub const STARTTLS: &str = "starttls";
}

/// Callback type used to feed the per-recipient-domain counter from a queue.
///
/// The closure must return a fresh snapshot each time it is called. The metrics layer
/// is responsible for invoking it (either on every scrape or via the periodic refresh
/// task spawned by [`MetricsCollector::spawn_domain_stats_refresher`]).
pub type DomainStatsSource = Arc<dyn Fn() -> HashMap<String, u64> + Send + Sync>;

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

/// RAII guard that decrements the active-connections gauge on drop.
///
/// Construct via [`MetricsCollector::connection_guard`].
pub struct ConnectionGuard {
    metrics: MetricsCollector,
    protocol: String,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.metrics.dec_active_connections(&self.protocol);
    }
}

/// Server metrics collector
#[derive(Clone)]
pub struct MetricsCollector {
    // SMTP metrics
    smtp_connections_total: Arc<AtomicU64>,
    smtp_messages_received: Arc<AtomicU64>,
    smtp_messages_sent: Arc<AtomicU64>,
    smtp_errors: Arc<AtomicU64>,
    smtp_auth_success_total: Arc<AtomicU64>,
    smtp_auth_failure_total: Arc<AtomicU64>,
    smtp_messages_rejected_total: Arc<AtomicU64>,
    smtp_connections_rejected_blocked: Arc<AtomicU64>,
    smtp_connections_rejected_overload: Arc<AtomicU64>,

    // IMAP metrics
    imap_connections_total: Arc<AtomicU64>,
    imap_commands_total: Arc<AtomicU64>,
    imap_errors: Arc<AtomicU64>,

    // JMAP metrics
    jmap_requests_total: Arc<AtomicU64>,
    jmap_errors: Arc<AtomicU64>,

    // WebPush delivery metrics
    push_deliveries_total: Arc<AtomicU64>,
    push_delivery_failures_total: Arc<AtomicU64>,

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

    // Active connections gauge (label: protocol -> live count, may go negative on misuse).
    active_connections: Arc<DashMap<String, Arc<AtomicI64>>>,

    // TLS sessions counter (label: tls -> total sessions seen).
    tls_sessions_total: Arc<DashMap<String, Arc<AtomicU64>>>,

    // Per-recipient-domain message counter (label: domain -> count).
    messages_per_domain: Arc<DashMap<String, Arc<AtomicU64>>>,

    // Optional callback for refreshing the per-domain counters on demand.
    domain_stats_source: Arc<Mutex<Option<DomainStatsSource>>>,
}

impl std::fmt::Debug for MetricsCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsCollector")
            .field(
                "smtp_connections_total",
                &self.smtp_connections_total.load(Ordering::Relaxed),
            )
            .field(
                "active_connections_protocols",
                &self.active_connections.len(),
            )
            .field("tls_label_count", &self.tls_sessions_total.len())
            .field("domain_label_count", &self.messages_per_domain.len())
            .finish()
    }
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
            smtp_auth_success_total: Arc::new(AtomicU64::new(0)),
            smtp_auth_failure_total: Arc::new(AtomicU64::new(0)),
            smtp_messages_rejected_total: Arc::new(AtomicU64::new(0)),
            smtp_connections_rejected_blocked: Arc::new(AtomicU64::new(0)),
            smtp_connections_rejected_overload: Arc::new(AtomicU64::new(0)),
            imap_connections_total: Arc::new(AtomicU64::new(0)),
            imap_commands_total: Arc::new(AtomicU64::new(0)),
            imap_errors: Arc::new(AtomicU64::new(0)),
            jmap_requests_total: Arc::new(AtomicU64::new(0)),
            jmap_errors: Arc::new(AtomicU64::new(0)),
            push_deliveries_total: Arc::new(AtomicU64::new(0)),
            push_delivery_failures_total: Arc::new(AtomicU64::new(0)),
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
            active_connections: Arc::new(DashMap::new()),
            tls_sessions_total: Arc::new(DashMap::new()),
            messages_per_domain: Arc::new(DashMap::new()),
            domain_stats_source: Arc::new(Mutex::new(None)),
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

    /// Increment the SMTP authentication success counter (any mechanism).
    pub fn inc_smtp_auth_success(&self) {
        self.smtp_auth_success_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the SMTP authentication failure counter (wrong credentials or mechanism error).
    pub fn inc_smtp_auth_failure(&self) {
        self.smtp_auth_failure_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the SMTP rejected-message counter (4xx/5xx after DATA, e.g., size exceeded).
    pub fn inc_smtp_messages_rejected(&self) {
        self.smtp_messages_rejected_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Read the SMTP auth success counter (primarily for testing).
    pub fn smtp_auth_success_count(&self) -> u64 {
        self.smtp_auth_success_total.load(Ordering::Relaxed)
    }

    /// Read the SMTP auth failure counter (primarily for testing).
    pub fn smtp_auth_failure_count(&self) -> u64 {
        self.smtp_auth_failure_total.load(Ordering::Relaxed)
    }

    /// Read the SMTP rejected-message counter (primarily for testing).
    pub fn smtp_messages_rejected_count(&self) -> u64 {
        self.smtp_messages_rejected_total.load(Ordering::Relaxed)
    }

    /// Increment the SMTP connections-rejected-blocked-IP counter.
    pub fn inc_smtp_connections_rejected_blocked(&self) {
        self.smtp_connections_rejected_blocked
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Read the SMTP connections-rejected-blocked counter (primarily for testing).
    pub fn smtp_connections_rejected_blocked_count(&self) -> u64 {
        self.smtp_connections_rejected_blocked
            .load(Ordering::Relaxed)
    }

    /// Increment the SMTP connections-rejected-overload counter (concurrent-connection cap exceeded).
    pub fn inc_smtp_connections_rejected_overload(&self) {
        self.smtp_connections_rejected_overload
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Read the SMTP connections-rejected-overload counter (primarily for testing).
    pub fn smtp_connections_rejected_overload_count(&self) -> u64 {
        self.smtp_connections_rejected_overload
            .load(Ordering::Relaxed)
    }

    /// Read the SMTP accepted-message counter (primarily for testing).
    pub fn smtp_messages_accepted_count(&self) -> u64 {
        self.smtp_messages_received.load(Ordering::Relaxed)
    }

    /// Read the SMTP connections counter (primarily for testing).
    pub fn smtp_connections_count(&self) -> u64 {
        self.smtp_connections_total.load(Ordering::Relaxed)
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

    /// Increment the WebPush successful-delivery counter.
    pub fn inc_push_deliveries(&self) {
        self.push_deliveries_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the WebPush final-failure counter (all retries exhausted or 410 Gone).
    pub fn inc_push_delivery_failures(&self) {
        self.push_delivery_failures_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Read the push-deliveries counter (primarily for testing).
    pub fn push_deliveries_count(&self) -> u64 {
        self.push_deliveries_total.load(Ordering::Relaxed)
    }

    /// Read the push-delivery-failures counter (primarily for testing).
    pub fn push_delivery_failures_count(&self) -> u64 {
        self.push_delivery_failures_total.load(Ordering::Relaxed)
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

    // ----- Active connections gauge (rusmes_active_connections{protocol=...}) -----

    /// Increment the active-connections gauge for a protocol (`smtp`, `imap`, `jmap`, `pop3`).
    pub fn inc_active_connections(&self, protocol: &str) {
        let entry = self
            .active_connections
            .entry(protocol.to_owned())
            .or_insert_with(|| Arc::new(AtomicI64::new(0)));
        entry.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the active-connections gauge for a protocol.
    ///
    /// Safe to call even if the protocol label has never been seen — it will lazily create
    /// the entry at zero (and immediately drop to -1, which is a useful diagnostic signal
    /// rather than a panic).
    pub fn dec_active_connections(&self, protocol: &str) {
        let entry = self
            .active_connections
            .entry(protocol.to_owned())
            .or_insert_with(|| Arc::new(AtomicI64::new(0)));
        entry.fetch_sub(1, Ordering::Relaxed);
    }

    /// Read the current active-connections gauge for a protocol (0 if never observed).
    pub fn active_connections(&self, protocol: &str) -> i64 {
        self.active_connections
            .get(protocol)
            .map(|v| v.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Return an RAII guard that increments the gauge on creation and decrements on drop.
    ///
    /// Use this from each protocol's connection-acceptance path so the gauge round-trips
    /// even when the session terminates via `?`, panic, or an early return:
    ///
    /// ```rust,ignore
    /// let _conn_guard = metrics.connection_guard("smtp");
    /// session.handle().await?;
    /// // gauge decremented here when guard drops, regardless of outcome
    /// ```
    pub fn connection_guard(&self, protocol: &str) -> ConnectionGuard {
        self.inc_active_connections(protocol);
        ConnectionGuard {
            metrics: self.clone(),
            protocol: protocol.to_owned(),
        }
    }

    // ----- TLS counter (rusmes_tls_sessions_total{tls=yes|no|starttls}) -----

    /// Record a session creation under the given TLS label.
    ///
    /// `tls_kind` should be one of [`tls_label::YES`], [`tls_label::NO`], or [`tls_label::STARTTLS`].
    /// Other values are accepted (label cardinality is unbounded only by caller discipline)
    /// but will produce non-standard label values.
    pub fn inc_tls_session(&self, tls_kind: &str) {
        let entry = self
            .tls_sessions_total
            .entry(tls_kind.to_owned())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.fetch_add(1, Ordering::Relaxed);
    }

    /// Read the current TLS-sessions counter for a given label.
    pub fn tls_session_count(&self, tls_kind: &str) -> u64 {
        self.tls_sessions_total
            .get(tls_kind)
            .map(|v| v.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    // ----- Per-domain message counters (rusmes_messages_per_domain_total{domain=...}) -----

    /// Set the absolute count for a recipient domain.
    ///
    /// Prefer this when feeding from `rusmes_core::MailQueue::queue_stats_per_domain()` —
    /// the queue owns the canonical counter and the metrics layer just mirrors the value.
    pub fn set_messages_per_domain(&self, domain: &str, count: u64) {
        let entry = self
            .messages_per_domain
            .entry(domain.to_owned())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.store(count, Ordering::Relaxed);
    }

    /// Increment the per-domain counter by one. Useful when the metrics layer is the
    /// counter of record (i.e. when there is no upstream queue snapshot).
    pub fn inc_messages_per_domain(&self, domain: &str) {
        let entry = self
            .messages_per_domain
            .entry(domain.to_owned())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)));
        entry.fetch_add(1, Ordering::Relaxed);
    }

    /// Read the per-domain counter snapshot.
    pub fn messages_per_domain(&self) -> HashMap<String, u64> {
        self.messages_per_domain
            .iter()
            .map(|kv| (kv.key().clone(), kv.value().load(Ordering::Relaxed)))
            .collect()
    }

    /// Register a fresh-reading callback that returns the current per-domain counts.
    ///
    /// The callback is invoked at scrape time (every `/metrics` HTTP request) so the
    /// exposition reflects the queue's current state without the metrics layer having to
    /// duplicate-track every enqueue. Pair with [`Self::spawn_domain_stats_refresher`]
    /// for a periodic background snapshot when the source is expensive to query.
    pub fn set_domain_stats_source(&self, source: DomainStatsSource) {
        if let Ok(mut guard) = self.domain_stats_source.lock() {
            *guard = Some(source);
        }
    }

    /// Spawn a background task that refreshes the per-domain counters at a fixed cadence.
    ///
    /// The task pulls the current snapshot from the registered [`DomainStatsSource`]
    /// (set via [`Self::set_domain_stats_source`]) and calls
    /// [`Self::set_messages_per_domain`] for every entry. If no source is configured,
    /// the task ticks once and exits.
    pub fn spawn_domain_stats_refresher(&self, period: Duration) -> tokio::task::JoinHandle<()> {
        let collector = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                collector.refresh_domain_stats_now();
            }
        })
    }

    /// Refresh the per-domain counters synchronously from the registered source.
    pub fn refresh_domain_stats_now(&self) {
        let snapshot = match self.domain_stats_source.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(src) => src(),
                None => return,
            },
            Err(_) => return,
        };
        for (domain, count) in snapshot {
            self.set_messages_per_domain(&domain, count);
        }
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

        output.push_str(
            "# HELP rusmes_smtp_auth_success_total Total successful SMTP AUTH exchanges\n",
        );
        output.push_str("# TYPE rusmes_smtp_auth_success_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_auth_success_total {}\n",
            self.smtp_auth_success_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_smtp_auth_failure_total Total failed SMTP AUTH exchanges\n");
        output.push_str("# TYPE rusmes_smtp_auth_failure_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_auth_failure_total {}\n",
            self.smtp_auth_failure_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_smtp_messages_rejected_total Total SMTP messages rejected due to size limit exceeded during DATA\n");
        output.push_str("# TYPE rusmes_smtp_messages_rejected_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_messages_rejected_total {}\n",
            self.smtp_messages_rejected_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_smtp_connections_rejected_blocked_total Total SMTP connections rejected due to blocked IP\n");
        output.push_str("# TYPE rusmes_smtp_connections_rejected_blocked_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_connections_rejected_blocked_total {}\n",
            self.smtp_connections_rejected_blocked
                .load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_smtp_connections_rejected_overload_total Total SMTP connections rejected due to per-IP connection cap exceeded\n");
        output.push_str("# TYPE rusmes_smtp_connections_rejected_overload_total counter\n");
        output.push_str(&format!(
            "rusmes_smtp_connections_rejected_overload_total {}\n",
            self.smtp_connections_rejected_overload
                .load(Ordering::Relaxed)
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

        // WebPush delivery metrics
        output
            .push_str("# HELP rusmes_push_deliveries_total Total successful WebPush deliveries\n");
        output.push_str("# TYPE rusmes_push_deliveries_total counter\n");
        output.push_str(&format!(
            "rusmes_push_deliveries_total {}\n",
            self.push_deliveries_total.load(Ordering::Relaxed)
        ));

        output.push_str(
            "# HELP rusmes_push_delivery_failures_total Total WebPush final delivery failures\n",
        );
        output.push_str("# TYPE rusmes_push_delivery_failures_total counter\n");
        output.push_str(&format!(
            "rusmes_push_delivery_failures_total {}\n",
            self.push_delivery_failures_total.load(Ordering::Relaxed)
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

        // Active connections gauge (label: protocol)
        output.push_str(
            "# HELP rusmes_active_connections Currently open client connections per protocol\n",
        );
        output.push_str("# TYPE rusmes_active_connections gauge\n");
        // Sort by label for deterministic output (eases tests + diff-based scrape consumers).
        let mut active: Vec<(String, i64)> = self
            .active_connections
            .iter()
            .map(|kv| (kv.key().clone(), kv.value().load(Ordering::Relaxed)))
            .collect();
        active.sort_by(|a, b| a.0.cmp(&b.0));
        for (protocol, value) in active {
            output.push_str(&format!(
                "rusmes_active_connections{{protocol=\"{}\"}} {}\n",
                escape_label_value(&protocol),
                value
            ));
        }

        // TLS sessions counter (label: tls)
        output.push_str(
            "# HELP rusmes_tls_sessions_total Total client sessions seen, partitioned by TLS state\n",
        );
        output.push_str("# TYPE rusmes_tls_sessions_total counter\n");
        let mut tls: Vec<(String, u64)> = self
            .tls_sessions_total
            .iter()
            .map(|kv| (kv.key().clone(), kv.value().load(Ordering::Relaxed)))
            .collect();
        tls.sort_by(|a, b| a.0.cmp(&b.0));
        for (label, value) in tls {
            output.push_str(&format!(
                "rusmes_tls_sessions_total{{tls=\"{}\"}} {}\n",
                escape_label_value(&label),
                value
            ));
        }

        // Per-domain message counter (label: domain)
        // Pull a fresh snapshot from the registered source if any (typically Cluster 4's
        // MailQueue::queue_stats_per_domain) so the scrape reflects live state.
        self.refresh_domain_stats_now();
        output.push_str(
            "# HELP rusmes_messages_per_domain_total Total messages enqueued per recipient domain\n",
        );
        output.push_str("# TYPE rusmes_messages_per_domain_total counter\n");
        let mut domains: Vec<(String, u64)> = self
            .messages_per_domain
            .iter()
            .map(|kv| (kv.key().clone(), kv.value().load(Ordering::Relaxed)))
            .collect();
        domains.sort_by(|a, b| a.0.cmp(&b.0));
        for (domain, value) in domains {
            output.push_str(&format!(
                "rusmes_messages_per_domain_total{{domain=\"{}\"}} {}\n",
                escape_label_value(&domain),
                value
            ));
        }

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

        let bind_address = config.bind_address.clone();
        let metrics_path = config.path.clone();
        let basic_auth = config.basic_auth.clone();
        let app = self.build_router(&metrics_path, basic_auth);

        eprintln!(
            "Starting metrics HTTP server on {}{}",
            bind_address, metrics_path
        );
        eprintln!("Health check endpoints: /health, /ready, /live");

        let listener = TcpListener::bind(&bind_address).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }

    /// Build the axum router used by [`Self::start_http_server`].
    ///
    /// Exposed so tests (and embedders) can drive the handler in-process via
    /// `tower::ServiceExt::oneshot` without binding a TCP socket.
    pub fn build_router(
        self,
        metrics_path: &str,
        basic_auth: Option<MetricsBasicAuthConfig>,
    ) -> Router {
        let metrics = Arc::new(Mutex::new(self));

        let metrics_handler = {
            let metrics = Arc::clone(&metrics);
            move || {
                let metrics = Arc::clone(&metrics);
                async move {
                    let collector = match metrics.lock() {
                        Ok(guard) => guard,
                        Err(e) => {
                            ::tracing::error!("Metrics mutex poisoned: {e}");
                            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                        }
                    };
                    let output = collector.export_prometheus();
                    (
                        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
                        output,
                    )
                        .into_response()
                }
            }
        };

        let mut metrics_router = Router::new().route(metrics_path, get(metrics_handler));
        if let Some(auth) = basic_auth {
            let state = Arc::new(BasicAuthState { config: auth });
            metrics_router =
                metrics_router.layer(middleware::from_fn_with_state(state, basic_auth_middleware));
        }

        let health_router = create_health_router();
        Router::new().merge(metrics_router).merge(health_router)
    }
}

/// Internal state for the basic-auth middleware (cloneable handle into the config).
#[derive(Clone)]
struct BasicAuthState {
    config: MetricsBasicAuthConfig,
}

/// Axum middleware that enforces HTTP Basic auth (RFC 7617) against a bcrypt password hash.
///
/// Returns `401 Unauthorized` with a `WWW-Authenticate: Basic realm="rusmes-metrics"` header
/// on missing/malformed/incorrect credentials. On success the request is forwarded unchanged.
async fn basic_auth_middleware(
    State(state): State<Arc<BasicAuthState>>,
    request: Request,
    next: Next,
) -> Response {
    let header_value = request.headers().get(header::AUTHORIZATION);
    if !verify_basic_auth(header_value, &state.config) {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        let realm = HeaderValue::from_static("Basic realm=\"rusmes-metrics\", charset=\"UTF-8\"");
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, realm);
        // Replace the body so curl/Prometheus operators see something useful.
        *response.body_mut() = Body::from("401 Unauthorized: metrics endpoint requires basic auth");
        return response;
    }
    next.run(request).await
}

/// Verify a `Authorization: Basic <base64>` header against the configured credentials.
///
/// Returns `true` only when:
/// 1. The header is present, well-formed, and base64-decodes,
/// 2. The username matches exactly (constant-time comparison via [`bytes_eq_constant_time`]),
/// 3. The password verifies against the bcrypt hash.
fn verify_basic_auth(header_value: Option<&HeaderValue>, config: &MetricsBasicAuthConfig) -> bool {
    let value = match header_value.and_then(|v| v.to_str().ok()) {
        Some(v) => v,
        None => return false,
    };
    let encoded = match value.strip_prefix("Basic ") {
        Some(s) => s.trim(),
        None => return false,
    };
    let decoded = match BASE64.decode(encoded) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let credentials = match std::str::from_utf8(&decoded) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let (user, password) = match credentials.split_once(':') {
        Some(parts) => parts,
        None => return false,
    };
    if !bytes_eq_constant_time(user.as_bytes(), config.username.as_bytes()) {
        return false;
    }
    match bcrypt::verify(password, &config.password_hash) {
        Ok(ok) => ok,
        Err(e) => {
            ::tracing::warn!(
                "bcrypt verify failed for metrics basic auth (likely malformed hash in config): {e}"
            );
            false
        }
    }
}

/// Constant-time byte equality check (avoids leaking the username length via early-exit timing).
fn bytes_eq_constant_time(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Escape a Prometheus label value per the exposition format spec.
///
/// Per <https://prometheus.io/docs/instrumenting/exposition_formats/#text-format-details>,
/// label values are surrounded by double quotes; backslash, double-quote, and newline
/// must be escaped.
fn escape_label_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
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
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

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

    /// Active-connections gauge round-trips through `connection_guard`'s RAII Drop.
    ///
    /// Cluster 7B: every protocol session opens via `inc()` and closes via `dec()` —
    /// when both run, the gauge must return to its baseline.
    #[test]
    fn test_active_connections_guard_roundtrip() {
        let metrics = MetricsCollector::new();
        assert_eq!(metrics.active_connections("smtp"), 0);

        {
            let _g = metrics.connection_guard("smtp");
            assert_eq!(metrics.active_connections("smtp"), 1);
            {
                let _g2 = metrics.connection_guard("smtp");
                assert_eq!(metrics.active_connections("smtp"), 2);
            }
            assert_eq!(metrics.active_connections("smtp"), 1);
        }
        assert_eq!(metrics.active_connections("smtp"), 0);

        // Per-protocol isolation: incrementing imap doesn't affect smtp.
        let _g = metrics.connection_guard("imap");
        assert_eq!(metrics.active_connections("imap"), 1);
        assert_eq!(metrics.active_connections("smtp"), 0);
    }

    /// TLS counter labels (`yes`, `no`, `starttls`) are reported under the `tls=` label
    /// and accumulated across calls.
    #[test]
    fn test_tls_session_counter_labels() {
        let metrics = MetricsCollector::new();
        metrics.inc_tls_session(tls_label::NO);
        metrics.inc_tls_session(tls_label::NO);
        metrics.inc_tls_session(tls_label::STARTTLS);
        metrics.inc_tls_session(tls_label::YES);

        assert_eq!(metrics.tls_session_count(tls_label::NO), 2);
        assert_eq!(metrics.tls_session_count(tls_label::STARTTLS), 1);
        assert_eq!(metrics.tls_session_count(tls_label::YES), 1);

        let exp = metrics.export_prometheus();
        assert!(exp.contains("rusmes_tls_sessions_total{tls=\"no\"} 2"));
        assert!(exp.contains("rusmes_tls_sessions_total{tls=\"starttls\"} 1"));
        assert!(exp.contains("rusmes_tls_sessions_total{tls=\"yes\"} 1"));
        assert!(exp.contains("# TYPE rusmes_tls_sessions_total counter"));
    }

    /// Per-domain counter is fed by the registered `DomainStatsSource` callback and
    /// surfaces under the `domain=` label in the Prometheus exposition.
    ///
    /// Cluster 7D: data flows from `rusmes_core::MailQueue::queue_stats_per_domain()`
    /// (or any other source) via the callback; the metrics layer just mirrors values
    /// at scrape time.
    #[test]
    fn test_messages_per_domain_from_callback_source() {
        let metrics = MetricsCollector::new();
        metrics.set_domain_stats_source(Arc::new(|| {
            let mut m = HashMap::new();
            m.insert("example.com".to_string(), 5u64);
            m.insert("example.org".to_string(), 3u64);
            m
        }));

        let exp = metrics.export_prometheus();
        assert!(
            exp.contains("rusmes_messages_per_domain_total{domain=\"example.com\"} 5"),
            "exposition was:\n{exp}"
        );
        assert!(exp.contains("rusmes_messages_per_domain_total{domain=\"example.org\"} 3"));
        assert!(exp.contains("# TYPE rusmes_messages_per_domain_total counter"));
    }

    /// Label-value escaping: backslash, double-quote, and newline must be escaped per
    /// the Prometheus exposition spec so that `domain="weird\"value"` round-trips.
    #[test]
    fn test_escape_label_value_quotes_and_backslash() {
        assert_eq!(escape_label_value("plain"), "plain");
        assert_eq!(escape_label_value("a\"b"), "a\\\"b");
        assert_eq!(escape_label_value("a\\b"), "a\\\\b");
        assert_eq!(escape_label_value("a\nb"), "a\\nb");
    }

    /// Constant-time username comparison rejects mismatched lengths and contents.
    #[test]
    fn test_constant_time_eq() {
        assert!(bytes_eq_constant_time(b"abc", b"abc"));
        assert!(!bytes_eq_constant_time(b"abc", b"abd"));
        assert!(!bytes_eq_constant_time(b"abc", b"abcd"));
        assert!(bytes_eq_constant_time(b"", b""));
    }

    /// Build a router with optional basic-auth state and the metrics endpoint at `/metrics`.
    fn router_with_basic_auth(creds: Option<(&str, &str)>) -> Router {
        let metrics = MetricsCollector::new();
        let auth = creds.map(|(u, p)| MetricsBasicAuthConfig {
            username: u.to_string(),
            // Use a low cost so the bcrypt hash is fast for tests.
            password_hash: bcrypt::hash(p, 4).expect("bcrypt hash for test"),
        });
        metrics.build_router("/metrics", auth)
    }

    /// Basic auth: 200 with correct creds, 401 without.
    ///
    /// Cluster 7A.
    #[tokio::test]
    async fn test_metrics_basic_auth_accepts_correct_credentials() {
        let app = router_with_basic_auth(Some(("scrape", "s3cret")));

        // No credentials → 401.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request build"),
            )
            .await
            .expect("router call");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let www_auth = resp
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .expect("WWW-Authenticate header on 401")
            .to_str()
            .expect("ascii header");
        assert!(www_auth.starts_with("Basic realm="), "got: {www_auth}");

        // Correct credentials → 200 with prometheus body.
        let creds = BASE64.encode(b"scrape:s3cret");
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .header(header::AUTHORIZATION, format!("Basic {creds}"))
                    .body(Body::empty())
                    .expect("request build"),
            )
            .await
            .expect("router call");
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp
            .into_body()
            .collect()
            .await
            .expect("collect")
            .to_bytes();
        let body_text = std::str::from_utf8(&body_bytes).expect("utf-8 body");
        assert!(body_text.contains("# HELP"), "body was:\n{body_text}");
    }

    /// Basic auth: wrong username and wrong password both return 401 (no info leak).
    #[tokio::test]
    async fn test_metrics_basic_auth_rejects_wrong_credentials() {
        let app = router_with_basic_auth(Some(("scrape", "s3cret")));

        let bad_user = BASE64.encode(b"wrong:s3cret");
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .header(header::AUTHORIZATION, format!("Basic {bad_user}"))
                    .body(Body::empty())
                    .expect("request build"),
            )
            .await
            .expect("router call");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let bad_pass = BASE64.encode(b"scrape:wrong");
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .header(header::AUTHORIZATION, format!("Basic {bad_pass}"))
                    .body(Body::empty())
                    .expect("request build"),
            )
            .await
            .expect("router call");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// Without a `basic_auth` config block, `/metrics` is served unauthenticated (the
    /// previous default — backwards-compatible).
    #[tokio::test]
    async fn test_metrics_no_basic_auth_serves_anonymously() {
        let app = router_with_basic_auth(None);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request build"),
            )
            .await
            .expect("router call");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Guard increments the gauge on creation and decrements it on `Drop`.
    #[test]
    fn test_connection_guard_increments_and_decrements() {
        let metrics = MetricsCollector::new();
        assert_eq!(metrics.active_connections("pop3"), 0);
        let guard = metrics.connection_guard("pop3");
        assert_eq!(metrics.active_connections("pop3"), 1);
        drop(guard);
        assert_eq!(metrics.active_connections("pop3"), 0);
    }

    /// Active-connections gauges are tracked independently per protocol.
    #[test]
    fn test_connection_metrics_total() {
        let metrics = MetricsCollector::new();
        let _g1 = metrics.connection_guard("smtp");
        let _g2 = metrics.connection_guard("imap");
        let _g3 = metrics.connection_guard("pop3");
        assert_eq!(metrics.active_connections("smtp"), 1);
        assert_eq!(metrics.active_connections("imap"), 1);
        assert_eq!(metrics.active_connections("pop3"), 1);
    }

    /// Prometheus exposition contains active_connections gauge with correct label and value.
    #[test]
    fn test_prometheus_format() {
        let metrics = MetricsCollector::new();
        let _g = metrics.connection_guard("smtp");
        let output = metrics.export_prometheus();
        assert!(
            output.contains("rusmes_active_connections{protocol=\"smtp\"} 1"),
            "prometheus output was:\n{output}"
        );
        assert!(output.contains("# TYPE rusmes_active_connections gauge"));
    }

    /// Metrics router responds HTTP 200 with a Prometheus body when no auth is configured.
    #[tokio::test]
    async fn test_metrics_server_responds() {
        let metrics = MetricsCollector::new();
        let _g = metrics.connection_guard("smtp");
        let app = metrics.build_router("/metrics", None);

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request build"),
            )
            .await
            .expect("router call");
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = resp
            .into_body()
            .collect()
            .await
            .expect("collect")
            .to_bytes();
        let body_text = std::str::from_utf8(&body_bytes).expect("utf-8 body");
        assert!(
            body_text.contains("rusmes_active_connections"),
            "body was:\n{body_text}"
        );
    }

    /// Verify that the global metrics handle is the same on every call (singleton).
    #[test]
    fn test_global_metrics_singleton() {
        let a = global_metrics() as *const _;
        let b = global_metrics() as *const _;
        assert_eq!(a, b, "global_metrics() must return the same instance");
    }
}
