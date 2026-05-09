//! Rate limiting for connection and message processing
//!
//! Provides per-IP, per-sender, and combined IP+sender rate limiting.
//! Bucket state can be persisted to a JSON file and reloaded on startup,
//! so limits survive server restarts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

// ── Key types ──────────────────────────────────────────────────────────────

/// Identifies the axis on which a rate limit is applied.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    /// Limit by remote client IP address
    Ip(IpAddr),
    /// Limit by MAIL FROM envelope sender address
    Sender(String),
    /// Limit by (IP, sender) pair simultaneously
    IpAndSender(IpAddr, String),
}

impl RateLimitKey {
    /// Serialize to a compact string suitable for use as a JSON map key.
    fn to_key_string(&self) -> String {
        match self {
            RateLimitKey::Ip(ip) => format!("ip:{}", ip),
            RateLimitKey::Sender(addr) => format!("sender:{}", addr),
            RateLimitKey::IpAndSender(ip, addr) => format!("ip+sender:{}:{}", ip, addr),
        }
    }
}

// ── Configuration ──────────────────────────────────────────────────────────

/// Rate limiter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum connections per IP address (sliding window)
    pub max_connections_per_ip: usize,
    /// Maximum messages per window per rate-limit key
    pub max_messages_per_window: usize,
    /// Duration of the rate-limit time window
    #[serde(with = "duration_secs_serde")]
    pub window_duration: Duration,
    /// How often (seconds) the bucket state is persisted to disk.
    /// None disables persistence.
    pub persist_interval_secs: Option<u64>,
    /// Directory where `ratelimit.json` is written.
    /// None disables persistence.
    pub runtime_dir: Option<PathBuf>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_connections_per_ip: 10,
            max_messages_per_window: 100,
            window_duration: Duration::from_secs(3600), // 1 hour
            persist_interval_secs: Some(60),
            runtime_dir: None,
        }
    }
}

mod duration_secs_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

// ── Bucket state (serializable) ────────────────────────────────────────────

/// A single message-count bucket entry — serializable for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BucketEntry {
    count: usize,
    /// Unix timestamp (seconds) of when this window started
    window_start_secs: u64,
}

impl BucketEntry {
    fn new(now: Instant) -> Self {
        Self {
            count: 1,
            window_start_secs: unix_secs_from_instant(now),
        }
    }

    fn is_expired(&self, window_duration: Duration) -> bool {
        let elapsed = unix_secs_now().saturating_sub(self.window_start_secs);
        elapsed >= window_duration.as_secs()
    }
}

/// Snapshot that maps string-keyed buckets to their entry data
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BucketSnapshot {
    /// message-count buckets, keyed by `RateLimitKey::to_key_string()`
    messages: HashMap<String, BucketEntry>,
}

// ── Connection counter (not persisted — transient, session-scoped) ─────────

#[derive(Debug, Clone)]
struct ConnectionEntry {
    count: usize,
    first_seen: Instant,
}

// ── RateLimiter ───────────────────────────────────────────────────────────

/// Rate limiter for SMTP connections and messages.
///
/// Supports three keying strategies:
///  - `RateLimitKey::Ip` — the classic per-IP limit
///  - `RateLimitKey::Sender` — per MAIL FROM address
///  - `RateLimitKey::IpAndSender` — combined, tightest control
///
/// State is periodically snapshotted to `<runtime_dir>/ratelimit.json`
/// (if `runtime_dir` is configured) and re-loaded on startup.
pub struct RateLimiter {
    config: Arc<RwLock<RateLimitConfig>>,
    connections: Arc<Mutex<HashMap<IpAddr, ConnectionEntry>>>,
    buckets: Arc<Mutex<HashMap<String, BucketEntry>>>,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// This constructor is sync-callable and does **not** spawn any background
    /// tasks; it can therefore be invoked outside of a Tokio runtime (e.g. in
    /// `#[test]` blocks or during synchronous wiring code).
    ///
    /// To enable periodic snapshotting of bucket state to disk, call
    /// [`RateLimiter::start_persistence_task`] from inside an async context
    /// after construction.
    pub fn new(config: RateLimitConfig) -> Self {
        let buckets = Arc::new(Mutex::new(HashMap::new()));
        let config_arc = Arc::new(RwLock::new(config));

        Self {
            config: Arc::clone(&config_arc),
            connections: Arc::new(Mutex::new(HashMap::new())),
            buckets: Arc::clone(&buckets),
        }
    }

    /// Create a rate limiter and immediately restore persisted state from `snapshot_path`.
    ///
    /// This is the production constructor; `new()` is the simpler form that
    /// relies on the runtime_dir in config. Use this to control the path explicitly
    /// (handy in tests).
    ///
    /// As with [`RateLimiter::new`], no background persistence task is spawned —
    /// call [`RateLimiter::start_persistence_task`] explicitly afterwards.
    pub async fn new_with_restore(config: RateLimitConfig, snapshot_path: &Path) -> Self {
        let buckets = Arc::new(Mutex::new(HashMap::new()));

        // Try to load persisted state
        if let Err(e) = restore_from_file(&buckets, snapshot_path).await {
            tracing::warn!(
                "Rate limit state not restored from {:?}: {}",
                snapshot_path,
                e
            );
        } else {
            tracing::info!("Rate limit state restored from {:?}", snapshot_path);
        }

        let config_arc = Arc::new(RwLock::new(config));

        Self {
            config: config_arc,
            connections: Arc::new(Mutex::new(HashMap::new())),
            buckets,
        }
    }

    /// Start the background persistence task.
    ///
    /// Spawns a Tokio task that snapshots the message-bucket state to
    /// `<runtime_dir>/ratelimit.json` every `interval`. Returns the
    /// [`JoinHandle`] so callers can manage the task lifecycle if desired.
    ///
    /// **Must be called from within a Tokio runtime.**
    pub fn start_persistence_task(
        &self,
        runtime_dir: PathBuf,
        interval: Duration,
    ) -> JoinHandle<()> {
        let buckets = Arc::clone(&self.buckets);
        tokio::spawn(async move {
            persistence_task(runtime_dir, interval, buckets).await;
        })
    }

    /// Snapshot the current bucket state to `path` (JSON format).
    pub async fn snapshot_to_file(&self, path: &Path) -> anyhow::Result<()> {
        let guard = self.buckets.lock().await;
        snapshot_to_file_locked(&guard, path).await
    }

    /// Restore bucket state from a JSON snapshot file.
    pub async fn restore_from_file(&self, path: &Path) -> anyhow::Result<()> {
        restore_from_file(&self.buckets, path).await
    }

    /// Update the rate limiter configuration (hot-reload support)
    pub async fn update_config(&self, new_config: RateLimitConfig) {
        let mut config = self.config.write().await;
        *config = new_config;
    }

    /// Check if a connection from this IP is allowed
    pub async fn allow_connection(&self, ip: IpAddr) -> bool {
        let config = self.config.read().await;
        let mut connections = self.connections.lock().await;

        // Clean up old entries
        let now = Instant::now();
        let window_duration = config.window_duration;
        connections.retain(|_, entry| now.duration_since(entry.first_seen) < window_duration);

        // Check current count
        let max_connections = config.max_connections_per_ip;
        match connections.get_mut(&ip) {
            Some(entry) => {
                if entry.count >= max_connections {
                    tracing::warn!("Connection rate limit exceeded for IP: {}", ip);
                    false
                } else {
                    entry.count += 1;
                    true
                }
            }
            None => {
                connections.insert(
                    ip,
                    ConnectionEntry {
                        count: 1,
                        first_seen: now,
                    },
                );
                true
            }
        }
    }

    /// Release a connection
    pub async fn release_connection(&self, ip: IpAddr) {
        let mut connections = self.connections.lock().await;
        if let Some(entry) = connections.get_mut(&ip) {
            if entry.count > 0 {
                entry.count -= 1;
            }
            if entry.count == 0 {
                connections.remove(&ip);
            }
        }
    }

    /// Check if a message for the given key is allowed (generic key variant).
    ///
    /// This is the primary per-sender/per-IP message check entry point.
    pub async fn allow_message_keyed(&self, key: &RateLimitKey) -> bool {
        let config = self.config.read().await;
        let max_messages = config.max_messages_per_window;
        let window_duration = config.window_duration;
        drop(config); // release read lock before locking buckets

        let key_str = key.to_key_string();
        let mut buckets = self.buckets.lock().await;

        match buckets.get_mut(&key_str) {
            Some(entry) => {
                if entry.is_expired(window_duration) {
                    // Reset window
                    *entry = BucketEntry::new(Instant::now());
                    true
                } else if entry.count >= max_messages {
                    tracing::warn!("Message rate limit exceeded for key: {}", key_str);
                    false
                } else {
                    entry.count += 1;
                    true
                }
            }
            None => {
                buckets.insert(key_str, BucketEntry::new(Instant::now()));
                true
            }
        }
    }

    /// Check if a message from this IP is allowed (legacy IP-only API for backwards compat)
    pub async fn allow_message(&self, ip: IpAddr) -> bool {
        self.allow_message_keyed(&RateLimitKey::Ip(ip)).await
    }

    /// Check if a message from this sender is allowed.
    pub async fn allow_message_from_sender(&self, sender: &str) -> bool {
        self.allow_message_keyed(&RateLimitKey::Sender(sender.to_string()))
            .await
    }

    /// Check if a message is allowed based on both IP and sender.
    pub async fn allow_message_ip_and_sender(&self, ip: IpAddr, sender: &str) -> bool {
        self.allow_message_keyed(&RateLimitKey::IpAndSender(ip, sender.to_string()))
            .await
    }

    /// Get current connection count for an IP
    pub async fn get_connection_count(&self, ip: IpAddr) -> usize {
        let connections = self.connections.lock().await;
        connections.get(&ip).map(|e| e.count).unwrap_or(0)
    }

    /// Get current message count for a key (for debugging/testing)
    pub async fn get_message_count_keyed(&self, key: &RateLimitKey) -> usize {
        let buckets = self.buckets.lock().await;
        buckets
            .get(&key.to_key_string())
            .map(|e| e.count)
            .unwrap_or(0)
    }

    /// Get current message count for an IP (legacy)
    pub async fn get_message_count(&self, ip: IpAddr) -> usize {
        self.get_message_count_keyed(&RateLimitKey::Ip(ip)).await
    }
}

// ── Persistence helpers ───────────────────────────────────────────────────

fn ratelimit_file_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("ratelimit.json")
}

async fn snapshot_to_file_locked(
    buckets: &HashMap<String, BucketEntry>,
    path: &Path,
) -> anyhow::Result<()> {
    let snapshot = BucketSnapshot {
        messages: buckets.clone(),
    };
    let json = serde_json::to_string_pretty(&snapshot)?;
    tokio::fs::write(path, json).await?;
    Ok(())
}

async fn restore_from_file(
    buckets: &Mutex<HashMap<String, BucketEntry>>,
    path: &Path,
) -> anyhow::Result<()> {
    if !tokio::fs::try_exists(path).await? {
        return Ok(());
    }
    let json = tokio::fs::read_to_string(path).await?;
    let snapshot: BucketSnapshot = serde_json::from_str(&json)?;
    let mut guard = buckets.lock().await;
    *guard = snapshot.messages;
    Ok(())
}

/// Background task that periodically persists rate limit state.
async fn persistence_task(
    runtime_dir: PathBuf,
    interval: Duration,
    buckets: Arc<Mutex<HashMap<String, BucketEntry>>>,
) {
    let path = ratelimit_file_path(&runtime_dir);
    loop {
        tokio::time::sleep(interval).await;

        let guard = buckets.lock().await;
        if let Err(e) = snapshot_to_file_locked(&guard, &path).await {
            tracing::warn!("Failed to persist rate limit state to {:?}: {}", path, e);
        } else {
            tracing::debug!("Rate limit state persisted to {:?}", path);
        }
    }
}

// ── Utility ───────────────────────────────────────────────────────────────

/// Current Unix timestamp in seconds (wall-clock, not monotonic)
fn unix_secs_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Convert a monotonic Instant to an approximate Unix timestamp.
/// Used only for the initial `window_start_secs` field.
fn unix_secs_from_instant(_instant: Instant) -> u64 {
    unix_secs_now()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_config(max_messages: usize) -> RateLimitConfig {
        RateLimitConfig {
            max_connections_per_ip: 2,
            max_messages_per_window: max_messages,
            window_duration: Duration::from_secs(3600),
            persist_interval_secs: None, // Don't spawn the background task interval
            runtime_dir: None,
        }
    }

    #[tokio::test]
    async fn test_connection_limit() {
        let limiter = RateLimiter::new(test_config(100));
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        assert!(limiter.allow_connection(ip).await);
        assert!(limiter.allow_connection(ip).await);
        assert!(!limiter.allow_connection(ip).await);

        limiter.release_connection(ip).await;
        assert!(limiter.allow_connection(ip).await);
    }

    #[tokio::test]
    async fn test_message_limit() {
        let config = RateLimitConfig {
            max_connections_per_ip: 10,
            max_messages_per_window: 2,
            ..test_config(2)
        };
        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

        assert!(limiter.allow_message(ip).await);
        assert!(limiter.allow_message(ip).await);
        assert!(!limiter.allow_message(ip).await);
    }

    #[tokio::test]
    async fn per_sender_rate_limit_sixth_rejected() {
        // 5 messages from spammer@x.com with limit=5 → 6th rejected
        let config = RateLimitConfig {
            max_messages_per_window: 5,
            persist_interval_secs: None,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);
        let sender = "spammer@x.com";

        for i in 1..=5 {
            let allowed = limiter.allow_message_from_sender(sender).await;
            assert!(allowed, "Message {} should be allowed", i);
        }

        let sixth_allowed = limiter.allow_message_from_sender(sender).await;
        assert!(!sixth_allowed, "6th message should be rejected");
    }

    #[tokio::test]
    async fn rate_limit_persistence_roundtrip() {
        let tmp_dir = std::env::temp_dir().join(format!("rusmes_rl_test_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&tmp_dir).await.unwrap();
        let snapshot_path = tmp_dir.join("ratelimit.json");

        // Create a limiter, add some bucket state
        {
            let config = RateLimitConfig {
                max_messages_per_window: 100,
                persist_interval_secs: None,
                runtime_dir: None,
                ..Default::default()
            };
            let limiter = RateLimiter::new(config);

            // Record 3 messages from spammer@example.com
            for _ in 0..3 {
                limiter
                    .allow_message_from_sender("spammer@example.com")
                    .await;
            }

            // Snapshot
            limiter.snapshot_to_file(&snapshot_path).await.unwrap();
        }

        // Reload into a new limiter
        {
            let config = RateLimitConfig {
                max_messages_per_window: 100,
                persist_interval_secs: None,
                runtime_dir: None,
                ..Default::default()
            };
            let limiter = RateLimiter::new_with_restore(config, &snapshot_path).await;

            let count = limiter
                .get_message_count_keyed(&RateLimitKey::Sender("spammer@example.com".to_string()))
                .await;
            assert_eq!(count, 3, "Bucket count should be preserved across restart");
        }

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
    }

    #[tokio::test]
    async fn rate_limit_ip_and_sender_key() {
        let config = RateLimitConfig {
            max_messages_per_window: 2,
            persist_interval_secs: None,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let sender = "user@spammer.com";

        assert!(limiter.allow_message_ip_and_sender(ip, sender).await);
        assert!(limiter.allow_message_ip_and_sender(ip, sender).await);
        assert!(!limiter.allow_message_ip_and_sender(ip, sender).await);

        // Different IP with same sender should be independent
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(limiter.allow_message_ip_and_sender(ip2, sender).await);
    }
}
