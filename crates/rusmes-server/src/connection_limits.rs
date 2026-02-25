//! Connection limits and tracking module
//!
//! This module provides connection management and limits enforcement:
//! - Max connections per IP address
//! - Max total connections
//! - Idle connection timeout enforcement
//! - Automatic connection reaping
//! - Connection tracking and statistics
//!
//! # Example Usage
//!
//! ```rust,no_run
//! use std::net::IpAddr;
//! use std::time::Duration;
//! use rusmes_server::connection_limits::{
//!     ConnectionLimiter, ConnectionLimitConfig, ConnectionLimitConfigBuilder
//! };
//!
//! #[tokio::main]
//! async fn main() {
//!     // Create a connection limiter with custom configuration
//!     let config = ConnectionLimitConfigBuilder::new()
//!         .max_connections_per_ip(10)
//!         .max_total_connections(1000)
//!         .idle_timeout(Duration::from_secs(300))
//!         .reaper_interval(Duration::from_secs(60))
//!         .build();
//!
//!     let limiter = ConnectionLimiter::new(config);
//!
//!     // Start the background reaper task
//!     let _reaper_handle = limiter.clone().start_reaper();
//!
//!     // Accept a connection from an IP
//!     let client_ip: IpAddr = "192.168.1.100".parse().unwrap();
//!
//!     match limiter.acquire(client_ip).await {
//!         Ok(guard) => {
//!             // Connection accepted, guard will auto-unregister on drop
//!             println!("Connection accepted: {}", guard.id());
//!
//!             // Update activity during the connection lifetime
//!             guard.update_activity().await;
//!
//!             // Guard is automatically dropped when it goes out of scope
//!         }
//!         Err(err) => {
//!             println!("Connection rejected: {}", err);
//!         }
//!     }
//!
//!     // Get statistics
//!     let stats = limiter.get_stats().await;
//!     println!("Current connections: {}", stats.current_connections);
//!     println!("Peak connections: {}", stats.peak_connections);
//!     println!("Total connections: {}", stats.total_connections);
//!     println!("Total rejected: {}", stats.total_rejected);
//!     println!("Total reaped: {}", stats.total_reaped);
//! }
//! ```

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, info};

/// Configuration for connection limits
#[derive(Debug, Clone)]
pub struct ConnectionLimitConfig {
    /// Maximum connections allowed per IP address (0 = unlimited)
    pub max_connections_per_ip: usize,
    /// Maximum total connections allowed (0 = unlimited)
    pub max_total_connections: usize,
    /// Idle timeout duration for connections
    pub idle_timeout: Duration,
    /// How often to run the reaper task
    pub reaper_interval: Duration,
}

impl Default for ConnectionLimitConfig {
    fn default() -> Self {
        Self {
            max_connections_per_ip: 10,
            max_total_connections: 1000,
            idle_timeout: Duration::from_secs(300), // 5 minutes
            reaper_interval: Duration::from_secs(60), // 1 minute
        }
    }
}

/// Connection tracking information
#[derive(Debug, Clone)]
struct ConnectionInfo {
    /// Unique connection ID
    #[allow(dead_code)]
    id: u64,
    /// IP address of the connection
    ip: IpAddr,
    /// Timestamp when connection was established
    #[allow(dead_code)]
    established_at: Instant,
    /// Timestamp of last activity
    last_activity: Instant,
}

impl ConnectionInfo {
    /// Create a new connection info
    fn new(id: u64, ip: IpAddr) -> Self {
        let now = Instant::now();
        Self {
            id,
            ip,
            established_at: now,
            last_activity: now,
        }
    }

    /// Update last activity timestamp
    fn update_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Check if connection is idle beyond the timeout
    fn is_idle(&self, timeout: Duration) -> bool {
        self.last_activity.elapsed() > timeout
    }
}

/// Connection statistics
#[derive(Debug, Clone, Default)]
pub struct ConnectionStats {
    /// Current number of active connections
    pub current_connections: usize,
    /// Peak number of concurrent connections
    pub peak_connections: usize,
    /// Total connections accepted since start
    pub total_connections: u64,
    /// Total connections rejected due to limits
    pub total_rejected: u64,
    /// Total connections reaped due to idle timeout
    pub total_reaped: u64,
}

/// Internal state for connection limiter
struct ConnectionLimiterState {
    /// Configuration
    config: ConnectionLimitConfig,
    /// Next connection ID
    next_id: u64,
    /// Active connections by ID
    connections: HashMap<u64, ConnectionInfo>,
    /// Connection IDs per IP address
    connections_per_ip: HashMap<IpAddr, Vec<u64>>,
    /// Statistics
    stats: ConnectionStats,
}

impl ConnectionLimiterState {
    fn new(config: ConnectionLimitConfig) -> Self {
        Self {
            config,
            next_id: 1,
            connections: HashMap::new(),
            connections_per_ip: HashMap::new(),
            stats: ConnectionStats::default(),
        }
    }

    /// Check if a new connection from the given IP can be accepted
    fn can_accept(&self, ip: IpAddr) -> Result<(), String> {
        // Check total connections limit
        if self.config.max_total_connections > 0
            && self.connections.len() >= self.config.max_total_connections
        {
            return Err(format!(
                "Maximum total connections ({}) reached",
                self.config.max_total_connections
            ));
        }

        // Check per-IP limit
        if self.config.max_connections_per_ip > 0 {
            let ip_count = self
                .connections_per_ip
                .get(&ip)
                .map(|v| v.len())
                .unwrap_or(0);
            if ip_count >= self.config.max_connections_per_ip {
                return Err(format!(
                    "Maximum connections per IP ({}) reached for {}",
                    self.config.max_connections_per_ip, ip
                ));
            }
        }

        Ok(())
    }

    /// Register a new connection
    fn register_connection(&mut self, ip: IpAddr) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        let conn_info = ConnectionInfo::new(id, ip);
        self.connections.insert(id, conn_info);

        self.connections_per_ip.entry(ip).or_default().push(id);

        // Update statistics
        self.stats.current_connections = self.connections.len();
        self.stats.total_connections = self.stats.total_connections.wrapping_add(1);
        if self.stats.current_connections > self.stats.peak_connections {
            self.stats.peak_connections = self.stats.current_connections;
        }

        id
    }

    /// Unregister a connection
    fn unregister_connection(&mut self, id: u64) {
        if let Some(conn_info) = self.connections.remove(&id) {
            // Remove from per-IP tracking
            if let Some(ip_conns) = self.connections_per_ip.get_mut(&conn_info.ip) {
                ip_conns.retain(|&conn_id| conn_id != id);
                if ip_conns.is_empty() {
                    self.connections_per_ip.remove(&conn_info.ip);
                }
            }

            // Update statistics
            self.stats.current_connections = self.connections.len();
        }
    }

    /// Update activity timestamp for a connection
    fn update_activity(&mut self, id: u64) {
        if let Some(conn_info) = self.connections.get_mut(&id) {
            conn_info.update_activity();
        }
    }

    /// Reap idle connections and return list of reaped connection IDs
    fn reap_idle_connections(&mut self) -> Vec<u64> {
        let mut reaped = Vec::new();
        let idle_timeout = self.config.idle_timeout;

        // Find idle connections
        for (&id, conn_info) in &self.connections {
            if conn_info.is_idle(idle_timeout) {
                reaped.push(id);
            }
        }

        // Remove them
        for &id in &reaped {
            self.unregister_connection(id);
            self.stats.total_reaped = self.stats.total_reaped.wrapping_add(1);
        }

        reaped
    }

    /// Get current statistics
    fn get_stats(&self) -> ConnectionStats {
        self.stats.clone()
    }
}

/// Connection limiter and tracker
#[derive(Clone)]
#[allow(dead_code)]
pub struct ConnectionLimiter {
    state: Arc<RwLock<ConnectionLimiterState>>,
}

impl ConnectionLimiter {
    /// Create a new connection limiter with the given configuration
    #[allow(dead_code)]
    pub fn new(config: ConnectionLimitConfig) -> Self {
        Self {
            state: Arc::new(RwLock::new(ConnectionLimiterState::new(config))),
        }
    }

    /// Create a connection limiter with default configuration
    #[allow(dead_code)]
    pub fn with_defaults() -> Self {
        Self::new(ConnectionLimitConfig::default())
    }

    /// Attempt to acquire a connection slot for the given IP address
    ///
    /// Returns a `ConnectionGuard` on success, or an error if limits are exceeded.
    #[allow(dead_code)]
    pub async fn acquire(&self, ip: IpAddr) -> Result<ConnectionGuard, String> {
        let mut state = self.state.write().await;

        // Check if we can accept this connection
        state.can_accept(ip)?;

        // Register the connection
        let id = state.register_connection(ip);

        debug!(
            "Connection accepted: id={}, ip={}, current={}, peak={}",
            id, ip, state.stats.current_connections, state.stats.peak_connections
        );

        Ok(ConnectionGuard {
            id,
            limiter: self.clone(),
        })
    }

    /// Update activity timestamp for a connection
    #[allow(dead_code)]
    pub async fn update_activity(&self, id: u64) {
        let mut state = self.state.write().await;
        state.update_activity(id);
    }

    /// Get current connection statistics
    #[allow(dead_code)]
    pub async fn get_stats(&self) -> ConnectionStats {
        let state = self.state.read().await;
        state.get_stats()
    }

    /// Update configuration (hot reload)
    #[allow(dead_code)]
    pub async fn update_config(&self, config: ConnectionLimitConfig) {
        let mut state = self.state.write().await;
        info!(
            "Updating connection limits: max_per_ip={}, max_total={}, idle_timeout={:?}",
            config.max_connections_per_ip, config.max_total_connections, config.idle_timeout
        );
        state.config = config;
    }

    /// Get current configuration
    #[allow(dead_code)]
    pub async fn get_config(&self) -> ConnectionLimitConfig {
        let state = self.state.read().await;
        state.config.clone()
    }

    /// Manually trigger idle connection reaping
    #[allow(dead_code)]
    pub async fn reap_idle(&self) -> usize {
        let mut state = self.state.write().await;
        let reaped = state.reap_idle_connections();
        let count = reaped.len();

        if count > 0 {
            info!("Reaped {} idle connections", count);
            debug!("Reaped connection IDs: {:?}", reaped);
        }

        count
    }

    /// Start the background reaper task
    ///
    /// This task runs periodically to clean up idle connections.
    /// Returns a handle that can be used to abort the task.
    #[allow(dead_code)]
    pub fn start_reaper(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let reaper_interval = {
                let state = self.state.read().await;
                state.config.reaper_interval
            };

            let mut ticker = interval(reaper_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                let count = self.reap_idle().await;
                if count > 0 {
                    let stats = self.get_stats().await;
                    info!(
                        "Connection reaper: reaped={}, current={}, total_reaped={}",
                        count, stats.current_connections, stats.total_reaped
                    );
                }
            }
        })
    }

    /// Reject a connection (for statistics tracking)
    #[allow(dead_code)]
    async fn record_rejection(&self) {
        let mut state = self.state.write().await;
        state.stats.total_rejected = state.stats.total_rejected.wrapping_add(1);
    }
}

/// RAII guard for a connection
///
/// When dropped, automatically unregisters the connection from the limiter.
#[allow(dead_code)]
pub struct ConnectionGuard {
    id: u64,
    limiter: ConnectionLimiter,
}

impl ConnectionGuard {
    /// Get the connection ID
    #[allow(dead_code)]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Update activity timestamp for this connection
    #[allow(dead_code)]
    pub async fn update_activity(&self) {
        self.limiter.update_activity(self.id).await;
    }

    /// Consume the guard without dropping the connection
    ///
    /// Returns the connection ID. The caller is responsible for
    /// manually unregistering the connection later.
    #[allow(dead_code)]
    pub fn into_id(self) -> u64 {
        let id = self.id;
        std::mem::forget(self); // Don't drop
        id
    }

    /// Manually unregister a connection by ID
    #[allow(dead_code)]
    pub async fn unregister(limiter: &ConnectionLimiter, id: u64) {
        let mut state = limiter.state.write().await;
        state.unregister_connection(id);
        debug!(
            "Connection released: id={}, current={}",
            id, state.stats.current_connections
        );
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        // Spawn a task to unregister the connection asynchronously
        let limiter = self.limiter.clone();
        let id = self.id;
        tokio::spawn(async move {
            ConnectionGuard::unregister(&limiter, id).await;
        });
    }
}

/// Builder for ConnectionLimitConfig
pub struct ConnectionLimitConfigBuilder {
    max_connections_per_ip: usize,
    max_total_connections: usize,
    idle_timeout: Duration,
    reaper_interval: Duration,
}

impl ConnectionLimitConfigBuilder {
    /// Create a new builder with default values
    pub fn new() -> Self {
        let defaults = ConnectionLimitConfig::default();
        Self {
            max_connections_per_ip: defaults.max_connections_per_ip,
            max_total_connections: defaults.max_total_connections,
            idle_timeout: defaults.idle_timeout,
            reaper_interval: defaults.reaper_interval,
        }
    }

    /// Set maximum connections per IP (0 = unlimited)
    pub fn max_connections_per_ip(mut self, max: usize) -> Self {
        self.max_connections_per_ip = max;
        self
    }

    /// Set maximum total connections (0 = unlimited)
    #[allow(dead_code)]
    pub fn max_total_connections(mut self, max: usize) -> Self {
        self.max_total_connections = max;
        self
    }

    /// Set idle timeout duration
    #[allow(dead_code)]
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Set reaper interval duration
    #[allow(dead_code)]
    pub fn reaper_interval(mut self, interval: Duration) -> Self {
        self.reaper_interval = interval;
        self
    }

    /// Build the configuration
    pub fn build(self) -> ConnectionLimitConfig {
        ConnectionLimitConfig {
            max_connections_per_ip: self.max_connections_per_ip,
            max_total_connections: self.max_total_connections,
            idle_timeout: self.idle_timeout,
            reaper_interval: self.reaper_interval,
        }
    }
}

impl Default for ConnectionLimitConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to create a connection limiter from rusmes-config
pub fn from_server_config(config: &rusmes_config::ServerConfig) -> ConnectionLimiter {
    let mut builder = ConnectionLimitConfigBuilder::new();

    // Use dedicated connection_limits config if available
    if let Some(conn_limits) = &config.connection_limits {
        builder = builder.max_connections_per_ip(conn_limits.max_connections_per_ip);
        builder = builder.max_total_connections(conn_limits.max_total_connections);

        if let Ok(idle_secs) = conn_limits.idle_timeout_seconds() {
            builder = builder.idle_timeout(Duration::from_secs(idle_secs));
        }

        if let Ok(reaper_secs) = conn_limits.reaper_interval_seconds() {
            builder = builder.reaper_interval(Duration::from_secs(reaper_secs));
        }
    } else {
        // Fall back to SMTP rate_limit config for backwards compatibility
        if let Some(rate_limit) = &config.smtp.rate_limit {
            builder = builder.max_connections_per_ip(rate_limit.max_connections_per_ip);
        }
        // Use default idle timeout
        builder = builder.idle_timeout(Duration::from_secs(300));
    }

    let limit_config = builder.build();
    ConnectionLimiter::new(limit_config)
}

#[allow(dead_code)]
fn create_connection_limiter_from_config(
    config: &rusmes_config::ServerConfig,
) -> ConnectionLimiter {
    from_server_config(config)
}
