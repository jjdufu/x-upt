use std::collections::HashMap;
use std::sync::Arc;

use crate::auth::token::TokenManager;
use crate::config::{AppConfig, FrontName, needs_token_bucket};

pub struct TokenHub {
    buckets: HashMap<FrontName, Arc<TokenManager>>,
}

impl TokenHub {
    #[must_use]
    pub fn new(config: &AppConfig) -> Self {
        let mut buckets = HashMap::new();
        for name in config.fronts.enabled_names() {
            let common = config.fronts.common(name);
            if !needs_token_bucket(name, common) {
                continue;
            }
            buckets.insert(
                name,
                Arc::new(TokenManager::new(
                    common.expire_minutes,
                    config.token.cleanup_interval_seconds,
                    config.token.max_per_client,
                )),
            );
        }
        Self { buckets }
    }

    #[must_use]
    pub fn bucket(&self, name: FrontName) -> Option<Arc<TokenManager>> {
        self.buckets.get(&name).cloned()
    }
}
