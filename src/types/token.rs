use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Deserialize, Default)]
pub struct TokenForm {
    #[serde(default)]
    pub grant_type: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub scope: String,
}

#[derive(Debug, Serialize)]
pub struct OAuthError {
    pub error: String,
    pub error_description: String,
}

#[derive(Debug, Clone)]
pub struct TokenEntry {
    pub client_id: String,
    pub methods: HashSet<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AuthedApp {
    pub client_id: String,
    pub methods: HashSet<String>,
}
