pub mod aws;
pub mod digest;
pub mod hawk;
pub mod jwt;
pub mod nonce;
pub mod oauth;
pub mod sign;
pub mod token;
pub mod token_hub;

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, header};
use base64::Engine;
use http_body_util::BodyExt;

use crate::auth::nonce::NonceStore;
use crate::auth::token::TokenManager;
use crate::config::AuthMethod;
use crate::service::apps::{AppRegistry, secrets_equal};
use crate::types::token::AuthedApp;

#[derive(Clone)]
pub struct AuthPolicy {
    pub methods: Vec<AuthMethod>,
    pub tokens: Option<Arc<TokenManager>>,
    pub apps: Arc<AppRegistry>,
    pub digest_nonces: Arc<NonceStore>,
    pub hawk_nonces: Arc<NonceStore>,
    pub listen_port: u16,
}

#[derive(Debug)]
pub enum AuthError {
    Missing,
    InvalidFormat,
    InvalidToken,
    DigestChallenge(Vec<String>),
}

pub struct AuthOk {
    pub app: AuthedApp,
    pub method: AuthMethod,
}

pub struct AuthInput<'a> {
    pub headers: &'a HeaderMap,
    pub query: Option<&'a str>,
    pub method: &'a str,
    pub path: &'a str,
    pub body: &'a [u8],
    pub query_overrides_header: bool,
}

impl AuthPolicy {
    #[must_use]
    pub fn allows(&self, method: AuthMethod) -> bool {
        self.methods.contains(&method)
    }

    #[must_use]
    pub fn uses_token_bucket(&self) -> bool {
        self.allows(AuthMethod::OauthCc) || self.allows(AuthMethod::Gettoken)
    }
}

pub fn policy_from(
    methods: Vec<AuthMethod>,
    tokens: Option<Arc<TokenManager>>,
    apps: Arc<AppRegistry>,
    listen_port: u16,
) -> AuthPolicy {
    AuthPolicy {
        methods,
        tokens,
        apps,
        digest_nonces: Arc::new(NonceStore::new(600)),
        hawk_nonces: Arc::new(NonceStore::new(600)),
        listen_port,
    }
}

pub async fn authenticate_request(
    policy: &AuthPolicy,
    req: Request,
    query_overrides_header: bool,
) -> Result<(AuthOk, Request), AuthError> {
    let (parts, body) = req.into_parts();
    let collected = body.collect().await.map_err(|_| AuthError::InvalidFormat)?;
    let bytes = collected.to_bytes();
    let query = parts.uri.query().map(ToOwned::to_owned);
    let input = AuthInput {
        headers: &parts.headers,
        query: query.as_deref(),
        method: parts.method.as_str(),
        path: parts.uri.path(),
        body: &bytes,
        query_overrides_header,
    };
    let ok = authenticate(policy, &input).await?;
    let req = Request::from_parts(parts, Body::from(bytes));
    Ok((ok, req))
}

pub async fn authenticate(policy: &AuthPolicy, input: &AuthInput<'_>) -> Result<AuthOk, AuthError> {
    if policy.allows(AuthMethod::None) {
        return unauthenticated_ok(policy);
    }
    if policy.methods.is_empty() {
        return Err(AuthError::Missing);
    }

    let merged = merge_credentials(input);
    let presented = credential_presented(&merged);

    if let Some(value) = merged.authorization.as_deref() {
        if let Some(rest) = value.strip_prefix("Hawk ") {
            if policy.allows(AuthMethod::Hawk) {
                return hawk_ok(policy, input, value)
                    .map(|app| AuthOk {
                        app,
                        method: AuthMethod::Hawk,
                    })
                    .ok_or(AuthError::InvalidToken);
            }
            let _ = rest;
        } else if value.starts_with("Digest ") || value.starts_with("digest ") {
            if policy.allows(AuthMethod::Digest) {
                return digest_ok(policy, input, value)
                    .map(|app| AuthOk {
                        app,
                        method: AuthMethod::Digest,
                    })
                    .ok_or(AuthError::InvalidToken);
            }
        } else if value.starts_with("AWS4-HMAC-SHA256 ") {
            if policy.allows(AuthMethod::AwsSigv4) {
                return aws_ok(policy, input, value)
                    .map(|app| AuthOk {
                        app,
                        method: AuthMethod::AwsSigv4,
                    })
                    .ok_or(AuthError::InvalidToken);
            }
        } else if let Some(token) = value.strip_prefix("Bearer ") {
            return bearer_ok(policy, token.trim()).await;
        } else if value.starts_with("Basic ") {
            if !policy.allows(AuthMethod::Basic) {
                return Err(AuthError::InvalidFormat);
            }
            return basic_ok(policy, value)
                .map(|app| AuthOk {
                    app,
                    method: AuthMethod::Basic,
                })
                .ok_or(AuthError::InvalidToken);
        } else if presented {
            return Err(AuthError::InvalidFormat);
        }
    }

    if policy.allows(AuthMethod::ApiKey)
        && let Some(key) = merged.api_key.as_deref()
    {
        return policy
            .apps
            .find_by_secret(key)
            .map(|app| AuthOk {
                app: AuthedApp::from(app),
                method: AuthMethod::ApiKey,
            })
            .ok_or(AuthError::InvalidToken);
    }

    if !presented && policy.allows(AuthMethod::Digest) {
        return Err(AuthError::DigestChallenge(digest::challenge_headers(
            &policy.digest_nonces,
        )));
    }

    Err(AuthError::Missing)
}

struct MergedCreds {
    authorization: Option<String>,
    api_key: Option<String>,
}

fn merge_credentials(input: &AuthInput<'_>) -> MergedCreds {
    let query = query_map(input.query.unwrap_or(""));
    let header_auth = input
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let header_key = header_api_key(input.headers).map(ToOwned::to_owned);
    let q_auth = query_first(&query, &["authorization", "access_token"]);
    let q_key = query_first(&query, &["api_key", "x-api-key"]);

    let authorization = if input.query_overrides_header {
        match q_auth {
            Some(token) if query.contains_key("access_token") => Some(format!("Bearer {token}")),
            Some(raw) => Some(raw),
            None => header_auth,
        }
    } else {
        header_auth.or_else(|| {
            if query.contains_key("access_token") {
                q_auth.map(|t| format!("Bearer {t}"))
            } else {
                q_auth
            }
        })
    };

    let api_key = if input.query_overrides_header {
        q_key.or(header_key)
    } else {
        header_key.or(q_key)
    };

    MergedCreds {
        authorization,
        api_key,
    }
}

fn query_map(raw: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(raw.as_bytes())
        .map(|(k, v)| (k.to_ascii_lowercase(), v.into_owned()))
        .collect()
}

fn query_first(map: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = map.get(*key).map(String::as_str).filter(|s| !s.is_empty()) {
            return Some(v.to_string());
        }
    }
    None
}

fn credential_presented(merged: &MergedCreds) -> bool {
    merged.authorization.is_some() || merged.api_key.is_some()
}

fn unauthenticated_ok(policy: &AuthPolicy) -> Result<AuthOk, AuthError> {
    let Some(app) = policy.apps.iter().next() else {
        return Err(AuthError::Missing);
    };
    Ok(AuthOk {
        app: AuthedApp::from(app),
        method: AuthMethod::None,
    })
}

fn token_bucket_method(policy: &AuthPolicy) -> AuthMethod {
    if policy.allows(AuthMethod::OauthCc) {
        AuthMethod::OauthCc
    } else if policy.allows(AuthMethod::Gettoken) {
        AuthMethod::Gettoken
    } else {
        AuthMethod::Bearer
    }
}

async fn bearer_ok(policy: &AuthPolicy, token: &str) -> Result<AuthOk, AuthError> {
    if policy.uses_token_bucket()
        && let Some(tokens) = &policy.tokens
        && let Some(found) = tokens.validate_token(token).await
    {
        return Ok(AuthOk {
            app: found,
            method: token_bucket_method(policy),
        });
    }
    if policy.allows(AuthMethod::Jwt) && jwt::looks_like_jwt(token) {
        return jwt::verify(&policy.apps, token)
            .map(|app| AuthOk {
                app,
                method: AuthMethod::Jwt,
            })
            .ok_or(AuthError::InvalidToken);
    }
    if policy.allows(AuthMethod::ApiKey) || policy.allows(AuthMethod::Bearer) {
        if let Some(app) = policy.apps.find_by_secret(token) {
            let method = if policy.allows(AuthMethod::Bearer) {
                AuthMethod::Bearer
            } else {
                AuthMethod::ApiKey
            };
            return Ok(AuthOk {
                app: AuthedApp::from(app),
                method,
            });
        }
        if policy.allows(AuthMethod::Bearer) && policy.uses_token_bucket() {
            return Err(AuthError::InvalidToken);
        }
        if policy.allows(AuthMethod::ApiKey) {
            return Err(AuthError::InvalidToken);
        }
    }
    if policy.allows(AuthMethod::Jwt) {
        return jwt::verify(&policy.apps, token)
            .map(|app| AuthOk {
                app,
                method: AuthMethod::Jwt,
            })
            .ok_or(AuthError::InvalidToken);
    }
    Err(AuthError::InvalidToken)
}

fn basic_ok(policy: &AuthPolicy, value: &str) -> Option<AuthedApp> {
    let (id, secret) = parse_basic(value)?;
    let found = policy.apps.get(&id)?;
    if !secrets_equal(&found.client_secret, &secret) {
        return None;
    }
    Some(AuthedApp::from(found))
}

fn hawk_ok(policy: &AuthPolicy, input: &AuthInput<'_>, value: &str) -> Option<AuthedApp> {
    let host_header = input
        .headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    let (host, port) = hawk::host_port(host_header, policy.listen_port);
    if host.is_empty() {
        return None;
    }
    let resource = hawk::resource_from_uri(input.path, input.query);
    let ct = input
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    hawk::verify(
        &policy.apps,
        &policy.hawk_nonces,
        value,
        &hawk::HawkParts {
            method: input.method,
            host: &host,
            port,
            resource: &resource,
            content_type: ct,
            body: input.body,
        },
    )
}

fn digest_ok(policy: &AuthPolicy, input: &AuthInput<'_>, value: &str) -> Option<AuthedApp> {
    let uri = hawk::resource_from_uri(input.path, input.query);
    digest::verify(
        &policy.apps,
        &policy.digest_nonces,
        value,
        input.method,
        &uri,
    )
}

fn aws_ok(policy: &AuthPolicy, input: &AuthInput<'_>, value: &str) -> Option<AuthedApp> {
    let host = input
        .headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    aws::verify(
        &policy.apps,
        value,
        &aws::AwsRequest {
            method: input.method,
            path: input.path,
            query: input.query.unwrap_or(""),
            host,
            headers: input.headers,
            body: input.body,
        },
    )
}

#[must_use]
pub fn header_api_key(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[must_use]
pub fn parse_basic(value: &str) -> Option<(String, String)> {
    let encoded = value.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let pair = String::from_utf8(decoded).ok()?;
    let (id, secret) = pair.split_once(':')?;
    if id.is_empty() || secret.is_empty() {
        return None;
    }
    Some((id.to_string(), secret.to_string()))
}

#[must_use]
pub fn credentials_from_headers_or_form(
    headers: &HeaderMap,
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Option<(String, String)> {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(basic) = parse_basic(value)
    {
        return Some(basic);
    }
    match (client_id.map(str::trim), client_secret) {
        (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
            Some((id.to_string(), secret.to_string()))
        }
        _ => None,
    }
}

pub fn attach_digest_headers(headers: &mut HeaderMap, challenges: &[String]) {
    for item in challenges {
        if let Ok(value) = HeaderValue::from_str(item) {
            headers.append(header::WWW_AUTHENTICATE, value);
        }
    }
}
