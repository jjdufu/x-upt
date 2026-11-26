//! Unified HTTP authentication dispatcher and `sign_gateway` HMAC-SHA256 helpers.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use base64::Engine;
use http::{HeaderMap, HeaderValue, header};
use x_upt_core::domain::{
    AppRegistry, AuthMethod, AuthedApp, BusinessMethod, secrets_equal, secrets_equal_bytes,
};

use crate::hawk_aws::{
    AwsRequest, HawkParts, hmac_sha256, host_port, resource_from_uri, verify_aws_sigv4, verify_hawk,
};
use crate::jwt_digest::{digest_challenges, looks_like_jwt, verify_digest, verify_jwt};
use crate::nonce::NonceStore;

/// Authentication error returned by [`authenticate`].
#[derive(Debug, Clone)]
pub enum AuthError {
    /// Missing credentials.
    Missing,
    /// Malformed Authorization header.
    InvalidFormat,
    /// Invalid or expired token/signature/secret.
    InvalidToken,
    /// Digest 401 challenge headers to emit.
    DigestChallenge(Vec<String>),
}

/// Successful authentication result.
#[derive(Debug, Clone)]
pub struct AuthOk {
    /// Authenticated caller identity & permissions.
    pub app: AuthedApp,
    /// Matched authentication mode.
    pub method: AuthMethod,
}

/// Request view passed to [`authenticate`].
pub struct AuthInput<'a> {
    /// HTTP headers.
    pub headers: &'a HeaderMap,
    /// Optional raw query string.
    pub query: Option<&'a str>,
    /// HTTP method string.
    pub method: &'a str,
    /// Request path.
    pub path: &'a str,
    /// Raw request body bytes.
    pub body: &'a [u8],
    /// Whether query credentials override headers (`true` on WebSocket upgrade).
    pub query_overrides_header: bool,
}

/// Policy and per-front nonce state for authenticating core HTTP/gRPC/WS fronts.
#[derive(Clone)]
pub struct AuthPolicy {
    /// Allowed authentication modes on this front.
    pub methods: Vec<AuthMethod>,
    /// Snapshot of registered caller applications.
    pub apps: Arc<AppRegistry>,
    /// Digest nonce store.
    pub digest_nonces: Arc<NonceStore>,
    /// Hawk nonce store.
    pub hawk_nonces: Arc<NonceStore>,
    /// Front listening TCP port.
    pub listen_port: u16,
}

impl AuthPolicy {
    /// Checks whether `method` is permitted on this front.
    #[must_use]
    pub fn allows(&self, method: AuthMethod) -> bool {
        self.methods.contains(&method)
    }

    /// Whether OAuth or Gettoken bucket lookup applies.
    #[must_use]
    pub fn uses_token_bucket(&self) -> bool {
        self.allows(AuthMethod::OauthCc) || self.allows(AuthMethod::Gettoken)
    }
}

/// Lookup function resolving a front-scoped OAuth/Gettoken string to [`AuthedApp`].
pub type TokenLookup<'a> = &'a dyn Fn(&str) -> Option<AuthedApp>;

/// Evaluates HTTP authentication against `policy` and an optional `token_lookup` closure.
///
/// # Errors
/// Returns [`AuthError`] when credentials are missing, malformed, or invalid.
pub fn authenticate(
    policy: &AuthPolicy,
    input: &AuthInput<'_>,
    token_lookup: Option<TokenLookup<'_>>,
) -> Result<AuthOk, AuthError> {
    if policy.allows(AuthMethod::None) {
        return Ok(unauthenticated_ok(&policy.apps));
    }
    if policy.methods.is_empty() {
        return Err(AuthError::Missing);
    }

    let (authorization, api_key) = merge_credentials(input);
    let presented = authorization.is_some() || api_key.is_some();

    if let Some(value) = authorization.as_deref() {
        return authenticate_authz(policy, input, value, presented, token_lookup);
    }

    if policy.allows(AuthMethod::ApiKey)
        && let Some(key) = api_key.as_deref()
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
        return Err(AuthError::DigestChallenge(digest_challenges(
            &policy.digest_nonces,
        )));
    }
    Err(AuthError::Missing)
}

fn authenticate_authz(
    policy: &AuthPolicy,
    input: &AuthInput<'_>,
    value: &str,
    presented: bool,
    token_lookup: Option<TokenLookup<'_>>,
) -> Result<AuthOk, AuthError> {
    if value.starts_with("Hawk ") && policy.allows(AuthMethod::Hawk) {
        let host_hdr = input
            .headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok());
        let (host, port) = host_port(host_hdr, policy.listen_port);
        if host.is_empty() {
            return Err(AuthError::InvalidToken);
        }
        let resource = resource_from_uri(input.path, input.query);
        let ct = input
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        return verify_hawk(
            &policy.apps,
            &policy.hawk_nonces,
            value,
            &HawkParts {
                method: input.method,
                host: &host,
                port,
                resource: &resource,
                content_type: ct,
                body: input.body,
            },
        )
        .map(|app| AuthOk {
            app,
            method: AuthMethod::Hawk,
        })
        .ok_or(AuthError::InvalidToken);
    }
    if (value.starts_with("Digest ") || value.starts_with("digest "))
        && policy.allows(AuthMethod::Digest)
    {
        let uri = resource_from_uri(input.path, input.query);
        return verify_digest(
            &policy.apps,
            &policy.digest_nonces,
            value,
            input.method,
            &uri,
        )
        .map(|app| AuthOk {
            app,
            method: AuthMethod::Digest,
        })
        .ok_or(AuthError::InvalidToken);
    }
    if value.starts_with("AWS4-HMAC-SHA256 ") && policy.allows(AuthMethod::AwsSigv4) {
        let host = input
            .headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        return verify_aws_sigv4(
            &policy.apps,
            value,
            &AwsRequest {
                method: input.method,
                path: input.path,
                query: input.query.unwrap_or(""),
                host,
                headers: input.headers,
                body: input.body,
            },
        )
        .map(|app| AuthOk {
            app,
            method: AuthMethod::AwsSigv4,
        })
        .ok_or(AuthError::InvalidToken);
    }
    if let Some(token) = value.strip_prefix("Bearer ") {
        return verify_bearer(policy, token.trim(), token_lookup);
    }
    if value.starts_with("Basic ") {
        if !policy.allows(AuthMethod::Basic) {
            return Err(AuthError::InvalidFormat);
        }
        return parse_basic(value)
            .and_then(|(id, secret)| {
                let app = policy.apps.get(&id)?;
                secrets_equal(&app.client_secret, &secret).then(|| AuthedApp::from(app))
            })
            .map(|app| AuthOk {
                app,
                method: AuthMethod::Basic,
            })
            .ok_or(AuthError::InvalidToken);
    }
    if presented {
        return Err(AuthError::InvalidFormat);
    }
    Err(AuthError::Missing)
}

fn verify_bearer(
    policy: &AuthPolicy,
    token: &str,
    token_lookup: Option<TokenLookup<'_>>,
) -> Result<AuthOk, AuthError> {
    if policy.uses_token_bucket()
        && let Some(lookup) = token_lookup
        && let Some(app) = lookup(token)
    {
        let method = if policy.allows(AuthMethod::OauthCc) {
            AuthMethod::OauthCc
        } else {
            AuthMethod::Gettoken
        };
        return Ok(AuthOk { app, method });
    }
    if policy.allows(AuthMethod::Jwt) && looks_like_jwt(token) {
        return verify_jwt(&policy.apps, token)
            .map(|app| AuthOk {
                app,
                method: AuthMethod::Jwt,
            })
            .ok_or(AuthError::InvalidToken);
    }
    if (policy.allows(AuthMethod::ApiKey) || policy.allows(AuthMethod::Bearer))
        && let Some(app) = policy.apps.find_by_secret(token)
    {
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
    if policy.allows(AuthMethod::Jwt) && !policy.allows(AuthMethod::ApiKey) {
        return verify_jwt(&policy.apps, token)
            .map(|app| AuthOk {
                app,
                method: AuthMethod::Jwt,
            })
            .ok_or(AuthError::InvalidToken);
    }
    Err(AuthError::InvalidToken)
}

fn unauthenticated_ok(apps: &AppRegistry) -> AuthOk {
    if let Some(app) = apps.iter().next() {
        return AuthOk {
            app: AuthedApp::from(app),
            method: AuthMethod::None,
        };
    }
    let methods: HashSet<String> = BusinessMethod::all()
        .into_iter()
        .map(|m| m.as_str().to_string())
        .collect();
    AuthOk {
        app: AuthedApp {
            client_id: "anonymous".to_string(),
            methods,
        },
        method: AuthMethod::None,
    }
}

fn merge_credentials(input: &AuthInput<'_>) -> (Option<String>, Option<String>) {
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(input.query.unwrap_or("").as_bytes())
            .map(|(k, v)| (k.to_ascii_lowercase(), v.into_owned()))
            .collect();
    let h_auth = input
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let h_key = input
        .headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let q_auth = query_first(&query, &["authorization", "access_token"]).map(|v| {
        if query.contains_key("access_token") {
            format!("Bearer {v}")
        } else {
            v
        }
    });
    let q_key = query_first(&query, &["api_key", "x-api-key"]);

    if input.query_overrides_header {
        (q_auth.or(h_auth), q_key.or(h_key))
    } else {
        (h_auth.or(q_auth), h_key.or(q_key))
    }
}

fn query_first(map: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| map.get(*k).filter(|s| !s.is_empty()).cloned())
}

/// Parses `Authorization: Basic base64(id:secret)`.
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

/// Extracts OAuth client credentials from HTTP Basic header (preferred) or form body.
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

/// Appends Digest `WWW-Authenticate` challenge headers to a response [`HeaderMap`].
pub fn attach_digest_headers(headers: &mut HeaderMap, challenges: &[String]) {
    for item in challenges {
        if let Ok(value) = HeaderValue::from_str(item) {
            headers.append(header::WWW_AUTHENTICATE, value);
        }
    }
}

/// Builds the ASCII-sorted `k=v&...` signing string for `sign_gateway` (excluding `sign` and empty values).
#[must_use]
pub fn sign_source(fields: &HashMap<String, String>) -> String {
    let mut pairs: Vec<(&str, &str)> = fields
        .iter()
        .filter(|(k, v)| k.as_str() != "sign" && !v.is_empty())
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Serializes a JSON value with lexicographically ordered object keys for `sign_gateway` response signing.
#[must_use]
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let ordered: BTreeMap<&str, &serde_json::Value> =
                map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            let inner = ordered
                .iter()
                .map(|(k, v)| {
                    let key_json = serde_json::to_string(k).unwrap_or_else(|_| format!("\"{k}\""));
                    format!("{key_json}:{}", canonical_json(v))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        serde_json::Value::Array(arr) => {
            let inner = arr.iter().map(canonical_json).collect::<Vec<_>>().join(",");
            format!("[{inner}]")
        }
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".to_string()),
    }
}

/// Verifies Base64-encoded HMAC-SHA256 signature for `sign_gateway`.
#[must_use]
pub fn verify_hmac(client_secret: &str, source: &str, sign_b64: &str) -> bool {
    let Some(expected_raw) = hmac_sha256(client_secret.as_bytes(), source.as_bytes()) else {
        return false;
    };
    let Ok(provided_raw) = base64::engine::general_purpose::STANDARD.decode(sign_b64.trim()) else {
        return false;
    };
    secrets_equal_bytes(&expected_raw, &provided_raw)
}

/// Computes Base64-encoded HMAC-SHA256 over canonical JSON for `sign_gateway` responses.
#[must_use]
pub fn sign_hmac(client_secret: &str, object: &serde_json::Value) -> String {
    let source = canonical_json(object);
    hmac_sha256(client_secret.as_bytes(), source.as_bytes())
        .map(|raw| base64::engine::general_purpose::STANDARD.encode(raw))
        .unwrap_or_default()
}
