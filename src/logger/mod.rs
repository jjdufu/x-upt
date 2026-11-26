pub mod context;

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;

use axum::http::{HeaderMap, HeaderName, header};
use uuid::Uuid;

use crate::types::logger::LogEntry;

pub const REQUEST_ID_MAX_BYTES: usize = 128;

static X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");

enum Sink {
    Single(Mutex<File>),
    Split(Mutex<HashMap<String, File>>),
}

static LOG_SINK: OnceLock<Sink> = OnceLock::new();

pub fn init(log_path: &str, split_by_front: bool, fronts: &[&str]) -> Result<(), String> {
    let path = Path::new(log_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create log dir: {e}"))?;
    }
    let sink = if split_by_front {
        let mut map = HashMap::new();
        for front in fronts {
            let front_path = split_path(path, front);
            if let Some(parent) = front_path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("Failed to create log dir: {e}"))?;
            }
            let file = open_append(&front_path)?;
            map.insert((*front).to_string(), file);
        }
        Sink::Split(Mutex::new(map))
    } else {
        Sink::Single(Mutex::new(open_append(path)?))
    };
    LOG_SINK
        .set(sink)
        .map_err(|_| "Logger already initialized".to_string())
}

fn open_append(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("Failed to open log file {}: {e}", path.display()))
}

#[must_use]
pub fn split_path(base: &Path, front: &str) -> PathBuf {
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "upupt".to_string());
    let ext = base.extension().and_then(|e| e.to_str()).unwrap_or("log");
    let name = format!("{stem}.{front}.{ext}");
    match base.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

#[must_use]
pub fn generate_request_id() -> String {
    Uuid::new_v4().to_string()
}

#[must_use]
pub fn is_valid_request_id(value: &str) -> bool {
    let len = value.len();
    (1..=REQUEST_ID_MAX_BYTES).contains(&len)
        && value
            .bytes()
            .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

#[must_use]
pub fn resolve_request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| is_valid_request_id(raw).then(|| raw.to_string()))
        .unwrap_or_else(generate_request_id)
}

#[must_use]
pub fn resolve_request_id_raw(value: Option<&str>) -> String {
    value
        .and_then(|raw| is_valid_request_id(raw).then(|| raw.to_string()))
        .unwrap_or_else(generate_request_id)
}

#[must_use]
pub fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[must_use]
pub fn redact_query(raw: Option<&str>) -> Option<String> {
    let raw = raw.filter(|s| !s.is_empty())?;
    let mut ser = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in url::form_urlencoded::parse(raw.as_bytes()) {
        let key_l = k.to_ascii_lowercase();
        let value = if matches!(
            key_l.as_str(),
            "api_key"
                | "x-api-key"
                | "access_token"
                | "authorization"
                | "secret"
                | "client_secret"
                | "image"
                | "clientid"
                | "clientsecret"
                | "password"
                | "passwd"
                | "sign"
                | "params"
                | "encryptstr"
                | "appcode"
                | "corpsecret"
                | "accesstoken"
                | "token"
                | "company_id"
        ) {
            "***"
        } else {
            v.as_ref()
        };
        ser.append_pair(&k, value);
    }
    let out = ser.finish();
    if out.is_empty() { None } else { Some(out) }
}

#[must_use]
pub fn header_text(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

#[must_use]
pub fn auth_hint(headers: &HeaderMap) -> Option<String> {
    if header_text(headers, &X_API_KEY).is_some() {
        return Some("api_key".to_string());
    }
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    if auth.starts_with("Hawk ") {
        Some("hawk".to_string())
    } else if auth.starts_with("Digest ") || auth.starts_with("digest ") {
        Some("digest".to_string())
    } else if auth.starts_with("AWS4-HMAC-SHA256 ") {
        Some("aws_sigv4".to_string())
    } else if auth.starts_with("Bearer ") {
        Some("bearer".to_string())
    } else if auth.starts_with("Basic ") {
        Some("basic".to_string())
    } else {
        None
    }
}

#[must_use]
pub fn host_name() -> Option<String> {
    static HOST: OnceLock<Option<String>> = OnceLock::new();
    HOST.get_or_init(|| {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
    })
    .clone()
}

#[must_use]
pub fn level_of(status: u16, error: Option<&str>) -> &'static str {
    if status >= 500 {
        "error"
    } else if status >= 400 || error.is_some() {
        "warn"
    } else {
        "info"
    }
}

fn fill(entry: &LogEntry) -> LogEntry {
    let mut out = entry.clone();
    if out.level.is_empty() {
        out.level = level_of(out.status, out.error.as_deref()).to_string();
    }
    if out.pid == 0 {
        out.pid = std::process::id();
    }
    if out.host.is_none() {
        out.host = host_name();
    }
    if out.ts.is_empty() {
        out.ts = chrono::Utc::now().to_rfc3339();
    }
    omit_logged_images(&mut out.input);
    omit_logged_images(&mut out.out);
    out
}

fn omit_logged_images(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if matches!(map.get("base64"), Some(serde_json::Value::String(_))) {
                map.remove("base64");
            }
            if matches!(map.get("image"), Some(serde_json::Value::String(_))) {
                map.remove("image");
            }
            if let Some(serde_json::Value::String(query)) = map.get("query").cloned() {
                let redacted = redact_gql_image_literals(&query);
                if redacted != query {
                    map.insert("query".to_string(), serde_json::Value::String(redacted));
                }
            }
            for child in map.values_mut() {
                omit_logged_images(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                omit_logged_images(item);
            }
        }
        _ => {}
    }
}

fn redact_gql_image_literals(query: &str) -> String {
    if !query.contains("image") {
        return query.to_string();
    }
    let bytes = query.as_bytes();
    let mut out = String::with_capacity(query.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let lit = string_literal_len(&query[i..]).unwrap_or(1);
            out.push_str(&query[i..i + lit]);
            i += lit;
            continue;
        }
        if is_image_name(bytes, i) {
            let name_end = i + 5;
            let after = &query[name_end..];
            let trimmed = after.trim_start();
            if let Some(after_colon) = trimmed.strip_prefix(':') {
                let ws_len = after.len() - trimmed.len();
                let value = after_colon.trim_start();
                if let Some(lit_len) = string_literal_len(value) {
                    let colon_ws = after_colon.len() - value.len();
                    out.push_str("image");
                    out.push_str(&after[..ws_len]);
                    out.push(':');
                    out.push_str(&after_colon[..colon_ws]);
                    out.push_str("\"\"");
                    i = name_end + ws_len + 1 + colon_ws + lit_len;
                    continue;
                }
            }
        }
        let ch = query[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn is_image_name(bytes: &[u8], i: usize) -> bool {
    let Some(end) = i.checked_add(5) else {
        return false;
    };
    if end > bytes.len() || &bytes[i..end] != b"image" {
        return false;
    }
    let prev_ok = i == 0 || !is_name_byte(bytes[i - 1]);
    let next_ok = end == bytes.len() || !is_name_byte(bytes[end]);
    prev_ok && next_ok
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn string_literal_len(s: &str) -> Option<usize> {
    if let Some(rest) = s.strip_prefix("\"\"\"") {
        let end = rest.find("\"\"\"")?;
        return Some(3 + end + 3);
    }
    if !s.starts_with('"') {
        return None;
    }
    let mut chars = s.chars();
    chars.next();
    let mut len = 1usize;
    while let Some(ch) = chars.next() {
        len += ch.len_utf8();
        if ch == '\\' {
            if let Some(escaped) = chars.next() {
                len += escaped.len_utf8();
            }
            continue;
        }
        if ch == '"' {
            return Some(len);
        }
    }
    None
}

pub fn log(entry: &LogEntry) {
    let filled = fill(entry);
    let Ok(json) = serde_json::to_string(&filled) else {
        return;
    };

    if let Some(sink) = LOG_SINK.get() {
        match sink {
            Sink::Single(writer_mutex) => write_line(writer_mutex, &json),
            Sink::Split(map_mutex) => {
                let mut map = match map_mutex.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some(writer) = map.get_mut(&entry.front)
                    && (writeln!(writer, "{json}").is_err() || writer.flush().is_err())
                {
                    eprintln!("Failed to write access log");
                }
            }
        }
    } else {
        println!("{json}");
    }
}

fn write_line(writer_mutex: &Mutex<File>, json: &str) {
    let mut writer = match writer_mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if writeln!(writer, "{json}").is_err() || writer.flush().is_err() {
        eprintln!("Failed to write access log");
    }
}

#[cfg(test)]
mod tests {
    use super::{omit_logged_images, redact_gql_image_literals, redact_query};

    #[test]
    fn query_hides_envelope_secrets_and_keeps_other_keys() {
        let out = redact_query(Some("company_id=acme&clientSecret=hidden&city=hz")).unwrap();
        assert!(out.contains("clientSecret=***") || out.contains("clientsecret=***"));
        assert!(out.contains("company_id=***"));
        assert!(out.contains("city=hz"));
        assert!(!out.contains("hidden"));
    }

    #[test]
    fn query_hides_image_param() {
        let out = redact_query(Some("filename=img_001.png&image=abc123&threshold=1")).unwrap();
        assert_eq!(out, "filename=img_001.png&image=***&threshold=1");
        assert!(!out.contains("abc123"));
    }

    #[test]
    fn log_json_drops_image_payload_only() {
        let mut value = serde_json::json!({
            "filename": "img_001.png",
            "image": "iVBORw0KGgoAAA",
            "threshold": 4,
            "query": "mutation { imgdefCompare(image: \"iVBORw0KGgoAAA\", filename: \"img_001.png\") { matched } }",
            "variables": {"image": "iVBORw0KGgoAAA", "filename": "img_001.png"},
            "result": {
                "image": {
                    "filename": "img_012.png",
                    "mime": "image/png",
                    "size": 12,
                    "base64": "iVBORw0KGgoAAA"
                }
            }
        });
        omit_logged_images(&mut value);
        let text = value.to_string();
        assert!(!text.contains("iVBORw0KGgoAAA"));
        assert_eq!(value["filename"], "img_001.png");
        assert_eq!(value["threshold"], 4);
        assert!(value.get("image").is_none());
        assert!(value["variables"].get("image").is_none());
        assert_eq!(value["variables"]["filename"], "img_001.png");
        assert_eq!(value["result"]["image"]["filename"], "img_012.png");
        assert_eq!(value["result"]["image"]["mime"], "image/png");
        assert_eq!(value["result"]["image"]["size"], 12);
        assert!(value["result"]["image"].get("base64").is_none());
        let query = value["query"].as_str().unwrap();
        assert!(query.contains("filename: \"img_001.png\""));
        assert!(query.contains("image: \"\""));
        assert!(query.contains("{ matched }"));
    }

    #[test]
    fn graphql_selection_is_kept() {
        let query = "{ photosRender { image { filename base64 } } }";
        assert_eq!(redact_gql_image_literals(query), query);
    }
}
