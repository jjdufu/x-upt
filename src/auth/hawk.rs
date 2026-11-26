use std::collections::HashMap;

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

use crate::auth::nonce::NonceStore;
use crate::service::apps::{AppRegistry, secrets_equal_bytes};
use crate::types::token::AuthedApp;

type HmacSha256 = Hmac<Sha256>;

pub const WINDOW_SECONDS: u64 = 300;

pub struct HawkParts<'a> {
    pub method: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub resource: &'a str,
    pub content_type: &'a str,
    pub body: &'a [u8],
}

#[must_use]
pub fn verify(
    apps: &AppRegistry,
    nonces: &NonceStore,
    authorization: &str,
    parts: &HawkParts<'_>,
) -> Option<AuthedApp> {
    let rest = authorization.strip_prefix("Hawk ")?;
    let params = parse_params(rest);
    let id = params.get("id")?;
    let ts = params.get("ts")?;
    let nonce = params.get("nonce")?;
    let mac_b64 = params.get("mac")?;
    let ext = params.get("ext").map_or("", String::as_str);
    let hash_b64 = params.get("hash").map_or("", String::as_str);
    let ts_unix = ts.parse::<i64>().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())?;
    let window = i64::try_from(WINDOW_SECONDS).unwrap_or(i64::MAX);
    if (now - ts_unix).abs() > window {
        return None;
    }
    let app = apps.get(id)?;

    let payload_hash = if hash_b64.is_empty() {
        String::new()
    } else {
        let ct = parts
            .content_type
            .split(';')
            .next()
            .unwrap_or(parts.content_type)
            .trim();
        let normalized = format!(
            "hawk.1.payload\n{ct}\n{}\n",
            std::str::from_utf8(parts.body).unwrap_or("")
        );
        let digest = Sha256::digest(normalized.as_bytes());
        let expected = base64::engine::general_purpose::STANDARD.encode(digest);
        if !secrets_equal_bytes(expected.as_bytes(), hash_b64.as_bytes()) {
            return None;
        }
        expected
    };

    let normalized = format!(
        "hawk.1.header\n{ts}\n{nonce}\n{}\n{}\n{}\n{}\n{payload_hash}\n{ext}\n",
        parts.method.to_ascii_uppercase(),
        parts.resource,
        parts.host,
        parts.port,
    );
    let expected = hmac_b64(app.client_secret.as_bytes(), normalized.as_bytes())?;
    let left = decode_b64(mac_b64)?;
    let right = decode_b64(&expected)?;
    if !secrets_equal_bytes(&left, &right) {
        return None;
    }
    let replay_key = format!("{id}\n{ts}\n{nonce}");
    if !nonces.insert(&replay_key) {
        return None;
    }
    Some(AuthedApp::from(app))
}

fn hmac_b64(key: &[u8], data: &[u8]) -> Option<String> {
    let mut mac = HmacSha256::new_from_slice(key).ok()?;
    mac.update(data);
    Some(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn decode_b64(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .ok()
}

fn parse_params(rest: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in rest.split(',') {
        let part = part.trim();
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim().trim_matches('"').to_string();
        out.insert(key, val);
    }
    out
}

#[must_use]
pub fn resource_from_uri(path: &str, query: Option<&str>) -> String {
    let mut r = if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    };
    if let Some(q) = query.filter(|s| !s.is_empty()) {
        r.push('?');
        r.push_str(q);
    }
    r
}

#[must_use]
pub fn host_port(host_header: Option<&str>, fallback_port: u16) -> (String, u16) {
    let Some(raw) = host_header.map(str::trim).filter(|s| !s.is_empty()) else {
        return (String::new(), fallback_port);
    };
    if let Some((h, p)) = raw.rsplit_once(':')
        && !h.is_empty()
        && !h.starts_with('[')
        && let Ok(port) = p.parse::<u16>()
    {
        return (h.to_string(), port);
    }
    if raw.starts_with('[')
        && let Some(end) = raw.find(']')
    {
        let host = raw[1..end].to_string();
        let port = raw
            .get(end + 1..)
            .and_then(|s| s.strip_prefix(':'))
            .and_then(|s| s.parse().ok())
            .unwrap_or(fallback_port);
        return (host, port);
    }
    (raw.to_string(), fallback_port)
}
