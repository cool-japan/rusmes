//! Maildrop locking for POP3 (RFC 1939 §3)
//!
//! POP3 mandates that during a Transaction state, the maildrop is *exclusively*
//! locked so that another concurrent session for the same user cannot mutate the
//! maildrop concurrently. From RFC 1939 §3:
//!
//! > Once the POP3 server has determined through the use of any authentication
//! > command that the client should be given access to the appropriate maildrop,
//! > the POP3 server then acquires an exclusive-access lock on the maildrop, as
//! > necessary to prevent messages from being modified or removed before the
//! > session enters the UPDATE state.  If the lock is successfully acquired, the
//! > POP3 server responds with a positive status indicator.  If the lock can not
//! > be acquired (...), the POP3 server responds with a negative status indicator.
//!
//! This module provides a per-user mutex map keyed by username. A successful
//! `try_acquire(user)` returns a [`MaildropGuard`] that releases the lock on
//! drop (RAII), so the lock is correctly released even if the session panics or
//! is dropped mid-transaction.
//!
//! The implementation uses `tokio::sync::Mutex` rather than `parking_lot::Mutex`
//! because the lock may be held across `await` points within a session (e.g.
//! waiting on storage backend I/O while in Transaction state).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Manages per-user exclusive maildrop locks.
///
/// Cheap to clone (internally `Arc`).
#[derive(Clone, Default)]
pub struct MaildropLockManager {
    inner: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl MaildropLockManager {
    /// Create a new, empty lock manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to acquire an exclusive lock on the given user's maildrop.
    ///
    /// Returns `Some(MaildropGuard)` on success, or `None` if another active
    /// session already holds the lock for this user.
    ///
    /// The username comparison is case-sensitive — POP3 traditionally treats
    /// usernames as case-sensitive, and any case-folding policy should be
    /// applied by the caller before invoking this method.
    pub async fn try_acquire(&self, user: &str) -> Option<MaildropGuard> {
        // Phase 1: get-or-insert the per-user mutex under the registry lock.
        // The registry lock is only held for this short critical section.
        let user_mutex = {
            let mut map = self.inner.lock().await;
            map.entry(user.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };

        // Phase 2: try (non-blocking) to acquire the per-user mutex via
        // OwnedMutexGuard, which does not borrow `user_mutex` and therefore
        // can outlive this function (held inside the returned guard).
        match user_mutex.try_lock_owned() {
            Ok(guard) => Some(MaildropGuard {
                _guard: guard,
                user: user.to_string(),
            }),
            Err(_) => None,
        }
    }

    /// Number of users currently registered (not necessarily locked).
    /// Exposed for diagnostics / tests.
    pub async fn registered_user_count(&self) -> usize {
        self.inner.lock().await.len()
    }
}

/// RAII guard for an acquired maildrop lock.
///
/// The lock is released when this guard is dropped (which happens on QUIT, on
/// explicit session-state teardown, or on panic).
pub struct MaildropGuard {
    _guard: OwnedMutexGuard<()>,
    user: String,
}

impl MaildropGuard {
    /// The user whose maildrop is locked.
    pub fn user(&self) -> &str {
        &self.user
    }
}

impl std::fmt::Debug for MaildropGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaildropGuard")
            .field("user", &self.user)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_first_acquire_succeeds() {
        let mgr = MaildropLockManager::new();
        let g = mgr.try_acquire("alice").await;
        assert!(g.is_some(), "first acquire for alice should succeed");
    }

    #[tokio::test]
    async fn test_second_concurrent_acquire_for_same_user_fails() {
        let mgr = MaildropLockManager::new();
        let g1 = mgr.try_acquire("alice").await;
        assert!(g1.is_some());

        // Second acquisition while g1 is alive must fail.
        let g2 = mgr.try_acquire("alice").await;
        assert!(g2.is_none(), "second acquire while first is held must fail");
    }

    #[tokio::test]
    async fn test_release_allows_subsequent_acquire() {
        let mgr = MaildropLockManager::new();
        {
            let g1 = mgr.try_acquire("alice").await;
            assert!(g1.is_some());
            // g1 goes out of scope at the end of this block — drop releases it.
        }
        let g2 = mgr.try_acquire("alice").await;
        assert!(
            g2.is_some(),
            "after first guard dropped, re-acquire must succeed"
        );
    }

    #[tokio::test]
    async fn test_different_users_independent() {
        let mgr = MaildropLockManager::new();
        let ga = mgr.try_acquire("alice").await;
        let gb = mgr.try_acquire("bob").await;
        assert!(
            ga.is_some() && gb.is_some(),
            "different users must not block each other"
        );
    }

    #[tokio::test]
    async fn test_guard_records_user() {
        let mgr = MaildropLockManager::new();
        let g = mgr.try_acquire("alice").await.expect("first acquire");
        assert_eq!(g.user(), "alice");
    }

    #[tokio::test]
    async fn test_clone_shares_state() {
        let mgr = MaildropLockManager::new();
        let mgr2 = mgr.clone();
        let g1 = mgr.try_acquire("alice").await;
        assert!(g1.is_some());
        let g2 = mgr2.try_acquire("alice").await;
        assert!(g2.is_none(), "cloned manager must share lock state");
    }
}
