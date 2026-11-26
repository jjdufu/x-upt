//! Thread-safe replay nonce and Digest `nc` counter store with TTL eviction.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct NonceEntry {
    created: Instant,
    last_nc: u64,
}

/// Replay-protection nonce table isolated per protocol front.
#[derive(Debug)]
pub struct NonceStore {
    ttl: Duration,
    inner: Mutex<HashMap<String, NonceEntry>>,
}

impl NonceStore {
    /// Creates a new [`NonceStore`] with the given retention `ttl_seconds`.
    #[must_use]
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_seconds.max(1)),
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Checks whether `nonce` currently exists and has not expired.
    #[must_use]
    pub fn contains(&self, nonce: &str) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let ttl = self.ttl;
        guard.retain(|_, e| e.created.elapsed() < ttl);
        guard.contains_key(nonce)
    }

    /// Inserts `nonce` if absent; returns `false` when replayed within TTL.
    #[must_use]
    pub fn insert(&self, nonce: &str) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let ttl = self.ttl;
        guard.retain(|_, e| e.created.elapsed() < ttl);
        if guard.contains_key(nonce) {
            return false;
        }
        guard.insert(
            nonce.to_string(),
            NonceEntry {
                created: Instant::now(),
                last_nc: 0,
            },
        );
        true
    }

    /// Validates and advances RFC 7616 Digest `nc` strictly monotonically.
    #[must_use]
    pub fn accept_nc(&self, nonce: &str, nc: u64) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let ttl = self.ttl;
        guard.retain(|_, e| e.created.elapsed() < ttl);
        let Some(entry) = guard.get_mut(nonce) else {
            return false;
        };
        if nc <= entry.last_nc {
            return false;
        }
        entry.last_nc = nc;
        true
    }
}
