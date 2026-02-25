//! Rate limiting for connection and message processing

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

/// Rate limiter configuration
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum connections per IP address
    pub max_connections_per_ip: usize,
    /// Maximum messages per hour per IP
    pub max_messages_per_hour: usize,
    /// Time window for rate limiting
    pub window_duration: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_connections_per_ip: 10,
            max_messages_per_hour: 100,
            window_duration: Duration::from_secs(3600), // 1 hour
        }
    }
}

/// Connection counter entry
#[derive(Debug, Clone)]
struct ConnectionEntry {
    count: usize,
    first_seen: Instant,
}

/// Message counter entry
#[derive(Debug, Clone)]
struct MessageEntry {
    count: usize,
    window_start: Instant,
}

/// Rate limiter for SMTP connections and messages
pub struct RateLimiter {
    config: Arc<RwLock<RateLimitConfig>>,
    connections: Arc<Mutex<HashMap<IpAddr, ConnectionEntry>>>,
    messages: Arc<Mutex<HashMap<IpAddr, MessageEntry>>>,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            connections: Arc::new(Mutex::new(HashMap::new())),
            messages: Arc::new(Mutex::new(HashMap::new())),
        }
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

    /// Check if a message from this IP is allowed
    pub async fn allow_message(&self, ip: IpAddr) -> bool {
        let config = self.config.read().await;
        let mut messages = self.messages.lock().await;

        let now = Instant::now();
        let window_duration = config.window_duration;
        let max_messages = config.max_messages_per_hour;

        match messages.get_mut(&ip) {
            Some(entry) => {
                // Check if we need to reset the window
                if now.duration_since(entry.window_start) >= window_duration {
                    entry.count = 1;
                    entry.window_start = now;
                    true
                } else if entry.count >= max_messages {
                    tracing::warn!("Message rate limit exceeded for IP: {}", ip);
                    false
                } else {
                    entry.count += 1;
                    true
                }
            }
            None => {
                messages.insert(
                    ip,
                    MessageEntry {
                        count: 1,
                        window_start: now,
                    },
                );
                true
            }
        }
    }

    /// Get current connection count for an IP
    pub async fn get_connection_count(&self, ip: IpAddr) -> usize {
        let connections = self.connections.lock().await;
        connections.get(&ip).map(|e| e.count).unwrap_or(0)
    }

    /// Get current message count for an IP
    pub async fn get_message_count(&self, ip: IpAddr) -> usize {
        let messages = self.messages.lock().await;
        messages.get(&ip).map(|e| e.count).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[tokio::test]
    async fn test_connection_limit() {
        let config = RateLimitConfig {
            max_connections_per_ip: 2,
            max_messages_per_hour: 100,
            window_duration: Duration::from_secs(60),
        };

        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // First two connections should succeed
        assert!(limiter.allow_connection(ip).await);
        assert!(limiter.allow_connection(ip).await);

        // Third should fail
        assert!(!limiter.allow_connection(ip).await);

        // Release one connection
        limiter.release_connection(ip).await;

        // Now should succeed again
        assert!(limiter.allow_connection(ip).await);
    }

    #[tokio::test]
    async fn test_message_limit() {
        let config = RateLimitConfig {
            max_connections_per_ip: 10,
            max_messages_per_hour: 2,
            window_duration: Duration::from_secs(60),
        };

        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

        // First two messages should succeed
        assert!(limiter.allow_message(ip).await);
        assert!(limiter.allow_message(ip).await);

        // Third should fail
        assert!(!limiter.allow_message(ip).await);
    }
}
