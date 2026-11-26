//! JWT (HS/RS) and HTTP Digest (MD5/SHA-256) authentication verifiers.

use std::collections::HashMap;

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use md5::Md5;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use x_upt_core::domain::{AppRegistry, AuthedApp, secrets_equal};

use crate::nonce::NonceStore;

/// Realm string used in `WWW-Authenticate` challenges.
pub const AUTH_REALM: &str = "x-upt";

#[derive(Debug, Deserialize)]
struct JwtClaims {
    iss: Option<String>,
    sub: Option<String>,
}

/// Heuristic check whether a bearer token has 3 dot-separated JWT segments.
#[must_use]
pub fn looks_like_jwt(token: &str) -> bool {
    let dots = token.bytes().filter(|&b| b == b'.').count();
    dots == 2 && token.len() > 20
}

/// Verifies an HS256/384/512 or RS256/384/512 JWT against registered [`AppRegistry`] callers.
#[must_use]
pub fn verify_jwt(apps: &AppRegistry, token: &str) -> Option<AuthedApp> {
    let token = token.trim();
    let header = decode_header(token).ok()?;
    let alg = match header.alg {
        Algorithm::HS256
        | Algorithm::HS384
        | Algorithm::HS512
        | Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512 => header.alg,
        _ => return None,
    };
    let is_hmac = matches!(alg, Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512);
    let mut val = Validation::new(alg);
    val.validate_aud = false;
    val.required_spec_claims.clear();

    for app in apps.iter() {
        let key = if is_hmac {
            let Ok(k) = DecodingKey::from_secret(app.client_secret.as_bytes()) else {
                continue;
            };
            k
        } else {
            let Some(pem) = app.rsa_public_pem.as_deref() else {
                continue;
            };
            let Ok(k) = DecodingKey::from_rsa_pem(pem.as_bytes()) else {
                continue;
            };
            k
        };
        let Ok(data) = decode::<JwtClaims>(token, &key, &val) else {
            continue;
        };
        if data.claims.iss.as_deref() == Some(app.client_id.as_str())
            || data.claims.sub.as_deref() == Some(app.client_id.as_str())
        {
            return Some(AuthedApp::from(app));
        }
    }
    None
}

/// Issues dual MD5 and SHA-256 RFC 7616 `WWW-Authenticate` Digest challenge header values.
#[must_use]
pub fn digest_challenges(nonces: &NonceStore) -> Vec<String> {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let _inserted: bool = nonces.insert(&nonce);
    vec![
        format!(r#"Digest realm="{AUTH_REALM}", qop="auth", algorithm=MD5, nonce="{nonce}""#),
        format!(r#"Digest realm="{AUTH_REALM}", qop="auth", algorithm=SHA-256, nonce="{nonce}""#),
    ]
}

/// Verifies an RFC 7616 `Authorization: Digest ...` header.
#[must_use]
pub fn verify_digest(
    apps: &AppRegistry,
    nonces: &NonceStore,
    authorization: &str,
    method: &str,
    uri: &str,
) -> Option<AuthedApp> {
    let rest = authorization
        .strip_prefix("Digest ")
        .or_else(|| authorization.strip_prefix("digest "))?;
    let params = parse_kv_params(rest);
    let username = params.get("username")?;
    let nonce = params.get("nonce")?;
    if !nonces.contains(nonce) {
        return None;
    }
    let realm = params.get("realm").map_or(AUTH_REALM, String::as_str);
    let algo = match params
        .get("algorithm")
        .map_or("MD5", String::as_str)
        .trim()
        .to_ascii_uppercase()
        .as_str()
    {
        "SHA-256" | "SHA256" => "SHA-256",
        _ => "MD5",
    };
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
    let ha1 = digest_hash(algo, &format!("{username}:{realm}:{}", app.client_secret));
    let ha2 = digest_hash(
        algo,
        &format!("{}:{digest_uri}", method.to_ascii_uppercase()),
    );
    let expected = digest_hash(algo, &format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"));
    if !secrets_equal(&expected, response) || !nonces.accept_nc(nonce, nc_n) {
        return None;
    }
    Some(AuthedApp::from(app))
}

fn parse_nc(raw: &str) -> Option<u64> {
    if raw.len() != 8 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(raw, 16).ok().filter(|&n| n > 0)
}

fn digest_hash(algo: &str, data: &str) -> String {
    if algo == "SHA-256" {
        hex::encode(Sha256::digest(data.as_bytes()))
    } else {
        hex::encode(Md5::digest(data.as_bytes()))
    }
}

/// Parses comma-separated `key="value"` authorization parameters.
#[must_use]
pub fn parse_kv_params(rest: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in rest.split(',') {
        let Some((k, v)) = part.trim().split_once('=') else {
            continue;
        };
        out.insert(
            k.trim().to_ascii_lowercase(),
            v.trim().trim_matches('"').to_string(),
        );
    }
    out
}
