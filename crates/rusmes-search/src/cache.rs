//! LRU result cache for the search index.
//!
//! Caches `(normalized_query, user)` -> `Vec<MessageId>` pairs to short-circuit
//! repeated identical queries. Invalidation is global: every write to the index
//! bumps a version counter, and cache entries stamped with an older version are
//! treated as stale on lookup.
//!
//! The version-stamp approach was chosen over per-user walk-and-evict for two
//! reasons:
//! 1. `index_message` does not carry a user identity (the rusmes-search
//!    `SearchIndex` trait is decoupled from the storage account model), so
//!    walking the cache by user is not directly possible at the public API.
//! 2. A stale entry can otherwise miss a freshly-indexed message, breaking the
//!    primary correctness invariant of the cache.
//!
//! Capacity defaults to 256 entries.
//!
//! # Concurrency
//!
//! The internal `LruCache` is not `Sync` on its own (mutation requires `&mut`),
//! so it is wrapped in a `parking_lot`-style `std::sync::Mutex`. The version
//! counter is an `AtomicU64` so writers can bump it without taking the cache
//! lock.

use lru::LruCache;
use rusmes_proto::MessageId;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Default cache capacity (entries).
pub const DEFAULT_CAPACITY: usize = 256;

/// Cache key: normalized query string + user (or empty string for global).
pub type CacheKey = (String, String);

/// Cached value: list of matching message IDs plus the version stamp at insert
/// time. A lookup that finds an entry whose version is below the current
/// invalidation version treats the entry as stale.
#[derive(Clone, Debug)]
struct CacheValue {
    ids: Vec<MessageId>,
    version: u64,
}

/// LRU + version-stamped result cache.
pub struct ResultCache {
    inner: Mutex<LruCache<CacheKey, CacheValue>>,
    /// Monotonically increasing invalidation counter. Bumped on every write
    /// (`index_message` / `delete_message`).
    version: AtomicU64,
}

impl ResultCache {
    /// Create a new cache with the default capacity.
    pub fn new_default() -> Self {
        // `DEFAULT_CAPACITY` is non-zero; if a future change sets it to zero,
        // fall back to a 1-entry cache instead of panicking.
        let cap = NonZeroUsize::new(DEFAULT_CAPACITY).unwrap_or(NonZeroUsize::MIN);
        Self::with_capacity(cap)
    }

    /// Create a new cache with the given capacity.
    pub fn with_capacity(cap: NonZeroUsize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            version: AtomicU64::new(0),
        }
    }

    /// Normalize a query string: lowercase + collapse whitespace.
    pub fn normalize_query(query: &str) -> String {
        let lower = query.to_lowercase();
        lower.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Build a cache key from a query and an optional user.
    pub fn make_key(query: &str, user: Option<&str>) -> CacheKey {
        (Self::normalize_query(query), user.unwrap_or("").to_string())
    }

    /// Look up an entry. Returns `Some(ids)` only if the entry exists AND its
    /// stamp matches the current global version (i.e. it has not been
    /// invalidated by a write).
    pub fn get(&self, key: &CacheKey) -> Option<Vec<MessageId>> {
        let current = self.version.load(Ordering::Acquire);
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let value = guard.get(key)?;
        if value.version == current {
            Some(value.ids.clone())
        } else {
            // Stale: drop it now to free the slot.
            guard.pop(key);
            None
        }
    }

    /// Insert (or replace) an entry stamped at the current version.
    pub fn put(&self, key: CacheKey, ids: Vec<MessageId>) {
        let current = self.version.load(Ordering::Acquire);
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.put(
            key,
            CacheValue {
                ids,
                version: current,
            },
        );
    }

    /// Bump the invalidation version. Called after any successful index
    /// mutation (add or delete). All subsequent lookups will treat existing
    /// entries as stale until they are re-populated.
    pub fn invalidate_all(&self) {
        // wrapping_add so we never panic; the cycle is 2^64 invalidations.
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    /// Return the current number of entries in the cache.
    pub fn len(&self) -> usize {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.len()
    }

    /// Return the current invalidation version (test helper / observability).
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Whether the cache currently holds zero entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for ResultCache {
    fn default() -> Self {
        Self::new_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusmes_proto::MessageId;

    #[test]
    fn normalize_lowercases_and_collapses_whitespace() {
        let n = ResultCache::normalize_query("  Hello   WORLD\t\nfoo  ");
        assert_eq!(n, "hello world foo");
    }

    #[test]
    fn put_get_roundtrip_returns_ids() {
        let cache = ResultCache::new_default();
        let key = ResultCache::make_key("hello world", Some("alice"));
        let id1 = MessageId::new();
        let id2 = MessageId::new();
        cache.put(key.clone(), vec![id1, id2]);
        let hit = cache.get(&key).expect("entry should be present");
        assert_eq!(hit, vec![id1, id2]);
    }

    #[test]
    fn invalidate_all_makes_existing_entries_stale() {
        let cache = ResultCache::new_default();
        let key = ResultCache::make_key("q", None);
        cache.put(key.clone(), vec![MessageId::new()]);
        assert!(cache.get(&key).is_some());
        cache.invalidate_all();
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn key_is_user_aware() {
        let cache = ResultCache::new_default();
        let k_alice = ResultCache::make_key("foo", Some("alice"));
        let k_bob = ResultCache::make_key("foo", Some("bob"));
        let id = MessageId::new();
        cache.put(k_alice.clone(), vec![id]);
        assert!(cache.get(&k_alice).is_some());
        assert!(cache.get(&k_bob).is_none());
    }

    #[test]
    fn make_key_normalizes_query_text() {
        let k1 = ResultCache::make_key("Hello World", Some("u"));
        let k2 = ResultCache::make_key("hello   world", Some("u"));
        assert_eq!(k1, k2);
    }
}
