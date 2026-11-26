use std::collections::HashMap;

use md5::Md5;
use sha2::{Digest, Sha256};

use crate::auth::nonce::NonceStore;
use crate::service::apps::{AppRegistry, secrets_equal};
use crate::types::token::AuthedApp;

const REALM: &str = "upupt";

#[must_use]
pub fn challenge_headers(nonces: &NonceStore) -> Vec<String> {
    let nonce = issue_nonce(nonces);
    vec![
        format!(r#"Digest realm="{REALM}", qop="auth", algorithm=MD5, nonce="{nonce}""#),
        format!(r#"Digest realm="{REALM}", qop="auth", algorithm=SHA-256, nonce="{nonce}""#),
    ]
}

fn issue_nonce(nonces: &NonceStore) -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    let _ = nonces.insert(&raw);
    raw
}

#[must_use]
pub fn verify(
    apps: &AppRegistry,
    nonces: &NonceStore,
    authorization: &str,
    method: &str,
    uri: &str,
) -> Option<AuthedApp> {
    let rest = authorization
        .strip_prefix("Digest ")
        .or_else(|| authorization.strip_prefix("digest "))?;
    let params = parse_params(rest);
    let username = params.get("username")?;
    let nonce = params.get("nonce")?;
    if !nonces.contains(nonce) {
        return None;
    }
    let realm = params.get("realm").map_or(REALM, String::as_str);
    let algo = normalize_algo(params.get("algorithm").map_or("MD5", String::as_str));
    let qop = params.get("qop").map_or("auth", String::as_str);
    if !qop.eq_ignore_ascii_case("auth") {
        return None;
    }
    let nc = params.get("nc")?;
    let nc_n = parse_nc(nc)?;
    let cnonce = params.get("cnonce")?;
    let response = params.get("response")?;
    let digest_uri = params.get("uri").map_or(uri, String::as_str);
    let app = apps.get(username)?;
    let ha1 = hash(algo, &format!("{username}:{realm}:{}", app.client_secret));
    let ha2 = hash(
        algo,
        &format!("{}:{digest_uri}", method.to_ascii_uppercase()),
    );
    let expected = hash(algo, &format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"));
    if !secrets_equal(&expected, response) {
        return None;
    }
    if !nonces.accept_nc(nonce, nc_n) {
        return None;
    }
    Some(AuthedApp::from(app))
}

fn parse_nc(raw: &str) -> Option<u64> {
    if raw.len() != 8 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(raw, 16).ok().filter(|n| *n > 0)
}

fn normalize_algo(a: &str) -> &'static str {
    match a.trim().to_ascii_uppercase().as_str() {
        "SHA-256" | "SHA256" => "SHA-256",
        _ => "MD5",
    }
}

fn hash(algo: &str, data: &str) -> String {
    if algo == "SHA-256" {
        hex::encode(Sha256::digest(data.as_bytes()))
    } else {
        hex::encode(Md5::digest(data.as_bytes()))
    }
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
