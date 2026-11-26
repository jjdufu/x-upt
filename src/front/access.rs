use std::net::SocketAddr;
use std::time::Instant;

use axum::body::HttpBody;
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::FrontName;
use crate::logger;
use crate::logger::context::{ExtraCall, LogContext, Snapshot};
use crate::types::logger::{LogEntry, LogEvent};

static X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

#[derive(Clone, Copy)]
pub struct AccessMeta {
    pub front: FrontName,
    pub port: u16,
}

pub async fn access_log(mut req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let query = logger::redact_query(req.uri().query());
    let bytes_in = content_length(req.headers()).or_else(|| req.body().size_hint().exact());
    let ip = client_ip(&req);
    let request_id = logger::resolve_request_id(req.headers());
    let user_agent = logger::header_text(req.headers(), &header::USER_AGENT);
    let referer = logger::header_text(req.headers(), &header::REFERER);
    let content_type = logger::header_text(req.headers(), &header::CONTENT_TYPE);
    let auth_hint = logger::auth_hint(req.headers());
    let meta = req
        .extensions()
        .get::<AccessMeta>()
        .copied()
        .unwrap_or(AccessMeta {
            front: FrontName::OauthGateway,
            port: 0,
        });

    let ctx = LogContext::new(
        request_id.clone(),
        meta.front.as_str().to_string(),
        meta.port,
    );
    ctx.set_ip(ip.clone());
    ctx.set_query(query.clone());
    ctx.set_user_agent(user_agent.clone());
    ctx.set_referer(referer.clone());
    ctx.set_content_type(content_type.clone());
    ctx.set_auth_hint(auth_hint);
    req.extensions_mut().insert(ctx.clone());

    let mut response = next.run(req).await;

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(X_REQUEST_ID.clone(), value);
    }

    let snapshot = ctx.snapshot();
    let bytes_out =
        content_length(response.headers()).or_else(|| response.body().size_hint().exact());
    let duration = logger::elapsed_ms(start);
    let status = response.status().as_u16();
    logger::log(&entry_from_snapshot(
        &snapshot,
        &method,
        &path,
        status,
        duration,
        snapshot.ip.clone().or(ip.clone()),
        bytes_in,
        bytes_out,
        snapshot.rpc.clone(),
        snapshot.input.clone(),
        snapshot.output.clone(),
        snapshot.error.clone(),
        snapshot.error_kind.clone(),
        snapshot.code.clone(),
        snapshot.job_id.clone(),
        snapshot.duration_core,
    ));
    for extra in &snapshot.extras {
        logger::log(&entry_from_extra(
            &snapshot,
            &method,
            &path,
            status,
            duration,
            snapshot.ip.clone().or(ip.clone()),
            bytes_in,
            bytes_out,
            extra.clone(),
        ));
    }

    response
}

fn entry_from_snapshot(
    snapshot: &Snapshot,
    method: &str,
    path: &str,
    status: u16,
    duration: u64,
    ip: Option<String>,
    bytes_in: Option<u64>,
    bytes_out: Option<u64>,
    rpc: Option<String>,
    input: serde_json::Value,
    output: serde_json::Value,
    error: Option<String>,
    error_kind: Option<String>,
    code: Option<serde_json::Value>,
    job_id: Option<String>,
    duration_core: Option<u64>,
) -> LogEntry {
    LogEntry {
        request_id: snapshot.request_id.clone(),
        ts: String::new(),
        level: String::new(),
        event: LogEvent::Access,
        method: method.to_string(),
        path: path.to_string(),
        query: snapshot.query.clone(),
        status,
        code,
        duration,
        duration_auth: snapshot.duration_auth,
        duration_core,
        duration_latency: snapshot.duration_latency,
        ip,
        client_id: snapshot.client_id.clone(),
        bytes_in,
        bytes_out,
        error,
        error_kind,
        input,
        out: output,
        front: snapshot.front.clone(),
        port: snapshot.port,
        rpc,
        job_id,
        user_agent: snapshot.user_agent.clone(),
        referer: snapshot.referer.clone(),
        content_type: snapshot.content_type.clone(),
        auth: snapshot.auth.clone(),
        pid: 0,
        host: None,
    }
}

fn entry_from_extra(
    snapshot: &Snapshot,
    method: &str,
    path: &str,
    status: u16,
    duration: u64,
    ip: Option<String>,
    bytes_in: Option<u64>,
    bytes_out: Option<u64>,
    extra: ExtraCall,
) -> LogEntry {
    entry_from_snapshot(
        snapshot,
        method,
        path,
        status,
        duration,
        ip,
        bytes_in,
        bytes_out,
        extra.rpc,
        extra.input,
        extra.output,
        extra.error,
        extra.error_kind,
        extra.code,
        extra.job_id,
        extra.duration_core,
    )
}

fn content_length(headers: &axum::http::HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
}

fn client_ip(req: &Request) -> Option<String> {
    if let Some(forwarded) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        let first = forwarded.split(',').next().map_or("", str::trim);
        if !first.is_empty() {
            return Some(first.to_string());
        }
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.to_string())
}

#[must_use]
pub fn client_ip_from(headers: &axum::http::HeaderMap, addr: Option<SocketAddr>) -> Option<String> {
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = forwarded.split(',').next().map_or("", str::trim);
        if !first.is_empty() {
            return Some(first.to_string());
        }
    }
    addr.map(|a| a.to_string())
}
