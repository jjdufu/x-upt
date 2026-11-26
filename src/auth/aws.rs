use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

use crate::service::apps::{AppRegistry, secrets_equal_bytes};
use crate::types::token::AuthedApp;

type HmacSha256 = Hmac<Sha256>;

pub struct AwsRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub host: &'a str,
    pub headers: &'a axum::http::HeaderMap,
    pub body: &'a [u8],
}

#[must_use]
pub fn verify(apps: &AppRegistry, authorization: &str, req: &AwsRequest<'_>) -> Option<AuthedApp> {
    let rest = authorization.strip_prefix("AWS4-HMAC-SHA256 ")?;
    let mut credential = None;
    let mut signed_headers = None;
    let mut signature = None;
    for part in rest.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("Credential=") {
            credential = Some(v.trim());
        } else if let Some(v) = part.strip_prefix("SignedHeaders=") {
            signed_headers = Some(v.trim());
        } else if let Some(v) = part.strip_prefix("Signature=") {
            signature = Some(v.trim());
        }
    }
    let credential = credential?;
    let signed_headers = signed_headers?;
    let signature = signature?;
    let mut cred_parts = credential.split('/');
    let access_key_id = cred_parts.next()?;
    let date_stamp = cred_parts.next()?;
    let region = cred_parts.next()?;
    let service = cred_parts.next()?;
    let term = cred_parts.next()?;
    if term != "aws4_request" || cred_parts.next().is_some() {
        return None;
    }
    if region.is_empty() || service.is_empty() {
        return None;
    }
    let app = apps.get(access_key_id)?;
    let amz_date = header_str(req.headers, "x-amz-date")?;
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
    let canonical_query = canonical_query(req.query);
    let canonical_request = format!(
        "{}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        req.method.to_ascii_uppercase()
    );
    let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
    let credential_scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign =
        format!("AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{canonical_hash}");
    let signing_key = aws_signing_key(&app.client_secret, date_stamp, region, service)?;
    let expected = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);
    if !secrets_equal_bytes(expected.as_bytes(), signature.as_bytes()) {
        return None;
    }
    Some(AuthedApp::from(app))
}

fn signed_header_value<'a>(
    name: &str,
    req: &'a AwsRequest<'a>,
    payload_hash: &'a str,
) -> Option<String> {
    match name {
        "host" => Some(req.host.to_ascii_lowercase()),
        "x-amz-content-sha256" => header_str(req.headers, "x-amz-content-sha256")
            .map(ToOwned::to_owned)
            .or_else(|| Some(payload_hash.to_string())),
        _ => header_str(req.headers, name)
            .map(str::trim)
            .map(ToOwned::to_owned),
    }
}

fn header_str<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
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
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*b));
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn aws_signing_key(secret: &str, date_stamp: &str, region: &str, service: &str) -> Option<Vec<u8>> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes())?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, service.as_bytes())?;
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key).ok()?;
    mac.update(data);
    Some(mac.finalize().into_bytes().to_vec())
}
