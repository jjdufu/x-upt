use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;

use crate::service::apps::AppRegistry;
use crate::types::token::AuthedApp;

#[derive(Debug, Deserialize)]
struct Claims {
    iss: Option<String>,
    sub: Option<String>,
}

fn alg_from_header(token: &str) -> Option<Algorithm> {
    let header = decode_header(token).ok()?;
    match header.alg {
        Algorithm::HS256
        | Algorithm::HS384
        | Algorithm::HS512
        | Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512 => Some(header.alg),
        _ => None,
    }
}

fn iss_sub_ok(claims: &Claims, client_id: &str) -> bool {
    claims.iss.as_deref() == Some(client_id) || claims.sub.as_deref() == Some(client_id)
}

fn validation(alg: Algorithm) -> Validation {
    let mut v = Validation::new(alg);
    v.validate_aud = false;
    v.required_spec_claims.clear();
    v
}

#[must_use]
pub fn looks_like_jwt(token: &str) -> bool {
    let mut dots = 0usize;
    for b in token.bytes() {
        if b == b'.' {
            dots += 1;
        }
    }
    dots == 2 && token.len() > 20
}

#[must_use]
pub fn verify(apps: &AppRegistry, token: &str) -> Option<AuthedApp> {
    let token = token.trim();
    let alg = alg_from_header(token)?;
    let is_hmac = matches!(alg, Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512);
    for app in apps.iter() {
        let key = if is_hmac {
            let Ok(key) = DecodingKey::from_secret(app.client_secret.as_bytes()) else {
                continue;
            };
            key
        } else {
            let Some(pem) = app.rsa_public_pem.as_deref() else {
                continue;
            };
            let Ok(key) = DecodingKey::from_rsa_pem(pem) else {
                continue;
            };
            key
        };
        let Ok(data) = decode::<Claims>(token, &key, &validation(alg)) else {
            continue;
        };
        if iss_sub_ok(&data.claims, &app.client_id) {
            return Some(AuthedApp::from(app));
        }
    }
    None
}
