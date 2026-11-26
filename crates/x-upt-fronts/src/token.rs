//! Per-front isolated OAuth / Gettoken / Envelope token and replay-nonce state.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use axum::response::{IntoResponse, Response};
use chrono::{Duration as ChronoDuration, Utc};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use uuid::Uuid;
use x_upt_core::domain::{
    AuthedApp, FrontName, OAuthError, TokenEntry, TokenForm, TokenResponse, granted_methods,
    secrets_equal,
};
use x_upt_crypto::http_auth::parse_basic;
use x_upt_crypto::nonce::NonceStore;
use x_upt_crypto::pipeline::EnvelopeTokenStore;

use crate::snapshot::SharedRuntime;

/// Per-front OAuth 2.0 / `gettoken` token bucket.
#[derive(Debug, Default)]
pub struct TokenBucket {
    entries: Mutex<HashMap<String, TokenEntry>>,
}

impl TokenBucket {
    /// Issues a new bearer token for `client_id` with `methods`, enforcing `max_per_client`.
    #[must_use]
    pub fn issue(
        &self,
        client_id: &str,
        methods: HashSet<String>,
        expire_minutes: u64,
        max_per_client: u64,
    ) -> (String, u64) {
        let now = Utc::now();
        let ttl_secs = expire_minutes.max(1).saturating_mul(60);
        let ttl_i64 = i64::try_from(ttl_secs).unwrap_or(1800);
        let expires_at = now + ChronoDuration::seconds(ttl_i64);
        let token = format!("tok_{}", Uuid::new_v4().simple());

        let mut map = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|_, v| v.expires_at > now);
        if max_per_client > 0 {
            let cap = usize::try_from(max_per_client).unwrap_or(usize::MAX);
            let mut client_tokens: Vec<(String, chrono::DateTime<Utc>)> = map
                .iter()
                .filter(|(_, v)| v.client_id == client_id)
                .map(|(k, v)| (k.clone(), v.expires_at))
                .collect();
            client_tokens.sort_by_key(|(_, exp)| *exp);
            while client_tokens.len() >= cap {
                let Some((oldest, _)) = client_tokens.first().cloned() else {
                    break;
                };
                map.remove(&oldest);
                client_tokens.remove(0);
            }
        }
        map.insert(
            token.clone(),
            TokenEntry {
                client_id: client_id.to_string(),
                methods,
                expires_at,
            },
        );
        (token, ttl_secs)
    }

    /// Resolves a bearer token into [`AuthedApp`] if present and unexpired.
    #[must_use]
    pub fn verify(&self, token: &str) -> Option<AuthedApp> {
        let now = Utc::now();
        let mut map = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = map.get(token) {
            if entry.expires_at > now {
                return Some(AuthedApp {
                    client_id: entry.client_id.clone(),
                    methods: entry.methods.clone(),
                });
            }
            map.remove(token);
        }
        None
    }
}

/// Isolated cryptographic state bundle for a single protocol front.
#[derive(Debug, Clone)]
pub struct FrontSecurityState {
    /// Core OAuth / Gettoken bucket.
    pub tokens: Arc<TokenBucket>,
    /// Envelope token store (`envelope_token_*`).
    pub envelope_tokens: Arc<EnvelopeTokenStore>,
    /// Digest challenge/nc nonce store.
    pub digest_nonces: Arc<NonceStore>,
    /// Hawk `(id, ts, nonce)` replay store.
    pub hawk_nonces: Arc<NonceStore>,
    /// `sign_gateway` and `envelope_*` replay nonce store.
    pub sign_nonces: Arc<NonceStore>,
}

impl FrontSecurityState {
    /// Creates fresh isolated stores for one front.
    #[must_use]
    pub fn new(expire_minutes: u64, nonce_ttl_seconds: u64) -> Self {
        Self {
            tokens: Arc::new(TokenBucket::default()),
            envelope_tokens: Arc::new(EnvelopeTokenStore::new(expire_minutes)),
            digest_nonces: Arc::new(NonceStore::new(nonce_ttl_seconds)),
            hawk_nonces: Arc::new(NonceStore::new(nonce_ttl_seconds)),
            sign_nonces: Arc::new(NonceStore::new(nonce_ttl_seconds)),
        }
    }
}

/// Handles RFC 6749 `POST /oauth/token` and `POST /v3/oauth/token` for `front`.
#[must_use]
pub fn handle_oauth_token(
    front: FrontName,
    runtime: &SharedRuntime,
    bucket: &TokenBucket,
    headers: &HeaderMap,
    form_bytes: &[u8],
) -> (StatusCode, Response, Option<String>) {
    let snap = runtime.load();
    let mut form = TokenForm::default();
    for (k, v) in url::form_urlencoded::parse(form_bytes).into_owned() {
        match k.as_str() {
            "grant_type" => form.grant_type = v,
            "client_id" => form.client_id = Some(v),
            "client_secret" => form.client_secret = Some(v),
            "scope" => form.scope = Some(v),
            _ => {}
        }
    }

    if form.grant_type != "client_credentials" {
        return oauth_err(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type must be client_credentials",
            false,
        );
    }

    let creds = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_basic)
        .or_else(|| Some((form.client_id?, form.client_secret?)));

    let Some((client_id, client_secret)) = creds else {
        return oauth_err(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "missing client credentials",
            true,
        );
    };

    let Some(app) = snap
        .apps
        .get(&client_id)
        .filter(|a| secrets_equal(&a.client_secret, &client_secret))
    else {
        return oauth_err(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "invalid client_id or client_secret",
            true,
        );
    };

    let mut allowed = granted_methods(&app.methods, snap.dataset.methods);
    if let Some(raw_scope) = form
        .scope
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let requested: HashSet<String> = raw_scope
            .split_whitespace()
            .map(ToString::to_string)
            .collect();
        if !requested.is_subset(&allowed) {
            return oauth_err(
                StatusCode::BAD_REQUEST,
                "invalid_scope",
                "requested scope exceeds granted/enabled methods",
                false,
            );
        }
        allowed = requested;
    }

    let expire_mins = snap
        .fronts
        .get(&front)
        .map_or(snap.settings.token_expire_minutes, |f| f.expire_minutes);
    let (access_token, expires_in) = bucket.issue(
        &app.client_id,
        allowed.clone(),
        expire_mins,
        snap.settings.token_max_per_client,
    );
    let mut scope_vec: Vec<String> = allowed.into_iter().collect();
    scope_vec.sort();
    let body = TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in,
        scope: scope_vec.join(" "),
    };
    let mut resp = axum::Json(body).into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp.headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    (StatusCode::OK, resp, Some(app.client_id.clone()))
}

fn oauth_err(
    status: StatusCode,
    error: &str,
    desc: &str,
    basic_challenge: bool,
) -> (StatusCode, Response, Option<String>) {
    let mut resp = (
        status,
        axum::Json(OAuthError {
            error: error.to_string(),
            error_description: desc.to_string(),
        }),
    )
        .into_response();
    if basic_challenge && let Ok(hv) = HeaderValue::from_str(r#"Basic realm="x-upt""#) {
        resp.headers_mut().insert(header::WWW_AUTHENTICATE, hv);
    }
    (status, resp, None)
}
