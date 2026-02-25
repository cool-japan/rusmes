//! EventSource (Server-Sent Events) endpoint for JMAP
//!
//! Implements RFC 8620 Section 7.3:
//! - GET /eventsource - Server-Sent Events for push notifications
//! - State change notifications
//! - Push subscription management

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;

/// EventSource state manager
#[derive(Clone)]
pub struct EventSourceManager {
    /// Broadcast channel for state changes
    tx: broadcast::Sender<StateChange>,
    /// Current state per data type
    states: Arc<RwLock<HashMap<String, String>>>,
}

/// State change event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChange {
    /// Data types that changed
    pub changed: HashMap<String, String>,
}

/// Push subscription
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscription {
    /// Push subscription URL
    pub url: String,
    /// Types to monitor
    #[serde(skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
}

/// EventSource query parameters
#[derive(Debug, Deserialize)]
pub struct EventSourceQuery {
    /// Data types to monitor (comma-separated)
    #[serde(default)]
    pub types: Option<String>,
    /// Close after this many seconds
    #[serde(default)]
    pub closeafter: Option<u64>,
    /// Ping interval in seconds
    #[serde(default)]
    pub ping: Option<u64>,
}

impl EventSourceManager {
    /// Create new EventSource manager
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(100);
        Self {
            tx,
            states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Notify state change
    pub fn notify_change(&self, data_type: String, new_state: String) {
        // Update stored state
        if let Ok(mut states) = self.states.write() {
            states.insert(data_type.clone(), new_state.clone());
        }

        // Broadcast change
        let mut changed = HashMap::new();
        changed.insert(data_type, new_state);

        let state_change = StateChange { changed };

        // Ignore send errors (no active listeners)
        let _ = self.tx.send(state_change);
    }

    /// Get current state for a data type
    pub fn get_state(&self, data_type: &str) -> Option<String> {
        self.states
            .read()
            .ok()
            .and_then(|states| states.get(data_type).cloned())
    }

    /// Subscribe to state changes
    fn subscribe(&self) -> broadcast::Receiver<StateChange> {
        self.tx.subscribe()
    }
}

impl Default for EventSourceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Create EventSource router
pub fn eventsource_routes() -> Router<EventSourceManager> {
    Router::new().route("/eventsource", get(eventsource_handler))
}

/// EventSource SSE handler
async fn eventsource_handler(
    Query(params): Query<EventSourceQuery>,
    State(manager): State<EventSourceManager>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Parse types filter
    let types_filter: Option<Vec<String>> = params
        .types
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect());

    // Subscribe to state changes
    let mut rx = manager.subscribe();

    // Determine close timeout
    let close_after = params.closeafter.map(Duration::from_secs);

    // Determine ping interval
    let ping_interval = params
        .ping
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30));

    // Create event stream
    let stream = async_stream::stream! {
        let start_time = tokio::time::Instant::now();

        loop {
            // Check if we should close
            if let Some(timeout) = close_after {
                if start_time.elapsed() >= timeout {
                    break;
                }
            }

            // Wait for next event or ping timeout
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(state_change) => {
                            // Filter by types if specified
                            let filtered_changes: HashMap<String, String> = if let Some(ref filter) = types_filter {
                                state_change.changed.into_iter()
                                    .filter(|(k, _)| filter.contains(k))
                                    .collect()
                            } else {
                                state_change.changed
                            };

                            // Only send if there are changes after filtering
                            if !filtered_changes.is_empty() {
                                let event_data = StateChange { changed: filtered_changes };
                                if let Ok(json) = serde_json::to_string(&event_data) {
                                    yield Ok(Event::default()
                                        .event("state")
                                        .data(json));
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // Client fell behind, send error event
                            yield Ok(Event::default()
                                .event("error")
                                .data("Client lagged behind"));
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            // Channel closed, end stream
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(ping_interval) => {
                    // Send ping to keep connection alive
                    yield Ok(Event::default().comment("ping"));
                }
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_source_manager_new() {
        let manager = EventSourceManager::new();
        assert!(manager.get_state("Email").is_none());
    }

    #[test]
    fn test_notify_change() {
        let manager = EventSourceManager::new();

        manager.notify_change("Email".to_string(), "state1".to_string());

        assert_eq!(manager.get_state("Email"), Some("state1".to_string()));
    }

    #[test]
    fn test_notify_multiple_changes() {
        let manager = EventSourceManager::new();

        manager.notify_change("Email".to_string(), "state1".to_string());
        manager.notify_change("Mailbox".to_string(), "state2".to_string());
        manager.notify_change("Thread".to_string(), "state3".to_string());

        assert_eq!(manager.get_state("Email"), Some("state1".to_string()));
        assert_eq!(manager.get_state("Mailbox"), Some("state2".to_string()));
        assert_eq!(manager.get_state("Thread"), Some("state3".to_string()));
    }

    #[test]
    fn test_state_update() {
        let manager = EventSourceManager::new();

        manager.notify_change("Email".to_string(), "state1".to_string());
        assert_eq!(manager.get_state("Email"), Some("state1".to_string()));

        manager.notify_change("Email".to_string(), "state2".to_string());
        assert_eq!(manager.get_state("Email"), Some("state2".to_string()));
    }

    #[test]
    fn test_subscribe() {
        let manager = EventSourceManager::new();
        let mut rx = manager.subscribe();

        manager.notify_change("Email".to_string(), "state1".to_string());

        let change = rx.try_recv().unwrap();
        assert_eq!(change.changed.get("Email"), Some(&"state1".to_string()));
    }

    #[test]
    fn test_multiple_subscribers() {
        let manager = EventSourceManager::new();
        let mut rx1 = manager.subscribe();
        let mut rx2 = manager.subscribe();

        manager.notify_change("Email".to_string(), "state1".to_string());

        // Both should receive the change
        let change1 = rx1.try_recv().unwrap();
        let change2 = rx2.try_recv().unwrap();

        assert_eq!(change1.changed.get("Email"), Some(&"state1".to_string()));
        assert_eq!(change2.changed.get("Email"), Some(&"state1".to_string()));
    }

    #[test]
    fn test_state_change_serialization() {
        let mut changed = HashMap::new();
        changed.insert("Email".to_string(), "state123".to_string());
        changed.insert("Mailbox".to_string(), "state456".to_string());

        let state_change = StateChange { changed };

        let json = serde_json::to_string(&state_change).unwrap();
        assert!(json.contains("Email"));
        assert!(json.contains("state123"));
    }

    #[test]
    fn test_push_subscription_serialization() {
        let subscription = PushSubscription {
            url: "https://push.example.com/abc123".to_string(),
            types: Some(vec!["Email".to_string(), "Mailbox".to_string()]),
        };

        let json = serde_json::to_string(&subscription).unwrap();
        assert!(json.contains("push.example.com"));
    }

    #[test]
    fn test_event_source_manager_default() {
        let manager = EventSourceManager::default();
        assert!(manager.get_state("any").is_none());
    }

    #[test]
    fn test_event_source_manager_clone() {
        let manager1 = EventSourceManager::new();
        manager1.notify_change("Email".to_string(), "state1".to_string());

        let manager2 = manager1.clone();
        assert_eq!(manager2.get_state("Email"), Some("state1".to_string()));
    }

    #[test]
    fn test_get_nonexistent_state() {
        let manager = EventSourceManager::new();
        assert_eq!(manager.get_state("NonExistent"), None);
    }

    #[test]
    fn test_notify_empty_state() {
        let manager = EventSourceManager::new();
        manager.notify_change("Email".to_string(), "".to_string());

        assert_eq!(manager.get_state("Email"), Some("".to_string()));
    }

    #[test]
    fn test_subscribe_before_notify() {
        let manager = EventSourceManager::new();
        let mut rx = manager.subscribe();

        // No changes yet
        assert!(rx.try_recv().is_err());

        manager.notify_change("Email".to_string(), "state1".to_string());

        // Now should receive
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_subscribe_after_notify() {
        let manager = EventSourceManager::new();

        manager.notify_change("Email".to_string(), "state1".to_string());

        // Subscribe after notification
        let mut rx = manager.subscribe();

        // Won't receive past notifications
        assert!(rx.try_recv().is_err());

        // But state is still accessible
        assert_eq!(manager.get_state("Email"), Some("state1".to_string()));
    }

    #[test]
    fn test_multiple_data_types() {
        let manager = EventSourceManager::new();

        manager.notify_change("Email".to_string(), "email_state".to_string());
        manager.notify_change("Mailbox".to_string(), "mailbox_state".to_string());
        manager.notify_change("Thread".to_string(), "thread_state".to_string());
        manager.notify_change("Identity".to_string(), "identity_state".to_string());

        assert_eq!(manager.get_state("Email"), Some("email_state".to_string()));
        assert_eq!(
            manager.get_state("Mailbox"),
            Some("mailbox_state".to_string())
        );
        assert_eq!(
            manager.get_state("Thread"),
            Some("thread_state".to_string())
        );
        assert_eq!(
            manager.get_state("Identity"),
            Some("identity_state".to_string())
        );
    }

    #[test]
    fn test_state_change_empty_changed() {
        let state_change = StateChange {
            changed: HashMap::new(),
        };

        let json = serde_json::to_string(&state_change).unwrap();
        assert!(json.contains("changed"));
    }

    #[test]
    fn test_push_subscription_without_types() {
        let subscription = PushSubscription {
            url: "https://push.example.com/def456".to_string(),
            types: None,
        };

        let json = serde_json::to_string(&subscription).unwrap();
        assert!(!json.contains("types"));
    }

    #[test]
    fn test_concurrent_notifications() {
        let manager = EventSourceManager::new();
        let mut rx = manager.subscribe();

        // Send multiple notifications quickly
        for i in 0..10 {
            manager.notify_change(format!("Type{}", i), format!("state{}", i));
        }

        // Should receive all (or most, depending on timing)
        let mut received = 0;
        while rx.try_recv().is_ok() {
            received += 1;
        }

        assert!(received > 0);
    }

    #[test]
    fn test_state_persistence_across_notifications() {
        let manager = EventSourceManager::new();

        manager.notify_change("Email".to_string(), "state1".to_string());
        manager.notify_change("Mailbox".to_string(), "state2".to_string());

        // Update Email again
        manager.notify_change("Email".to_string(), "state3".to_string());

        // Email should have new state, Mailbox unchanged
        assert_eq!(manager.get_state("Email"), Some("state3".to_string()));
        assert_eq!(manager.get_state("Mailbox"), Some("state2".to_string()));
    }

    #[test]
    fn test_subscriber_receives_only_new_changes() {
        let manager = EventSourceManager::new();

        manager.notify_change("Email".to_string(), "old_state".to_string());

        let mut rx = manager.subscribe();

        manager.notify_change("Email".to_string(), "new_state".to_string());

        let change = rx.try_recv().unwrap();
        assert_eq!(change.changed.get("Email"), Some(&"new_state".to_string()));
    }

    #[test]
    fn test_broadcast_channel_capacity() {
        let manager = EventSourceManager::new();
        let mut rx = manager.subscribe();

        // Send more than channel capacity
        for i in 0..200 {
            manager.notify_change(format!("Type{}", i), format!("state{}", i));
        }

        // Receiver might lag
        let mut received = 0;
        let mut lagged = false;
        loop {
            match rx.try_recv() {
                Ok(_) => received += 1,
                Err(broadcast::error::TryRecvError::Lagged(_)) => {
                    // Expected when overwhelmed
                    lagged = true;
                    break;
                }
                Err(_) => break,
            }
        }

        // Either we received some messages or we lagged (which means channel was full)
        assert!(received > 0 || lagged);
    }

    #[test]
    fn test_state_change_deserialization() {
        let json = r#"{"changed":{"Email":"state1","Mailbox":"state2"}}"#;
        let state_change: StateChange = serde_json::from_str(json).unwrap();

        assert_eq!(
            state_change.changed.get("Email"),
            Some(&"state1".to_string())
        );
        assert_eq!(
            state_change.changed.get("Mailbox"),
            Some(&"state2".to_string())
        );
    }

    #[test]
    fn test_push_subscription_deserialization() {
        let json = r#"{"url":"https://example.com","types":["Email"]}"#;
        let subscription: PushSubscription = serde_json::from_str(json).unwrap();

        assert_eq!(subscription.url, "https://example.com");
        assert_eq!(subscription.types, Some(vec!["Email".to_string()]));
    }
}
