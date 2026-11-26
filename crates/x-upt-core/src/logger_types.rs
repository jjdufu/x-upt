//! Structured 31-field `access_log` types, request ID validation, and payload redaction.

use serde::{Deserialize, Serialize};

/// Maximum byte length of a valid caller-supplied `X-Request-Id`.
pub const REQUEST_ID_MAX_BYTES: usize = 128;

/// Event classification of a structured access log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEvent {
    /// Real inbound HTTP / gRPC / WS request.
    Access,
    /// Async job execution completed (`succeeded` or `failed`).
    JobDone,
    /// Async job expired before or after queuing.
    JobExpired,
}

/// 31-field structured access log entry written to disk and the in-memory ring buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Request trace identifier.
    pub request_id: String,
    /// RFC 3339 UTC timestamp.
    pub ts: String,
    /// Log severity (`info`, `warn`, `error`).
    pub level: String,
    /// Event kind.
    pub event: LogEvent,
    /// HTTP/protocol method (`GET`, `POST`, `WS`).
    pub method: String,
    /// Request path.
    pub path: String,
    /// Redacted query string.
    pub query: Option<String>,
    /// HTTP or protocol status code.
    pub status: u16,
    /// Protocol-specific business status/error code.
    pub code: Option<serde_json::Value>,
    /// Total request duration in milliseconds.
    pub duration: u64,
    /// Authentication duration in milliseconds.
    pub duration_auth: Option<u64>,
    /// Core dispatch duration in milliseconds.
    pub duration_core: Option<u64>,
    /// Artificial latency sleep in milliseconds.
    pub duration_latency: Option<u64>,
    /// Client socket/forwarded IP.
    pub ip: Option<String>,
    /// Authenticated caller `client_id`.
    pub client_id: Option<String>,
    /// Inbound payload size in bytes.
    pub bytes_in: Option<u64>,
    /// Outbound payload size in bytes.
    pub bytes_out: Option<u64>,
    /// Error summary text.
    pub error: Option<String>,
    /// Machine-readable error category.
    pub error_kind: Option<String>,
    /// Redacted inbound business parameters.
    #[serde(rename = "in")]
    pub input: serde_json::Value,
    /// Redacted outbound business result.
    pub out: serde_json::Value,
    /// Front snake_case name.
    pub front: String,
    /// Listening TCP port.
    pub port: u16,
    /// Invoked RPC / method wire name.
    pub rpc: Option<String>,
    /// Async job UUID if applicable.
    pub job_id: Option<String>,
    /// HTTP `User-Agent`.
    pub user_agent: Option<String>,
    /// HTTP `Referer`.
    pub referer: Option<String>,
    /// HTTP `Content-Type`.
    pub content_type: Option<String>,
    /// Verified or inferred authentication mode.
    pub auth: Option<String>,
    /// OS process ID.
    pub pid: u32,
    /// Hostname if available.
    pub host: Option<String>,
}

/// Validates `X-Request-Id` against `[A-Za-z0-9._-]{1,128}`.
#[must_use]
pub fn is_valid_request_id(value: &str) -> bool {
    let len = value.len();
    (1..=REQUEST_ID_MAX_BYTES).contains(&len)
        && value
            .bytes()
            .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

/// Generates a fresh UUIDv4 request ID.
#[must_use]
pub fn generate_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Resolves a caller `X-Request-Id` or generates a new UUIDv4.
#[must_use]
pub fn resolve_request_id_raw(value: Option<&str>) -> String {
    value
        .and_then(|raw| is_valid_request_id(raw).then(|| raw.to_string()))
        .unwrap_or_else(generate_request_id)
}

/// Derives log severity (`info`, `warn`, `error`) from status and error presence.
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

/// Masks sensitive credential and image Base64 parameters in URL query strings.
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

/// Strips image Base64 payloads (`image` string inputs, `base64` fields, GraphQL `image:` literals) from logged JSON.
pub fn omit_logged_images(value: &mut serde_json::Value) {
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

/// Replaces inline `image: "..."` string literals inside GraphQL query text with `image: ""`.
#[must_use]
pub fn redact_gql_image_literals(query: &str) -> String {
    if !query.contains("image") {
        return query.to_string();
    }
    let bytes = query.as_bytes();
    let mut out = String::with_capacity(query.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'"') {
            let tail = query.get(i..).unwrap_or("");
            let lit = string_literal_len(tail).unwrap_or(1);
            out.push_str(query.get(i..i + lit).unwrap_or(""));
            i += lit;
            continue;
        }
        if is_image_name(bytes, i) {
            let name_end = i + 5;
            let after = query.get(name_end..).unwrap_or("");
            let trimmed = after.trim_start();
            if let Some(after_colon) = trimmed.strip_prefix(':') {
                let ws_len = after.len() - trimmed.len();
                let val_str = after_colon.trim_start();
                if let Some(lit_len) = string_literal_len(val_str) {
                    let colon_ws = after_colon.len() - val_str.len();
                    out.push_str("image");
                    out.push_str(after.get(..ws_len).unwrap_or(""));
                    out.push(':');
                    out.push_str(after_colon.get(..colon_ws).unwrap_or(""));
                    out.push_str("\"\"");
                    i = name_end + ws_len + 1 + colon_ws + lit_len;
                    continue;
                }
            }
        }
        let tail = query.get(i..).unwrap_or("");
        let ch = tail.chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn is_image_name(bytes: &[u8], i: usize) -> bool {
    let Some(end) = i.checked_add(5) else {
        return false;
    };
    if bytes.get(i..end) != Some(b"image".as_slice()) {
        return false;
    }
    let prev_ok = i == 0 || bytes.get(i - 1).is_none_or(|b| !is_name_byte(*b));
    let next_ok = end == bytes.len() || bytes.get(end).is_none_or(|b| !is_name_byte(*b));
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
    chars.next()?;
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
