//! Hawk SHA-256 MAC and AWS Signature Version 4 (`AWS4-HMAC-SHA256`) verifiers.

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use http::HeaderMap;
use sha2::{Digest, Sha256};
use x_upt_core::domain::{AppRegistry, AuthedApp, secrets_equal_bytes};

use crate::jwt_digest::parse_kv_params;
use crate::nonce::NonceStore;

type HmacSha256 = Hmac<Sha256>;

/// Fixed Hawk timestamp skew window in seconds.
pub const HAWK_WINDOW_SECONDS: u64 = 300;

/// Request components used for Hawk MAC verification.
pub struct HawkParts<'a> {
    /// HTTP method.
    pub method: &'a str,
    /// Host header hostname.
    pub host: &'a str,
    /// Effective port.
    pub port: u16,
    /// Request path + query.
    pub resource: &'a str,
    /// Content-Type header value.
    pub content_type: &'a str,
    /// Raw request body.
    pub body: &'a [u8],
}

/// Verifies an `Authorization: Hawk ...` header.
#[must_use]
pub fn verify_hawk(
    apps: &AppRegistry,
    nonces: &NonceStore,
    authorization: &str,
    parts: &HawkParts<'_>,
) -> Option<AuthedApp> {
    let rest = authorization.strip_prefix("Hawk ")?;
    let params = parse_kv_params(rest);
    let id = params.get("id")?;
    let ts = params.get("ts")?;
    let nonce = params.get("nonce")?;
    let mac_b64 = params.get("mac")?;
    let ext = params.get("ext").map_or("", String::as_str);
    let hash_b64 = params.get("hash").map_or("", String::as_str);

    let ts_unix = ts.parse::<i64>().ok()?;
    let now = chrono::Utc::now().timestamp();
    let window = i64::try_from(HAWK_WINDOW_SECONDS).unwrap_or(i64::MAX);
    if (now - ts_unix).abs() > window {
        return None;
    }
    let app = apps.get(id)?;
    let payload_hash = verify_hawk_payload(hash_b64, parts)?;

    let normalized = format!(
        "hawk.1.header\n{ts}\n{nonce}\n{}\n{}\n{}\n{}\n{payload_hash}\n{ext}\n",
        parts.method.to_ascii_uppercase(),
        parts.resource,
        parts.host,
        parts.port,
    );
    let expected = base64::engine::general_purpose::STANDARD.encode(hmac_sha256(
        app.client_secret.as_bytes(),
        normalized.as_bytes(),
    )?);
    let left = base64::engine::general_purpose::STANDARD
        .decode(mac_b64.trim())
        .ok()?;
    let right = base64::engine::general_purpose::STANDARD
        .decode(expected.trim())
        .ok()?;
    if !secrets_equal_bytes(&left, &right) {
        return None;
    }
    if !nonces.insert(&format!("{id}\n{ts}\n{nonce}")) {
        return None;
    }
    Some(AuthedApp::from(app))
}

fn verify_hawk_payload(hash_b64: &str, parts: &HawkParts<'_>) -> Option<String> {
    if hash_b64.is_empty() {
        return Some(String::new());
    }
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
    let expected =
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(normalized.as_bytes()));
    if !secrets_equal_bytes(expected.as_bytes(), hash_b64.as_bytes()) {
        return None;
    }
    Some(expected)
}

/// Builds `path?query` resource string for Hawk and Digest verification.
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

/// Splits an HTTP `Host` header into `(host, port)` with IPv6 bracket support.
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
        let host = raw.get(1..end).unwrap_or("").to_string();
        let port = raw
            .get(end + 1..)
            .and_then(|s| s.strip_prefix(':'))
            .and_then(|s| s.parse().ok())
            .unwrap_or(fallback_port);
        return (host, port);
    }
    (raw.to_string(), fallback_port)
}

/// Request components used for AWS SigV4 verification.
pub struct AwsRequest<'a> {
    /// HTTP method.
    pub method: &'a str,
    /// Request path.
    pub path: &'a str,
    /// Raw query string.
    pub query: &'a str,
    /// Host header text.
    pub host: &'a str,
    /// Request headers.
    pub headers: &'a HeaderMap,
    /// Raw request body.
    pub body: &'a [u8],
}

/// Verifies an `Authorization: AWS4-HMAC-SHA256 ...` header.
#[must_use]
pub fn verify_aws_sigv4(
    apps: &AppRegistry,
    authorization: &str,
    req: &AwsRequest<'_>,
) -> Option<AuthedApp> {
    let (credential, signed_headers, signature) = parse_aws_header(authorization)?;
    let (access_key_id, date_stamp, region, service) = parse_aws_cred(credential)?;
    let app = apps.get(access_key_id)?;
    let amz_date = req
        .headers
        .get("x-amz-date")
        .and_then(|v| v.to_str().ok())?;
    let payload_hash = hex::encode(Sha256::digest(req.body));

    let mut canonical_headers = String::new();
    for name in signed_headers.split(';') {
        let value = signed_header_value(name, req, &payload_hash)?;
        canonical_headers.push_str(name);
        canonical_headers.push(':');
        canonical_headers.push_str(value.trim());
        canonical_headers.push('\n');
    }
    let canonical_uri = if req.path.is_empty() { "/" } else { req.path };
    let canonical_request = format!(
        "{}\n{canonical_uri}\n{}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        req.method.to_ascii_uppercase(),
        canonical_query(req.query)
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{date_stamp}/{region}/{service}/aws4_request\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let signing_key = aws_signing_key(&app.client_secret, date_stamp, region, service)?;
    let expected = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);
    if !secrets_equal_bytes(expected.as_bytes(), signature.as_bytes()) {
        return None;
    }
    Some(AuthedApp::from(app))
}

fn parse_aws_header(auth: &str) -> Option<(&str, &str, &str)> {
    let rest = auth.strip_prefix("AWS4-HMAC-SHA256 ")?;
    let mut cred = None;
    let mut signed = None;
    let mut sig = None;
    for part in rest.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("Credential=") {
            cred = Some(v.trim());
        } else if let Some(v) = part.strip_prefix("SignedHeaders=") {
            signed = Some(v.trim());
        } else if let Some(v) = part.strip_prefix("Signature=") {
            sig = Some(v.trim());
        }
    }
    Some((cred?, signed?, sig?))
}

fn parse_aws_cred(credential: &str) -> Option<(&str, &str, &str, &str)> {
    let mut parts = credential.split('/');
    let ak = parts.next()?;
    let date = parts.next()?;
    let region = parts.next()?;
    let service = parts.next()?;
    let term = parts.next()?;
    if term != "aws4_request" || parts.next().is_some() || region.is_empty() || service.is_empty() {
        return None;
    }
    Some((ak, date, region, service))
}

fn signed_header_value(name: &str, req: &AwsRequest<'_>, payload_hash: &str) -> Option<String> {
    match name {
        "host" => Some(req.host.to_ascii_lowercase()),
        "x-amz-content-sha256" => req
            .headers
            .get("x-amz-content-sha256")
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned)
            .or_else(|| Some(payload_hash.to_string())),
        _ => req
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string()),
    }
}

fn canonical_query(query: &str) -> String {
    if query.is_empty() {
        return String::new();
    }
    let mut pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (uri_encode(&k), uri_encode(&v)))
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn uri_encode(s: &str) -> String {
    let mut out = String::new();
    for &b in s.as_bytes() {
        if matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn aws_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Option<Vec<u8>> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes())?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, service.as_bytes())?;
    hmac_sha256(&k_service, b"aws4_request")
}

/// Computes raw HMAC-SHA256 bytes.
#[must_use]
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key).ok()?;
    mac.update(data);
    Some(mac.finalize().into_bytes().to_vec())
}
