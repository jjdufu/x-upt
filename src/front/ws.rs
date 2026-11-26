use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Form, Router, middleware};
use tokio::time::timeout;

use crate::auth::oauth::{issue_token, token_success};
use crate::auth::token::TokenManager;
use crate::auth::{AuthInput, AuthPolicy, authenticate, policy_from};
use crate::config::{AppConfig, AuthMethod, apply_latency};
use crate::core::{Call, Core, dispatch_logged};
use crate::front::access::{AccessMeta, access_log};
use crate::logger::context::LogContext;
use crate::types::envelope::{CODE_OK, MSG_OK};
use crate::types::logger::{LogEntry, LogEvent};
use crate::types::token::{AuthedApp, TokenForm};

#[derive(Clone)]
struct WsState {
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Option<Arc<TokenManager>>,
    policy: AuthPolicy,
    conns: Arc<AtomicUsize>,
}

pub fn router(
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Option<Arc<TokenManager>>,
    meta: AccessMeta,
) -> Router {
    let policy = policy_from(
        config.fronts.ws.common.auth.clone(),
        tokens.clone(),
        Arc::clone(&config.apps),
        config.fronts.ws.common.port,
    );
    let oauth = policy.allows(AuthMethod::OauthCc);
    let state = WsState {
        config,
        core,
        tokens,
        policy,
        conns: Arc::new(AtomicUsize::new(0)),
    };
    let mut app = Router::new().route("/ws", get(handle_upgrade));
    if oauth {
        app = app.route("/oauth/token", axum::routing::post(handle_token));
    }
    app.fallback(handle_not_found)
        .layer(middleware::from_fn(access_log))
        .layer(axum::Extension(meta))
        .with_state(state)
}

async fn handle_not_found(Extension(ctx): Extension<LogContext>) -> Response {
    ctx.set_fail("not_found", "not found", serde_json::json!("not_found"));
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn handle_token(
    axum::Extension(ctx): axum::Extension<LogContext>,
    State(state): State<WsState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    ctx.set_auth("oauth_cc");
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let Some(tokens) = &state.tokens else {
        ctx.set_fail(
            "invalid_client",
            "oauth is not enabled",
            serde_json::json!("invalid_client"),
        );
        return crate::auth::oauth::oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "oauth is not enabled",
        );
    };
    match issue_token(
        &ctx,
        &state.config.apps,
        tokens,
        state.config.methods,
        &headers,
        &form,
    )
    .await
    {
        Ok(resp) => token_success(resp),
        Err(resp) => resp,
    }
}

struct ConnSlot(Arc<AtomicUsize>);

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn try_acquire(conns: &AtomicUsize, max: usize) -> bool {
    let mut cur = conns.load(Ordering::SeqCst);
    loop {
        if cur >= max {
            return false;
        }
        match conns.compare_exchange_weak(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(actual) => cur = actual,
        }
    }
}

#[derive(Clone)]
struct WsConnMeta {
    ip: Option<String>,
    query: Option<String>,
    user_agent: Option<String>,
    referer: Option<String>,
    content_type: Option<String>,
    auth: Option<String>,
}

async fn handle_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<WsState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Extension(ctx): Extension<LogContext>,
) -> Response {
    let max = state.config.fronts.ws.max_conn;
    if !try_acquire(&state.conns, max) {
        ctx.set_fail(
            "unavailable",
            "too many connections",
            serde_json::json!("unavailable"),
        );
        return (StatusCode::SERVICE_UNAVAILABLE, "too many connections").into_response();
    }
    let slot = ConnSlot(Arc::clone(&state.conns));
    let query = crate::logger::redact_query(uri.query());
    let start_auth = std::time::Instant::now();
    let authed = auth_from_upgrade(&state, &headers, uri.query()).await;
    ctx.add_duration_auth(crate::logger::elapsed_ms(start_auth));
    if let Some((app, method)) = &authed {
        ctx.set_client_id(app.client_id.clone());
        ctx.set_auth(method.as_str());
    }
    let meta = WsConnMeta {
        ip: crate::front::access::client_ip_from(&headers, Some(addr)),
        query,
        user_agent: crate::logger::header_text(&headers, &axum::http::header::USER_AGENT),
        referer: crate::logger::header_text(&headers, &axum::http::header::REFERER),
        content_type: crate::logger::header_text(&headers, &axum::http::header::CONTENT_TYPE),
        auth: authed.as_ref().map(|(_, m)| m.as_str().to_string()),
    };
    let app = authed.map(|(app, _)| app);
    ws.on_upgrade(move |socket| async move {
        handle_socket(socket, state, app, meta).await;
        drop(slot);
    })
}

async fn auth_from_upgrade(
    state: &WsState,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Option<(AuthedApp, crate::config::AuthMethod)> {
    let input = AuthInput {
        headers,
        query,
        method: "GET",
        path: "/ws",
        body: &[],
        query_overrides_header: true,
    };
    authenticate(&state.policy, &input)
        .await
        .ok()
        .map(|ok| (ok.app, ok.method))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: WsState,
    mut app: Option<AuthedApp>,
    mut meta: WsConnMeta,
) {
    let ping = Duration::from_secs(state.config.fronts.ws.ping_secs.max(1));
    if app.is_none() {
        match timeout(Duration::from_secs(3), socket.recv()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
                    && v.get("type").and_then(|x| x.as_str()) == Some("auth")
                {
                    let mut headers = HeaderMap::new();
                    if let Some(token) = v.get("access_token").and_then(|x| x.as_str())
                        && let Ok(h) = axum::http::HeaderValue::from_str(&format!("Bearer {token}"))
                    {
                        headers.insert(axum::http::header::AUTHORIZATION, h);
                    }
                    if let Some(key) = v.get("api_key").and_then(|x| x.as_str())
                        && let Ok(h) = axum::http::HeaderValue::from_str(key)
                    {
                        headers.insert("x-api-key", h);
                    }
                    let input = AuthInput {
                        headers: &headers,
                        query: None,
                        method: "GET",
                        path: "/ws",
                        body: &[],
                        query_overrides_header: false,
                    };
                    app = authenticate(&state.policy, &input).await.ok().map(|ok| {
                        meta.auth = Some(ok.method.as_str().to_string());
                        ok.app
                    });
                }
            }
            _ => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        }
        if app.is_none() {
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    }
    let app = app.expect("authed");
    let mut ticker = tokio::time::interval(ping);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let reply = handle_text(&state, &app, &meta, &text).await;
                        if socket.send(Message::Text(reply.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(bin))) => {
                        log_ws_binary(&state, &app, &meta, bin.len() as u64);
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

async fn handle_text(state: &WsState, app: &AuthedApp, meta: &WsConnMeta, text: &str) -> String {
    let start = std::time::Instant::now();
    let bytes_in = Some(text.len() as u64);
    let parsed = serde_json::from_str::<serde_json::Value>(text);
    let Ok(v) = parsed else {
        let request_id = crate::logger::generate_request_id();
        let ctx = frame_ctx(state, &request_id, app, meta);
        ctx.set_fail("bad_request", "invalid json", serde_json::json!(40001));
        let body = serde_json::json!({
            "id": "",
            "request_id": request_id,
            "code": 40001,
            "msg": "invalid json",
            "data": null
        })
        .to_string();
        emit_ws_frame(&ctx, start, 400, bytes_in, Some(body.len() as u64));
        return body;
    };
    let (id, request_id) = ws_id_and_request_id(&v);
    let method = v
        .get("method")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let biz = v
        .get("biz_content")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let ctx = frame_ctx(state, &request_id, app, meta);
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let call = Call::from_app(app, method, biz);
    match dispatch_logged(&state.core, &ctx, call) {
        Ok(data) => {
            ctx.set_code(serde_json::json!(CODE_OK));
            let body = serde_json::json!({
                "id": id,
                "request_id": request_id,
                "code": CODE_OK,
                "msg": MSG_OK,
                "data": data
            })
            .to_string();
            emit_ws_frame(&ctx, start, 200, bytes_in, Some(body.len() as u64));
            body
        }
        Err(err) => {
            let (status, code) = crate::front::util::core_to_envelope(&err);
            ctx.set_code(serde_json::json!(code));
            let body = serde_json::json!({
                "id": id,
                "request_id": request_id,
                "code": code,
                "msg": err.message(),
                "data": null
            })
            .to_string();
            emit_ws_frame(
                &ctx,
                start,
                status.as_u16(),
                bytes_in,
                Some(body.len() as u64),
            );
            body
        }
    }
}

fn ws_id_and_request_id(v: &serde_json::Value) -> (serde_json::Value, String) {
    match v.get("id") {
        None => (
            serde_json::Value::String(String::new()),
            crate::logger::generate_request_id(),
        ),
        Some(id) => {
            let request_id = match id {
                serde_json::Value::String(s) if crate::logger::is_valid_request_id(s) => s.clone(),
                serde_json::Value::Number(n) => {
                    let s = n.to_string();
                    if crate::logger::is_valid_request_id(&s) {
                        s
                    } else {
                        crate::logger::generate_request_id()
                    }
                }
                _ => crate::logger::generate_request_id(),
            };
            (id.clone(), request_id)
        }
    }
}

fn frame_ctx(state: &WsState, request_id: &str, app: &AuthedApp, meta: &WsConnMeta) -> LogContext {
    let ctx = LogContext::new(
        request_id.to_string(),
        "ws".to_string(),
        state.config.fronts.ws.common.port,
    );
    ctx.set_client_id(app.client_id.clone());
    ctx.set_ip(meta.ip.clone());
    ctx.set_query(meta.query.clone());
    ctx.set_user_agent(meta.user_agent.clone());
    ctx.set_referer(meta.referer.clone());
    ctx.set_content_type(meta.content_type.clone());
    if let Some(auth) = &meta.auth {
        ctx.set_auth(auth.clone());
    }
    ctx
}

fn emit_ws_frame(
    ctx: &LogContext,
    start: std::time::Instant,
    status: u16,
    bytes_in: Option<u64>,
    bytes_out: Option<u64>,
) {
    let snap = ctx.snapshot();
    crate::logger::log(&LogEntry {
        request_id: snap.request_id,
        ts: String::new(),
        level: String::new(),
        event: LogEvent::Access,
        method: "WS".to_string(),
        path: "/ws".to_string(),
        query: snap.query,
        status,
        code: snap.code,
        duration: crate::logger::elapsed_ms(start),
        duration_auth: snap.duration_auth,
        duration_core: snap.duration_core,
        duration_latency: snap.duration_latency,
        ip: snap.ip,
        client_id: snap.client_id,
        bytes_in,
        bytes_out,
        error: snap.error,
        error_kind: snap.error_kind,
        input: snap.input,
        out: snap.output,
        front: snap.front,
        port: snap.port,
        rpc: snap.rpc,
        job_id: None,
        user_agent: snap.user_agent,
        referer: snap.referer,
        content_type: snap.content_type,
        auth: snap.auth,
        pid: 0,
        host: None,
    });
}

fn log_ws_binary(state: &WsState, app: &AuthedApp, meta: &WsConnMeta, bytes_in: u64) {
    let request_id = crate::logger::generate_request_id();
    crate::logger::log(&LogEntry {
        request_id,
        ts: String::new(),
        level: String::new(),
        event: LogEvent::Access,
        method: "WS".to_string(),
        path: "/ws".to_string(),
        query: meta.query.clone(),
        status: 400,
        code: Some(serde_json::json!("bad_request")),
        duration: 0,
        duration_auth: None,
        duration_core: None,
        duration_latency: None,
        ip: meta.ip.clone(),
        client_id: Some(app.client_id.clone()),
        bytes_in: Some(bytes_in),
        bytes_out: None,
        error: Some("binary frame".to_string()),
        error_kind: Some("bad_request".to_string()),
        input: serde_json::json!({}),
        out: serde_json::Value::Null,
        front: "ws".to_string(),
        port: state.config.fronts.ws.common.port,
        rpc: None,
        job_id: None,
        user_agent: meta.user_agent.clone(),
        referer: meta.referer.clone(),
        content_type: meta.content_type.clone(),
        auth: meta.auth.clone(),
        pid: 0,
        host: None,
    });
}
