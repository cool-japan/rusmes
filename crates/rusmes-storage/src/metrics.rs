//! Storage metrics collection and tracking
//!
//! This module provides comprehensive metrics tracking for storage operations including:
//! - Disk usage tracking (per user, per mailbox, total)
//! - Message counts (total, per mailbox, per state)
//! - Operation latency histograms (append, fetch, delete, search)
//! - Storage backend health metrics
//! - Integration with Prometheus metrics
//!
//! ## Usage
//!
//! ```rust,no_run
//! use rusmes_storage::metrics::StorageMetrics;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let metrics = StorageMetrics::new();
//!
//! // Track message operations
//! metrics.inc_messages_total(1);
//! metrics.add_disk_usage_bytes(1024);
//!
//! // Track operation latency
//! let timer = metrics.start_append_timer();
//! // ... perform append operation ...
//! timer.observe();
//!
//! // Export to Prometheus format
//! let prometheus_output = metrics.export_prometheus();
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// Histogram bucket for tracking latency distributions
#[derive(Debug, Clone)]
pub struct Histogram {
    buckets: Vec<f64>,
    counts: Vec<Arc<AtomicU64>>,
    sum: Arc<AtomicU64>,
    count: Arc<AtomicU64>,
}

impl Histogram {
    /// Create a new histogram with specified bucket boundaries (in seconds)
    pub fn new(buckets: Vec<f64>) -> Self {
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

    /// Observe a value (in seconds)
    pub fn observe(&self, value: f64) {
        let millis = (value * 1000.0) as u64;
        self.sum.fetch_add(millis, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        for (i, &bucket) in self.buckets.iter().enumerate() {
            if value <= bucket {
                self.counts[i].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Export histogram in Prometheus format
    pub fn export(&self, name: &str, help: &str) -> String {
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

    /// Get total count of observations
    pub fn get_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Get sum of all observations (in milliseconds)
    pub fn get_sum(&self) -> u64 {
        self.sum.load(Ordering::Relaxed)
    }

    /// Calculate average (in seconds)
    pub fn average(&self) -> f64 {
        let count = self.get_count();
        if count == 0 {
            0.0
        } else {
            (self.get_sum() as f64 / 1000.0) / count as f64
        }
    }
}

/// Timer for tracking operation duration
pub struct StorageTimer {
    start: Instant,
    histogram: Arc<Histogram>,
}

impl StorageTimer {
    fn new(histogram: Arc<Histogram>) -> Self {
        Self {
            start: Instant::now(),
            histogram,
        }
    }

    /// Observe and record the elapsed time
    pub fn observe(self) {
        let duration = self.start.elapsed().as_secs_f64();
        self.histogram.observe(duration);
    }

    /// Get elapsed time without recording
    pub fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

/// Storage metrics collector
#[derive(Debug, Clone)]
pub struct StorageMetrics {
    // Message counts
    messages_total: Arc<AtomicU64>,
    messages_deleted: Arc<AtomicU64>,
    messages_flagged: Arc<AtomicU64>,
    messages_seen: Arc<AtomicU64>,
    messages_unseen: Arc<AtomicU64>,

    // Mailbox counts
    mailboxes_total: Arc<AtomicU64>,
    mailboxes_created: Arc<AtomicU64>,
    mailboxes_deleted: Arc<AtomicU64>,

    // Disk usage (in bytes)
    disk_usage_total_bytes: Arc<AtomicU64>,

    // Per-user disk usage (in bytes)
    disk_usage_per_user: Arc<RwLock<HashMap<String, Arc<AtomicU64>>>>,

    // Per-mailbox message counts
    messages_per_mailbox: Arc<RwLock<HashMap<String, Arc<AtomicU64>>>>,

    // Operation counters
    append_operations_total: Arc<AtomicU64>,
    fetch_operations_total: Arc<AtomicU64>,
    delete_operations_total: Arc<AtomicU64>,
    search_operations_total: Arc<AtomicU64>,
    copy_operations_total: Arc<AtomicU64>,

    // Operation error counters
    append_errors_total: Arc<AtomicU64>,
    fetch_errors_total: Arc<AtomicU64>,
    delete_errors_total: Arc<AtomicU64>,
    search_errors_total: Arc<AtomicU64>,

    // Backend health
    backend_healthy: Arc<AtomicU64>,
    backend_last_check: Arc<AtomicU64>,

    // Operation latency histograms
    append_latency: Arc<Histogram>,
    fetch_latency: Arc<Histogram>,
    delete_latency: Arc<Histogram>,
    search_latency: Arc<Histogram>,
}

impl Default for StorageMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageMetrics {
    /// Create a new storage metrics collector
    pub fn new() -> Self {
        // Define histogram buckets for operation latency (in seconds)
        // Optimized for storage operations: 1ms to 10s
        let latency_buckets = vec![
            0.001, 0.002, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ];

        Self {
            messages_total: Arc::new(AtomicU64::new(0)),
            messages_deleted: Arc::new(AtomicU64::new(0)),
            messages_flagged: Arc::new(AtomicU64::new(0)),
            messages_seen: Arc::new(AtomicU64::new(0)),
            messages_unseen: Arc::new(AtomicU64::new(0)),
            mailboxes_total: Arc::new(AtomicU64::new(0)),
            mailboxes_created: Arc::new(AtomicU64::new(0)),
            mailboxes_deleted: Arc::new(AtomicU64::new(0)),
            disk_usage_total_bytes: Arc::new(AtomicU64::new(0)),
            disk_usage_per_user: Arc::new(RwLock::new(HashMap::new())),
            messages_per_mailbox: Arc::new(RwLock::new(HashMap::new())),
            append_operations_total: Arc::new(AtomicU64::new(0)),
            fetch_operations_total: Arc::new(AtomicU64::new(0)),
            delete_operations_total: Arc::new(AtomicU64::new(0)),
            search_operations_total: Arc::new(AtomicU64::new(0)),
            copy_operations_total: Arc::new(AtomicU64::new(0)),
            append_errors_total: Arc::new(AtomicU64::new(0)),
            fetch_errors_total: Arc::new(AtomicU64::new(0)),
            delete_errors_total: Arc::new(AtomicU64::new(0)),
            search_errors_total: Arc::new(AtomicU64::new(0)),
            backend_healthy: Arc::new(AtomicU64::new(1)),
            backend_last_check: Arc::new(AtomicU64::new(0)),
            append_latency: Arc::new(Histogram::new(latency_buckets.clone())),
            fetch_latency: Arc::new(Histogram::new(latency_buckets.clone())),
            delete_latency: Arc::new(Histogram::new(latency_buckets.clone())),
            search_latency: Arc::new(Histogram::new(latency_buckets)),
        }
    }

    // Message count metrics

    /// Increment total message count
    pub fn inc_messages_total(&self, count: u64) {
        self.messages_total.fetch_add(count, Ordering::Relaxed);
    }

    /// Decrement total message count
    pub fn dec_messages_total(&self, count: u64) {
        self.messages_total.fetch_sub(count, Ordering::Relaxed);
    }

    /// Set total message count
    pub fn set_messages_total(&self, count: u64) {
        self.messages_total.store(count, Ordering::Relaxed);
    }

    /// Get total message count
    pub fn get_messages_total(&self) -> u64 {
        self.messages_total.load(Ordering::Relaxed)
    }

    /// Increment deleted message count
    pub fn inc_messages_deleted(&self, count: u64) {
        self.messages_deleted.fetch_add(count, Ordering::Relaxed);
    }

    /// Increment flagged message count
    pub fn inc_messages_flagged(&self) {
        self.messages_flagged.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement flagged message count
    pub fn dec_messages_flagged(&self) {
        self.messages_flagged.fetch_sub(1, Ordering::Relaxed);
    }

    /// Increment seen message count
    pub fn inc_messages_seen(&self) {
        self.messages_seen.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement seen message count
    pub fn dec_messages_seen(&self) {
        self.messages_seen.fetch_sub(1, Ordering::Relaxed);
    }

    /// Increment unseen message count
    pub fn inc_messages_unseen(&self) {
        self.messages_unseen.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement unseen message count
    pub fn dec_messages_unseen(&self) {
        self.messages_unseen.fetch_sub(1, Ordering::Relaxed);
    }

    // Mailbox count metrics

    /// Increment mailbox count
    pub fn inc_mailboxes_total(&self) {
        self.mailboxes_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement mailbox count
    pub fn dec_mailboxes_total(&self) {
        self.mailboxes_total.fetch_sub(1, Ordering::Relaxed);
    }

    /// Set mailbox count
    pub fn set_mailboxes_total(&self, count: u64) {
        self.mailboxes_total.store(count, Ordering::Relaxed);
    }

    /// Increment mailbox created counter
    pub fn inc_mailboxes_created(&self) {
        self.mailboxes_created.fetch_add(1, Ordering::Relaxed);
        self.inc_mailboxes_total();
    }

    /// Increment mailbox deleted counter
    pub fn inc_mailboxes_deleted(&self) {
        self.mailboxes_deleted.fetch_add(1, Ordering::Relaxed);
        self.dec_mailboxes_total();
    }

    // Disk usage metrics

    /// Add to total disk usage
    pub fn add_disk_usage_bytes(&self, bytes: u64) {
        self.disk_usage_total_bytes
            .fetch_add(bytes, Ordering::Relaxed);
    }

    /// Subtract from total disk usage
    pub fn sub_disk_usage_bytes(&self, bytes: u64) {
        self.disk_usage_total_bytes
            .fetch_sub(bytes, Ordering::Relaxed);
    }

    /// Set total disk usage
    pub fn set_disk_usage_bytes(&self, bytes: u64) {
        self.disk_usage_total_bytes.store(bytes, Ordering::Relaxed);
    }

    /// Get total disk usage
    pub fn get_disk_usage_bytes(&self) -> u64 {
        self.disk_usage_total_bytes.load(Ordering::Relaxed)
    }

    /// Set disk usage for a specific user
    pub fn set_user_disk_usage(&self, user: &str, bytes: u64) {
        if let Ok(mut map) = self.disk_usage_per_user.write() {
            map.entry(user.to_string())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .store(bytes, Ordering::Relaxed);
        }
    }

    /// Add to disk usage for a specific user
    pub fn add_user_disk_usage(&self, user: &str, bytes: u64) {
        let found = if let Ok(map) = self.disk_usage_per_user.read() {
            if let Some(counter) = map.get(user) {
                counter.fetch_add(bytes, Ordering::Relaxed);
                true
            } else {
                false
            }
        } else {
            false
        };
        if !found {
            self.set_user_disk_usage(user, bytes);
        }
        self.add_disk_usage_bytes(bytes);
    }

    /// Subtract from disk usage for a specific user
    pub fn sub_user_disk_usage(&self, user: &str, bytes: u64) {
        if let Ok(map) = self.disk_usage_per_user.read() {
            if let Some(counter) = map.get(user) {
                counter.fetch_sub(bytes, Ordering::Relaxed);
            }
        }
        self.sub_disk_usage_bytes(bytes);
    }

    /// Get disk usage for a specific user
    pub fn get_user_disk_usage(&self, user: &str) -> u64 {
        self.disk_usage_per_user
            .read()
            .ok()
            .and_then(|map| map.get(user).map(|c| c.load(Ordering::Relaxed)))
            .unwrap_or(0)
    }

    // Per-mailbox message counts

    /// Set message count for a specific mailbox
    pub fn set_mailbox_message_count(&self, mailbox_id: &str, count: u64) {
        if let Ok(mut map) = self.messages_per_mailbox.write() {
            map.entry(mailbox_id.to_string())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .store(count, Ordering::Relaxed);
        }
    }

    /// Increment message count for a specific mailbox
    pub fn inc_mailbox_message_count(&self, mailbox_id: &str, count: u64) {
        let found = if let Ok(map) = self.messages_per_mailbox.read() {
            if let Some(counter) = map.get(mailbox_id) {
                counter.fetch_add(count, Ordering::Relaxed);
                true
            } else {
                false
            }
        } else {
            false
        };
        if !found {
            self.set_mailbox_message_count(mailbox_id, count);
        }
    }

    /// Decrement message count for a specific mailbox
    pub fn dec_mailbox_message_count(&self, mailbox_id: &str, count: u64) {
        if let Ok(map) = self.messages_per_mailbox.read() {
            if let Some(counter) = map.get(mailbox_id) {
                counter.fetch_sub(count, Ordering::Relaxed);
            }
        }
    }

    /// Get message count for a specific mailbox
    pub fn get_mailbox_message_count(&self, mailbox_id: &str) -> u64 {
        self.messages_per_mailbox
            .read()
            .ok()
            .and_then(|map| map.get(mailbox_id).map(|c| c.load(Ordering::Relaxed)))
            .unwrap_or(0)
    }

    // Operation counters

    /// Increment append operation counter
    pub fn inc_append_operations(&self) {
        self.append_operations_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment fetch operation counter
    pub fn inc_fetch_operations(&self) {
        self.fetch_operations_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment delete operation counter
    pub fn inc_delete_operations(&self) {
        self.delete_operations_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment search operation counter
    pub fn inc_search_operations(&self) {
        self.search_operations_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment copy operation counter
    pub fn inc_copy_operations(&self) {
        self.copy_operations_total.fetch_add(1, Ordering::Relaxed);
    }

    // Error counters

    /// Increment append error counter
    pub fn inc_append_errors(&self) {
        self.append_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment fetch error counter
    pub fn inc_fetch_errors(&self) {
        self.fetch_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment delete error counter
    pub fn inc_delete_errors(&self) {
        self.delete_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment search error counter
    pub fn inc_search_errors(&self) {
        self.search_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    // Backend health metrics

    /// Set backend healthy status (1 = healthy, 0 = unhealthy)
    pub fn set_backend_healthy(&self, healthy: bool) {
        self.backend_healthy
            .store(if healthy { 1 } else { 0 }, Ordering::Relaxed);
    }

    /// Get backend healthy status
    pub fn is_backend_healthy(&self) -> bool {
        self.backend_healthy.load(Ordering::Relaxed) == 1
    }

    /// Update last health check timestamp (Unix timestamp in seconds)
    pub fn update_health_check_time(&self, timestamp: u64) {
        self.backend_last_check.store(timestamp, Ordering::Relaxed);
    }

    /// Get last health check timestamp
    pub fn get_last_health_check(&self) -> u64 {
        self.backend_last_check.load(Ordering::Relaxed)
    }

    // Latency timing

    /// Start timer for append operation
    pub fn start_append_timer(&self) -> StorageTimer {
        StorageTimer::new(Arc::clone(&self.append_latency))
    }

    /// Start timer for fetch operation
    pub fn start_fetch_timer(&self) -> StorageTimer {
        StorageTimer::new(Arc::clone(&self.fetch_latency))
    }

    /// Start timer for delete operation
    pub fn start_delete_timer(&self) -> StorageTimer {
        StorageTimer::new(Arc::clone(&self.delete_latency))
    }

    /// Start timer for search operation
    pub fn start_search_timer(&self) -> StorageTimer {
        StorageTimer::new(Arc::clone(&self.search_latency))
    }

    // Helper methods for common operations

    /// Record successful append operation with message size
    pub fn record_append_success(&self, size_bytes: u64) {
        self.inc_append_operations();
        self.inc_messages_total(1);
        self.add_disk_usage_bytes(size_bytes);
    }

    /// Record failed append operation
    pub fn record_append_failure(&self) {
        self.inc_append_errors();
    }

    /// Record successful delete operation with message size
    pub fn record_delete_success(&self, size_bytes: u64, count: u64) {
        self.inc_delete_operations();
        self.dec_messages_total(count);
        self.inc_messages_deleted(count);
        self.sub_disk_usage_bytes(size_bytes);
    }

    /// Record failed delete operation
    pub fn record_delete_failure(&self) {
        self.inc_delete_errors();
    }

    /// Record successful fetch operation
    pub fn record_fetch_success(&self) {
        self.inc_fetch_operations();
    }

    /// Record failed fetch operation
    pub fn record_fetch_failure(&self) {
        self.inc_fetch_errors();
    }

    /// Record successful search operation
    pub fn record_search_success(&self) {
        self.inc_search_operations();
    }

    /// Record failed search operation
    pub fn record_search_failure(&self) {
        self.inc_search_errors();
    }

    /// Export metrics in Prometheus text format
    pub fn export_prometheus(&self) -> String {
        let mut output = String::new();

        // Message count metrics
        output
            .push_str("# HELP rusmes_storage_messages_total Total number of messages in storage\n");
        output.push_str("# TYPE rusmes_storage_messages_total gauge\n");
        output.push_str(&format!(
            "rusmes_storage_messages_total {}\n",
            self.messages_total.load(Ordering::Relaxed)
        ));

        output.push_str(
            "# HELP rusmes_storage_messages_deleted_total Total number of deleted messages\n",
        );
        output.push_str("# TYPE rusmes_storage_messages_deleted_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_messages_deleted_total {}\n",
            self.messages_deleted.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_messages_flagged Number of flagged messages\n");
        output.push_str("# TYPE rusmes_storage_messages_flagged gauge\n");
        output.push_str(&format!(
            "rusmes_storage_messages_flagged {}\n",
            self.messages_flagged.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_messages_seen Number of seen messages\n");
        output.push_str("# TYPE rusmes_storage_messages_seen gauge\n");
        output.push_str(&format!(
            "rusmes_storage_messages_seen {}\n",
            self.messages_seen.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_messages_unseen Number of unseen messages\n");
        output.push_str("# TYPE rusmes_storage_messages_unseen gauge\n");
        output.push_str(&format!(
            "rusmes_storage_messages_unseen {}\n",
            self.messages_unseen.load(Ordering::Relaxed)
        ));

        // Mailbox count metrics
        output.push_str("# HELP rusmes_storage_mailboxes_total Total number of mailboxes\n");
        output.push_str("# TYPE rusmes_storage_mailboxes_total gauge\n");
        output.push_str(&format!(
            "rusmes_storage_mailboxes_total {}\n",
            self.mailboxes_total.load(Ordering::Relaxed)
        ));

        output.push_str(
            "# HELP rusmes_storage_mailboxes_created_total Total number of mailboxes created\n",
        );
        output.push_str("# TYPE rusmes_storage_mailboxes_created_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_mailboxes_created_total {}\n",
            self.mailboxes_created.load(Ordering::Relaxed)
        ));

        output.push_str(
            "# HELP rusmes_storage_mailboxes_deleted_total Total number of mailboxes deleted\n",
        );
        output.push_str("# TYPE rusmes_storage_mailboxes_deleted_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_mailboxes_deleted_total {}\n",
            self.mailboxes_deleted.load(Ordering::Relaxed)
        ));

        // Disk usage metrics
        output.push_str("# HELP rusmes_storage_disk_usage_bytes Total disk usage in bytes\n");
        output.push_str("# TYPE rusmes_storage_disk_usage_bytes gauge\n");
        output.push_str(&format!(
            "rusmes_storage_disk_usage_bytes {}\n",
            self.disk_usage_total_bytes.load(Ordering::Relaxed)
        ));

        // Per-user disk usage
        if let Ok(map) = self.disk_usage_per_user.read() {
            if !map.is_empty() {
                output.push_str(
                    "# HELP rusmes_storage_user_disk_usage_bytes Disk usage per user in bytes\n",
                );
                output.push_str("# TYPE rusmes_storage_user_disk_usage_bytes gauge\n");
                for (user, counter) in map.iter() {
                    output.push_str(&format!(
                        "rusmes_storage_user_disk_usage_bytes{{user=\"{}\"}} {}\n",
                        user,
                        counter.load(Ordering::Relaxed)
                    ));
                }
            }
        }

        // Per-mailbox message counts
        if let Ok(map) = self.messages_per_mailbox.read() {
            if !map.is_empty() {
                output
                    .push_str("# HELP rusmes_storage_mailbox_messages Message count per mailbox\n");
                output.push_str("# TYPE rusmes_storage_mailbox_messages gauge\n");
                for (mailbox_id, counter) in map.iter() {
                    output.push_str(&format!(
                        "rusmes_storage_mailbox_messages{{mailbox=\"{}\"}} {}\n",
                        mailbox_id,
                        counter.load(Ordering::Relaxed)
                    ));
                }
            }
        }

        // Operation counters
        output.push_str("# HELP rusmes_storage_append_operations_total Total append operations\n");
        output.push_str("# TYPE rusmes_storage_append_operations_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_append_operations_total {}\n",
            self.append_operations_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_fetch_operations_total Total fetch operations\n");
        output.push_str("# TYPE rusmes_storage_fetch_operations_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_fetch_operations_total {}\n",
            self.fetch_operations_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_delete_operations_total Total delete operations\n");
        output.push_str("# TYPE rusmes_storage_delete_operations_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_delete_operations_total {}\n",
            self.delete_operations_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_search_operations_total Total search operations\n");
        output.push_str("# TYPE rusmes_storage_search_operations_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_search_operations_total {}\n",
            self.search_operations_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_copy_operations_total Total copy operations\n");
        output.push_str("# TYPE rusmes_storage_copy_operations_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_copy_operations_total {}\n",
            self.copy_operations_total.load(Ordering::Relaxed)
        ));

        // Error counters
        output.push_str("# HELP rusmes_storage_append_errors_total Total append errors\n");
        output.push_str("# TYPE rusmes_storage_append_errors_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_append_errors_total {}\n",
            self.append_errors_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_fetch_errors_total Total fetch errors\n");
        output.push_str("# TYPE rusmes_storage_fetch_errors_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_fetch_errors_total {}\n",
            self.fetch_errors_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_delete_errors_total Total delete errors\n");
        output.push_str("# TYPE rusmes_storage_delete_errors_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_delete_errors_total {}\n",
            self.delete_errors_total.load(Ordering::Relaxed)
        ));

        output.push_str("# HELP rusmes_storage_search_errors_total Total search errors\n");
        output.push_str("# TYPE rusmes_storage_search_errors_total counter\n");
        output.push_str(&format!(
            "rusmes_storage_search_errors_total {}\n",
            self.search_errors_total.load(Ordering::Relaxed)
        ));

        // Backend health
        output.push_str("# HELP rusmes_storage_backend_healthy Backend health status (1=healthy, 0=unhealthy)\n");
        output.push_str("# TYPE rusmes_storage_backend_healthy gauge\n");
        output.push_str(&format!(
            "rusmes_storage_backend_healthy {}\n",
            self.backend_healthy.load(Ordering::Relaxed)
        ));

        output.push_str(
            "# HELP rusmes_storage_backend_last_check_timestamp Last health check timestamp\n",
        );
        output.push_str("# TYPE rusmes_storage_backend_last_check_timestamp gauge\n");
        output.push_str(&format!(
            "rusmes_storage_backend_last_check_timestamp {}\n",
            self.backend_last_check.load(Ordering::Relaxed)
        ));

        // Latency histograms
        output.push_str(&self.append_latency.export(
            "rusmes_storage_append_latency_seconds",
            "Append operation latency in seconds",
        ));

        output.push_str(&self.fetch_latency.export(
            "rusmes_storage_fetch_latency_seconds",
            "Fetch operation latency in seconds",
        ));

        output.push_str(&self.delete_latency.export(
            "rusmes_storage_delete_latency_seconds",
            "Delete operation latency in seconds",
        ));

        output.push_str(&self.search_latency.export(
            "rusmes_storage_search_latency_seconds",
            "Search operation latency in seconds",
        ));

        output
    }

    /// Get a summary of current metrics
    pub fn get_summary(&self) -> MetricsSummary {
        MetricsSummary {
            messages_total: self.messages_total.load(Ordering::Relaxed),
            messages_deleted: self.messages_deleted.load(Ordering::Relaxed),
            mailboxes_total: self.mailboxes_total.load(Ordering::Relaxed),
            disk_usage_bytes: self.disk_usage_total_bytes.load(Ordering::Relaxed),
            append_operations: self.append_operations_total.load(Ordering::Relaxed),
            fetch_operations: self.fetch_operations_total.load(Ordering::Relaxed),
            delete_operations: self.delete_operations_total.load(Ordering::Relaxed),
            search_operations: self.search_operations_total.load(Ordering::Relaxed),
            append_errors: self.append_errors_total.load(Ordering::Relaxed),
            fetch_errors: self.fetch_errors_total.load(Ordering::Relaxed),
            delete_errors: self.delete_errors_total.load(Ordering::Relaxed),
            search_errors: self.search_errors_total.load(Ordering::Relaxed),
            backend_healthy: self.is_backend_healthy(),
            append_avg_latency_ms: self.append_latency.average() * 1000.0,
            fetch_avg_latency_ms: self.fetch_latency.average() * 1000.0,
            delete_avg_latency_ms: self.delete_latency.average() * 1000.0,
            search_avg_latency_ms: self.search_latency.average() * 1000.0,
        }
    }
}

/// Summary of storage metrics
#[derive(Debug, Clone)]
pub struct MetricsSummary {
    pub messages_total: u64,
    pub messages_deleted: u64,
    pub mailboxes_total: u64,
    pub disk_usage_bytes: u64,
    pub append_operations: u64,
    pub fetch_operations: u64,
    pub delete_operations: u64,
    pub search_operations: u64,
    pub append_errors: u64,
    pub fetch_errors: u64,
    pub delete_errors: u64,
    pub search_errors: u64,
    pub backend_healthy: bool,
    pub append_avg_latency_ms: f64,
    pub fetch_avg_latency_ms: f64,
    pub delete_avg_latency_ms: f64,
    pub search_avg_latency_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram_observe() {
        let hist = Histogram::new(vec![0.001, 0.01, 0.1, 1.0]);

        hist.observe(0.005);
        hist.observe(0.05);
        hist.observe(0.5);

        assert_eq!(hist.get_count(), 3);
        assert!(hist.average() > 0.0);
    }

    #[test]
    fn test_storage_metrics_messages() {
        let metrics = StorageMetrics::new();

        metrics.inc_messages_total(5);
        assert_eq!(metrics.get_messages_total(), 5);

        metrics.dec_messages_total(2);
        assert_eq!(metrics.get_messages_total(), 3);

        metrics.set_messages_total(10);
        assert_eq!(metrics.get_messages_total(), 10);
    }

    #[test]
    fn test_storage_metrics_disk_usage() {
        let metrics = StorageMetrics::new();

        metrics.add_disk_usage_bytes(1024);
        assert_eq!(metrics.get_disk_usage_bytes(), 1024);

        metrics.sub_disk_usage_bytes(512);
        assert_eq!(metrics.get_disk_usage_bytes(), 512);

        metrics.set_disk_usage_bytes(2048);
        assert_eq!(metrics.get_disk_usage_bytes(), 2048);
    }

    #[test]
    fn test_storage_metrics_per_user() {
        let metrics = StorageMetrics::new();

        metrics.add_user_disk_usage("user1", 1024);
        metrics.add_user_disk_usage("user2", 2048);

        assert_eq!(metrics.get_user_disk_usage("user1"), 1024);
        assert_eq!(metrics.get_user_disk_usage("user2"), 2048);
        assert_eq!(metrics.get_disk_usage_bytes(), 3072);

        metrics.sub_user_disk_usage("user1", 512);
        assert_eq!(metrics.get_user_disk_usage("user1"), 512);
        assert_eq!(metrics.get_disk_usage_bytes(), 2560);
    }

    #[test]
    fn test_storage_metrics_per_mailbox() {
        let metrics = StorageMetrics::new();

        metrics.set_mailbox_message_count("mailbox1", 10);
        metrics.inc_mailbox_message_count("mailbox1", 5);

        assert_eq!(metrics.get_mailbox_message_count("mailbox1"), 15);

        metrics.dec_mailbox_message_count("mailbox1", 3);
        assert_eq!(metrics.get_mailbox_message_count("mailbox1"), 12);
    }

    #[test]
    fn test_storage_metrics_operations() {
        let metrics = StorageMetrics::new();

        metrics.inc_append_operations();
        metrics.inc_fetch_operations();
        metrics.inc_delete_operations();
        metrics.inc_search_operations();

        let summary = metrics.get_summary();
        assert_eq!(summary.append_operations, 1);
        assert_eq!(summary.fetch_operations, 1);
        assert_eq!(summary.delete_operations, 1);
        assert_eq!(summary.search_operations, 1);
    }

    #[test]
    fn test_storage_metrics_errors() {
        let metrics = StorageMetrics::new();

        metrics.inc_append_errors();
        metrics.inc_fetch_errors();
        metrics.inc_delete_errors();
        metrics.inc_search_errors();

        let summary = metrics.get_summary();
        assert_eq!(summary.append_errors, 1);
        assert_eq!(summary.fetch_errors, 1);
        assert_eq!(summary.delete_errors, 1);
        assert_eq!(summary.search_errors, 1);
    }

    #[test]
    fn test_storage_metrics_backend_health() {
        let metrics = StorageMetrics::new();

        assert!(metrics.is_backend_healthy());

        metrics.set_backend_healthy(false);
        assert!(!metrics.is_backend_healthy());

        metrics.set_backend_healthy(true);
        assert!(metrics.is_backend_healthy());
    }

    #[test]
    fn test_storage_metrics_helper_methods() {
        let metrics = StorageMetrics::new();

        metrics.record_append_success(1024);
        assert_eq!(metrics.get_messages_total(), 1);
        assert_eq!(metrics.get_disk_usage_bytes(), 1024);

        metrics.record_delete_success(512, 1);
        assert_eq!(metrics.get_messages_total(), 0);
        assert_eq!(metrics.get_disk_usage_bytes(), 512);

        metrics.record_append_failure();
        let summary = metrics.get_summary();
        assert_eq!(summary.append_errors, 1);
    }

    #[test]
    fn test_prometheus_export() {
        let metrics = StorageMetrics::new();

        metrics.inc_messages_total(100);
        metrics.add_disk_usage_bytes(1048576);
        metrics.inc_mailboxes_total();
        metrics.inc_append_operations();

        let output = metrics.export_prometheus();

        assert!(output.contains("rusmes_storage_messages_total 100"));
        assert!(output.contains("rusmes_storage_disk_usage_bytes 1048576"));
        assert!(output.contains("rusmes_storage_mailboxes_total 1"));
        assert!(output.contains("rusmes_storage_append_operations_total 1"));
        assert!(output.contains("# HELP"));
        assert!(output.contains("# TYPE"));
    }

    #[test]
    fn test_timer() {
        let metrics = StorageMetrics::new();

        let timer = metrics.start_append_timer();
        std::thread::sleep(std::time::Duration::from_millis(10));
        timer.observe();

        assert!(metrics.append_latency.get_count() > 0);
        assert!(metrics.append_latency.average() >= 0.01);
    }
}
