use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::types::token::{AuthedApp, TokenEntry, TokenResponse};

pub struct TokenManager {
    expire_minutes: u64,
    max_per_client: u64,
    store: Arc<RwLock<HashMap<String, TokenEntry>>>,
}

pub enum IssueError {
    TooMany,
}

impl TokenManager {
    #[must_use]
    pub fn new(expire_minutes: u64, cleanup_interval_seconds: u64, max_per_client: u64) -> Self {
        let store: Arc<RwLock<HashMap<String, TokenEntry>>> = Arc::new(RwLock::new(HashMap::new()));
        let manager = Self {
            expire_minutes,
            max_per_client,
            store,
        };

        let cleanup_store = Arc::clone(&manager.store);
        tokio::spawn(async move {
            Self::cleanup_loop(cleanup_store, cleanup_interval_seconds).await;
        });

        manager
    }

    pub async fn create_token(
        &self,
        client_id: String,
        methods: HashSet<String>,
    ) -> Result<TokenResponse, IssueError> {
        let access_token = Uuid::new_v4().to_string();
        let now = Utc::now();
        let minutes = i64::try_from(self.expire_minutes).unwrap_or(i64::MAX);
        let expires_at = now
            + chrono::Duration::try_minutes(minutes)
                .unwrap_or_else(|| chrono::Duration::minutes(i64::MAX / 2));

        let mut scope_list: Vec<String> = methods.iter().cloned().collect();
        scope_list.sort();
        let scope = scope_list.join(" ");

        let entry = TokenEntry {
            client_id: client_id.clone(),
            methods,
            expires_at,
        };

        let mut store = self.store.write().await;
        if self.max_per_client > 0 {
            let count = store
                .values()
                .filter(|e| e.client_id == client_id && e.expires_at >= now)
                .count();
            if count >= usize::try_from(self.max_per_client).unwrap_or(usize::MAX) {
                return Err(IssueError::TooMany);
            }
        }
        store.insert(access_token.clone(), entry);

        Ok(TokenResponse {
            access_token,
            token_type: "Bearer".to_string(),
            expires_in: self.expire_minutes * 60,
            scope,
        })
    }

    pub async fn validate_token(&self, token: &str) -> Option<AuthedApp> {
        {
            let store = self.store.read().await;
            let entry = store.get(token)?;
            if entry.expires_at >= Utc::now() {
                return Some(AuthedApp {
                    client_id: entry.client_id.clone(),
                    methods: entry.methods.clone(),
                });
            }
        }
        let mut store = self.store.write().await;
        if let Some(entry) = store.get(token)
            && entry.expires_at < Utc::now()
        {
            store.remove(token);
        }
        None
    }

    async fn cleanup_loop(store: Arc<RwLock<HashMap<String, TokenEntry>>>, interval_seconds: u64) {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(interval_seconds)).await;
            let mut store = store.write().await;
            let now = Utc::now();
            store.retain(|_, entry| entry.expires_at >= now);
        }
    }
}
