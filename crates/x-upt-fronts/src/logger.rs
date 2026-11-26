//! Structured 31-column `access_log` writer and redacted in-memory ring buffer.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use chrono::Utc;
use http::{HeaderMap, header};
use x_upt_core::domain::FrontName;
use x_upt_core::error::AppError;
use x_upt_core::logger_types::{
    LogEntry, LogEvent, level_of, omit_logged_images, redact_query, resolve_request_id_raw,
};

const RING_CAPACITY: usize = 500;

/// Filter criteria for `GET /api/v1/logs/recent`.
#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    /// Exact front snake_case filter.
    pub front: Option<String>,
    /// Status code or prefix filter (e.g. `"200"`, `"4"`).
    pub status: Option<String>,
    /// Substring match on `request_id`.
    pub request_id: Option<String>,
    /// Maximum entries returned (`1`..=`500`).
    pub limit: usize,
}

/// Thread-safe structured access logger backed by `<data-dir>/logs/x-upt.log` and a 500-item ring buffer.
#[derive(Debug, Clone)]
pub struct AccessLogger {
    file: Arc<Mutex<Option<File>>>,
    ring: Arc<Mutex<VecDeque<LogEntry>>>,
    pid: u32,
}

impl AccessLogger {
    /// Opens `<data_dir>/logs/x-upt.log` in append mode and initializes the ring buffer.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] if log directory/file creation fails.
    pub fn open(data_dir: &Path) -> Result<Self, AppError> {
        let logs_dir = data_dir.join("logs");
        std::fs::create_dir_all(&logs_dir)
            .map_err(|e| AppError::Internal(format!("create logs dir failed: {e}")))?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs_dir.join("x-upt.log"))
            .map_err(|e| AppError::Internal(format!("open access log failed: {e}")))?;
        Ok(Self {
            file: Arc::new(Mutex::new(Some(file))),
            ring: Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY))),
            pid: std::process::id(),
        })
    }

    /// Creates an in-memory-only logger for unit tests.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            file: Arc::new(Mutex::new(None)),
            ring: Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY))),
            pid: std::process::id(),
        }
    }

    /// Redacts image Base64 literals and appends an entry to disk and the ring buffer.
    pub fn record(&self, mut entry: LogEntry) {
        omit_logged_images(&mut entry.input);
        omit_logged_images(&mut entry.out);
        entry.pid = self.pid;

        if let Ok(line) = serde_json::to_string(&entry) {
            let mut f_guard = self.file.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(f) = f_guard.as_mut() {
                drop(writeln!(f, "{line}"));
            }
        }

        let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        if ring.len() >= RING_CAPACITY {
            ring.pop_back();
        }
        ring.push_front(entry);
    }

    /// Queries recent log entries newest-first.
    #[must_use]
    pub fn query_recent(&self, q: &LogQuery) -> Vec<LogEntry> {
        let limit = q.limit.clamp(1, RING_CAPACITY);
        let front_f = q.front.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let status_f = q.status.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let req_f = q
            .request_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        ring.iter()
            .filter(|e| front_f.is_none_or(|f| e.front == f))
            .filter(|e| {
                status_f.is_none_or(|sf| {
                    let code_s = e.status.to_string();
                    code_s == sf || code_s.starts_with(sf)
                })
            })
            .filter(|e| req_f.is_none_or(|rf| e.request_id.contains(rf)))
            .take(limit)
            .cloned()
            .collect()
    }
}

/// Request metadata captured on arrival before protocol dispatch.
#[derive(Debug, Clone)]
pub struct ReqMeta {
    /// Validated or generated request ID.
    pub request_id: String,
    /// HTTP/protocol method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// Redacted query string.
    pub query: Option<String>,
    /// Peer IP string.
    pub ip: Option<String>,
    /// `User-Agent` header.
    pub user_agent: Option<String>,
    /// `Referer` header.
    pub referer: Option<String>,
    /// `Content-Type` header.
    pub content_type: Option<String>,
    /// `Host` header.
    pub host: Option<String>,
    /// Inbound body byte count.
    pub bytes_in: Option<u64>,
    /// Request start instant.
    pub started: Instant,
}

impl ReqMeta {
    /// Extracts request metadata from HTTP method, URI, headers, peer address, and body length.
    #[must_use]
    pub fn from_http(
        method: &str,
        path: &str,
        raw_query: Option<&str>,
        headers: &HeaderMap,
        peer: Option<SocketAddr>,
        bytes_in: Option<usize>,
    ) -> Self {
        let req_hdr = headers.get("x-request-id").and_then(|v| v.to_str().ok());
        Self {
            request_id: resolve_request_id_raw(req_hdr),
            method: method.to_string(),
            path: path.to_string(),
            query: redact_query(raw_query),
            ip: peer.map(|p| p.ip().to_string()),
            user_agent: hdr_str(headers, header::USER_AGENT.as_str()),
            referer: hdr_str(headers, header::REFERER.as_str()),
            content_type: hdr_str(headers, header::CONTENT_TYPE.as_str()),
            host: hdr_str(headers, header::HOST.as_str()),
            bytes_in: bytes_in.and_then(|n| u64::try_from(n).ok()),
            started: Instant::now(),
        }
    }
}

fn hdr_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string)
}

/// Outcome fields merged with [`ReqMeta`] to emit a 31-column [`LogEntry`].
#[derive(Debug, Default)]
pub struct LogOutcome {
    /// Event kind (`Access`, `JobDone`, `JobExpired`).
    pub event: Option<LogEvent>,
    /// HTTP/protocol status code.
    pub status: u16,
    /// Protocol business code.
    pub code: Option<serde_json::Value>,
    /// Auth duration in ms.
    pub duration_auth: Option<u64>,
    /// Core execution duration in ms.
    pub duration_core: Option<u64>,
    /// Simulated latency sleep in ms.
    pub duration_latency: Option<u64>,
    /// Verified client ID.
    pub client_id: Option<String>,
    /// Outbound payload byte count.
    pub bytes_out: Option<usize>,
    /// Error description.
    pub error: Option<String>,
    /// Error category slug.
    pub error_kind: Option<String>,
    /// Inbound business object.
    pub input: serde_json::Value,
    /// Outbound business object.
    pub out: serde_json::Value,
    /// Invoked RPC/method wire name.
    pub rpc: Option<String>,
    /// Async job ID if applicable.
    pub job_id: Option<String>,
    /// Authentication mode name.
    pub auth: Option<String>,
}

/// Builds a [`LogEntry`] from [`ReqMeta`], front info, and [`LogOutcome`].
#[must_use]
pub fn build_log_entry(
    front: FrontName,
    port: u16,
    meta: ReqMeta,
    outcome: LogOutcome,
) -> LogEntry {
    let elapsed_ms = u64::try_from(meta.started.elapsed().as_millis()).unwrap_or(0);
    let level = level_of(outcome.status, outcome.error.as_deref()).to_string();
    LogEntry {
        request_id: meta.request_id,
        ts: Utc::now().to_rfc3339(),
        level,
        event: outcome.event.unwrap_or(LogEvent::Access),
        method: meta.method,
        path: meta.path,
        query: meta.query,
        status: outcome.status,
        code: outcome.code,
        duration: elapsed_ms,
        duration_auth: outcome.duration_auth,
        duration_core: outcome.duration_core,
        duration_latency: outcome.duration_latency,
        ip: meta.ip,
        client_id: outcome.client_id,
        bytes_in: meta.bytes_in,
        bytes_out: outcome.bytes_out.and_then(|n| u64::try_from(n).ok()),
        error: outcome.error,
        error_kind: outcome.error_kind,
        input: outcome.input,
        out: outcome.out,
        front: front.as_str().to_string(),
        port,
        rpc: outcome.rpc,
        job_id: outcome.job_id,
        user_agent: meta.user_agent,
        referer: meta.referer,
        content_type: meta.content_type,
        auth: outcome.auth,
        pid: 0,
        host: meta.host,
    }
}
