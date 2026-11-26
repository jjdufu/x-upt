use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct NonceEntry {
    created: Instant,
    last_nc: u64,
}

pub struct NonceStore {
    ttl: Duration,
    inner: Mutex<HashMap<String, NonceEntry>>,
}

impl NonceStore {
    #[must_use]
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_seconds.max(1)),
            inner: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn contains(&self, nonce: &str) -> bool {
        self.gc();
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.contains_key(nonce)
    }

    pub fn insert(&self, nonce: &str) -> bool {
        self.gc();
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    #[must_use]
    pub fn accept_nc(&self, nonce: &str, nc: u64) -> bool {
        self.gc();
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = guard.get_mut(nonce) else {
            return false;
        };
        if nc <= entry.last_nc {
            return false;
        }
        entry.last_nc = nc;
        true
    }

    fn gc(&self) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ttl = self.ttl;
        guard.retain(|_, e| e.created.elapsed() < ttl);
    }
}
