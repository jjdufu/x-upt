use std::collections::BTreeMap;
use std::collections::HashMap;

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::service::apps::secrets_equal_bytes;

type HmacSha256 = Hmac<Sha256>;

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

#[must_use]
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let ordered: BTreeMap<&str, &serde_json::Value> =
                map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            let inner = ordered
                .iter()
                .map(|(k, v)| format!("\"{}\":{}", json_escape(k), canonical_json(v)))
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

fn json_escape(s: &str) -> String {
    serde_json::to_string(s)
        .unwrap_or_else(|_| format!("\"{s}\""))
        .trim_matches('"')
        .to_string()
}

#[must_use]
pub fn verify_hmac(client_secret: &str, source: &str, sign_b64: &str) -> bool {
    let Some(expected) = hmac_b64(client_secret, source.as_bytes()) else {
        return false;
    };
    let Some(left) = decode_b64(sign_b64) else {
        return false;
    };
    let Some(right) = decode_b64(&expected) else {
        return false;
    };
    secrets_equal_bytes(&left, &right)
}

#[must_use]
pub fn sign_hmac(client_secret: &str, object: &serde_json::Value) -> String {
    let source = canonical_json(object);
    hmac_b64(client_secret, source.as_bytes()).unwrap_or_default()
}

fn decode_b64(sign_b64: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(sign_b64.trim())
        .ok()
}

fn hmac_b64(secret: &str, data: &[u8]) -> Option<String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(data);
    Some(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}
