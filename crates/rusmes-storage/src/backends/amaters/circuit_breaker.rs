//! Circuit breaker for handling AmateRS node failures.

use std::sync::Arc;
use tokio::sync::RwLock;

/// Circuit breaker state for failover handling
#[derive(Debug, Clone)]
pub(super) enum CircuitBreakerState {
    Closed,
    Open { opened_at: std::time::Instant },
    HalfOpen,
}

/// Circuit breaker for handling node failures
pub(super) struct CircuitBreaker {
    pub(super) state: Arc<RwLock<CircuitBreakerState>>,
    pub(super) failure_count: Arc<RwLock<usize>>,
    pub(super) threshold: usize,
    pub(super) timeout_ms: u64,
}

impl CircuitBreaker {
    pub(super) fn new(threshold: usize, timeout_ms: u64) -> Self {
        Self {
            state: Arc::new(RwLock::new(CircuitBreakerState::Closed)),
            failure_count: Arc::new(RwLock::new(0)),
            threshold,
            timeout_ms,
        }
    }

    pub(super) async fn is_open(&self) -> bool {
        let state = self.state.read().await;
        matches!(*state, CircuitBreakerState::Open { .. })
    }

    pub(super) async fn record_success(&self) {
        let mut count = self.failure_count.write().await;
        *count = 0;
        let mut state = self.state.write().await;
        *state = CircuitBreakerState::Closed;
    }

    pub(super) async fn record_failure(&self) {
        let mut count = self.failure_count.write().await;
        *count += 1;

        if *count >= self.threshold {
            let mut state = self.state.write().await;
            *state = CircuitBreakerState::Open {
                opened_at: std::time::Instant::now(),
            };
        }
    }

    pub(super) async fn attempt_reset(&self) {
        let state = self.state.read().await;
        if let CircuitBreakerState::Open { opened_at } = *state {
            if opened_at.elapsed().as_millis() as u64 >= self.timeout_ms {
                drop(state);
                let mut state = self.state.write().await;
                *state = CircuitBreakerState::HalfOpen;
            }
        }
    }
}
