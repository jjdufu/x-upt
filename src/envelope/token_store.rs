use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;

#[derive(Clone)]
struct Entry {
    expires: Instant,
}

#[derive(Clone)]
pub struct EnvelopeTokenStore {
    expire: Duration,
    inner: Arc<Mutex<HashMap<String, Entry>>>,
}

impl EnvelopeTokenStore {
    #[must_use]
    pub fn new(expire_minutes: u64, cleanup_interval_seconds: u64) -> Self {
        let inner = Arc::new(Mutex::new(HashMap::new()));
        let cleanup = Arc::clone(&inner);
        let interval = cleanup_interval_seconds.max(1);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(interval)).await;
                retain_fresh(&cleanup);
            }
        });
        Self {
            expire: Duration::from_secs(expire_minutes.saturating_mul(60).max(1)),
            inner,
        }
    }

    #[must_use]
    pub fn issue(&self) -> (String, u64) {
        let mut bytes = [0u8; 32];
        rand::fill(&mut bytes);
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let expires = Instant::now() + self.expire;
        self.lock().insert(token.clone(), Entry { expires });
        (token, self.expire.as_secs())
    }

    #[must_use]
    pub fn contains(&self, token: &str) -> bool {
        let mut guard = self.lock();
        let Some(entry) = guard.get(token) else {
            return false;
        };
        if entry.expires <= Instant::now() {
            guard.remove(token);
            return false;
        }
        true
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn retain_fresh(inner: &Mutex<HashMap<String, Entry>>) {
    let mut guard = inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = Instant::now();
    guard.retain(|_, entry| entry.expires > now);
}

#[cfg(test)]
mod tests {
    use super::EnvelopeTokenStore;

    #[tokio::test]
    async fn token_does_not_cross_stores() {
        let left = EnvelopeTokenStore::new(30, 300);
        let right = EnvelopeTokenStore::new(30, 300);
        let (token, expires) = left.issue();
        assert!(expires >= 32);
        assert!(token.len() >= 32);
        assert!(left.contains(&token));
        assert!(!right.contains(&token));
        assert!(!left.contains("missing"));
    }
}
