//! Lock-free-across-await runtime snapshot holder and simulated latency helper.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use rand::RngExt;
use x_upt_core::domain::GlobalSettings;
use x_upt_store::dto::RuntimeSnapshotData;

/// Thread-safe handle to the current immutable [`RuntimeSnapshotData`].
///
/// Readers clone the inner [`Arc`] and release the lock immediately before any `.await`.
#[derive(Debug, Clone)]
pub struct SharedRuntime {
    inner: Arc<RwLock<Arc<RuntimeSnapshotData>>>,
}

impl SharedRuntime {
    /// Wraps an initial snapshot.
    #[must_use]
    pub fn new(initial: RuntimeSnapshotData) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(initial))),
        }
    }

    /// Loads a cheap [`Arc`] clone of the current runtime snapshot.
    #[must_use]
    pub fn load(&self) -> Arc<RuntimeSnapshotData> {
        let guard = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(&guard)
    }

    /// Atomically swaps in a freshly rebuilt snapshot.
    pub fn store(&self, next: RuntimeSnapshotData) {
        let mut guard = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        *guard = Arc::new(next);
    }
}

/// Sleeps for a random duration within `[latency_min_ms, latency_max_ms]` and returns elapsed ms.
pub async fn simulate_latency(settings: &GlobalSettings) -> u64 {
    let min_ms = settings.latency_min_ms;
    let max_ms = settings.latency_max_ms.max(min_ms);
    if max_ms == 0 {
        return 0;
    }
    let ms = if min_ms == max_ms {
        min_ms
    } else {
        rand::rng().random_range(min_ms..=max_ms)
    };
    if ms > 0 {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    ms
}
