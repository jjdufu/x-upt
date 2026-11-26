//! Routers for `ws` (`/ws`), `job` (`/jobs`, `/jobs/{id}`), and the 6 `envelope_*` fronts.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, OriginalUri, Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use serde::Deserialize;
use tokio::sync::Semaphore;
use x_upt_core::domain::{
    AuthedApp, CODE_BAD_REQUEST, CODE_FORBIDDEN, CODE_INTERNAL, CODE_NOT_FOUND, CODE_OK,
    CODE_UNKNOWN_METHOD, CODE_UNPROCESSABLE, MSG_OK, is_known_method,
};
use x_upt_core::error::{CoreError, CoreErrorKind};
use x_upt_core::logger_types::resolve_request_id_raw;
use x_upt_core::services::Call;
use x_upt_crypto::pipeline::{DriveCtx, drive};
use x_upt_crypto::slots::Incoming;

use crate::fronts_gateway::oauth_token_ep;
use crate::fronts_rest_rpc::{rest_auth_fail, rest_error, rest_not_found};
use crate::http_guard::{FrontCtx, attach_std_headers, verify_http_request};
use crate::logger::{LogOutcome, ReqMeta, build_log_entry};
use crate::snapshot::simulate_latency;

const ENVELOPE_BODY_MAX: usize = 1024 * 1024;

#[derive(Clone)]
struct WsState {
    ctx: FrontCtx,
    sem: Arc<Semaphore>,
    ping_secs: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobSubmitReq {
    method: String,
    biz_content: serde_json::Value,
}

/// Builds the `ws` front `axum::Router`.
pub fn build_ws_router(ctx: FrontCtx, max_conn: usize, ping_secs: u64) -> Router {
    let ws_state = WsState {
        ctx: ctx.clone(),
        sem: Arc::new(Semaphore::new(max_conn.max(1))),
        ping_secs: ping_secs.max(1),
    };
    let oauth_sub = Router::new()
        .route("/oauth/token", post(oauth_token_ep))
        .with_state(ctx.clone());
    Router::new()
        .route("/ws", get(ws_upgrade_ep))
        .with_state(ws_state)
        .merge(oauth_sub)
        .fallback(rest_not_found)
        .with_state(ctx)
}

/// Builds the `job` front `axum::Router`.
pub fn build_job_router(ctx: FrontCtx) -> Router {
    Router::new()
        .route("/oauth/token", post(oauth_token_ep))
        .route("/jobs", post(job_submit_ep))
        .route("/jobs/{id}", get(job_poll_ep))
        .fallback(rest_not_found)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(ctx)
}

/// Builds an `envelope_*` front `axum::Router`.
pub fn build_envelope_router(ctx: FrontCtx) -> Router {
    Router::new().fallback(envelope_ep).with_state(ctx)
}

async fn ws_upgrade_ep(
    State(state): State<WsState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Ok(permit) = Arc::clone(&state.sem).try_acquire_owned() else {
        let meta = ReqMeta::from_http("GET", uri.path(), uri.query(), &headers, None, Some(0));
        let req_id = meta.request_id.clone();
        return rest_error(
            &state.ctx,
            meta,
            &req_id,
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "max WebSocket connections reached",
            None,
        );
    };

    let snap = state.ctx.runtime.load();
    let handshake_auth = verify_http_request(
        &state.ctx,
        &snap,
        "GET",
        uri.path(),
        uri.query(),
        &headers,
        &[],
    )
    .ok();
    let has_creds_in_handshake = headers.contains_key(header::AUTHORIZATION)
        || headers.contains_key("x-api-key")
        || uri
            .query()
            .is_some_and(|q| q.contains("api_key=") || q.contains("access_token="));

    if has_creds_in_handshake && handshake_auth.is_none() {
        let meta = ReqMeta::from_http("GET", uri.path(), uri.query(), &headers, None, Some(0));
        let req_id = meta.request_id.clone();
        return rest_error(
            &state.ctx,
            meta,
            &req_id,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid WebSocket handshake credentials",
            None,
        );
    }

    let initial_app = handshake_auth.map(|c| (c.app, c.method.as_str().to_string()));
    ws.on_upgrade(move |socket| async move {
        let _keep = permit;
        run_ws_session(state, socket, initial_app).await;
    })
}

async fn run_ws_session(
    state: WsState,
    mut socket: WebSocket,
    initial_app: Option<(AuthedApp, String)>,
) {
    let (authed_app, auth_mode) = match initial_app {
        Some(pair) => pair,
        None => {
            let Ok(Some(Ok(WsMessage::Text(first_txt)))) =
                tokio::time::timeout(Duration::from_secs(3), socket.recv()).await
            else {
                drop(socket.send(WsMessage::Close(None)).await);
                return;
            };
            let Some(pair) = verify_ws_first_frame(&state.ctx, &first_txt) else {
                drop(socket.send(WsMessage::Close(None)).await);
                return;
            };
            pair
        }
    };

    let mut ping_iv = tokio::time::interval(Duration::from_secs(state.ping_secs));
    ping_iv.tick().await;

    while let Some(evt) = tokio::select! {
        _ = ping_iv.tick() => {
            if socket.send(WsMessage::Ping(Bytes::new())).await.is_err() {
                None
            } else {
                Some(None)
            }
        }
        msg = socket.recv() => msg.map(Some),
    } {
        let Some(msg_res) = evt else {
            continue;
        };
        let Ok(msg) = msg_res else {
            break;
        };
        match msg {
            WsMessage::Text(txt) => {
                let reply = handle_ws_text(&state.ctx, &authed_app, &auth_mode, &txt).await;
                if socket.send(WsMessage::Text(reply.into())).await.is_err() {
                    break;
                }
            }
            WsMessage::Binary(bin) => {
                let meta =
                    ReqMeta::from_http("WS", "/ws", None, &HeaderMap::new(), None, Some(bin.len()));
                state.ctx.logger.record(build_log_entry(
                    state.ctx.front,
                    state.ctx.port,
                    meta,
                    LogOutcome {
                        status: 400,
                        client_id: Some(authed_app.client_id),
                        error: Some("binary frames not supported".to_string()),
                        error_kind: Some("bad_request".to_string()),
                        ..LogOutcome::default()
                    },
                ));
                drop(socket.send(WsMessage::Close(None)).await);
                break;
            }
            WsMessage::Close(_) => break,
            WsMessage::Ping(_) | WsMessage::Pong(_) => {}
        }
    }
}

fn verify_ws_first_frame(ctx: &FrontCtx, text: &str) -> Option<(AuthedApp, String)> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    if val.get("type").and_then(serde_json::Value::as_str) != Some("auth") {
        return None;
    }
    let mut hdrs = HeaderMap::new();
    if let Some(tok) = val.get("access_token").and_then(serde_json::Value::as_str) {
        let hv = HeaderValue::from_str(&format!("Bearer {tok}")).ok()?;
        hdrs.insert(header::AUTHORIZATION, hv);
    } else {
        let key = val.get("api_key").and_then(serde_json::Value::as_str)?;
        let hv = HeaderValue::from_str(key).ok()?;
        hdrs.insert("x-api-key", hv);
    }
    let snap = ctx.runtime.load();
    let ok = verify_http_request(ctx, &snap, "GET", "/ws", None, &hdrs, &[]).ok()?;
    Some((ok.app, ok.method.as_str().to_string()))
}

async fn handle_ws_text(ctx: &FrontCtx, app: &AuthedApp, auth_mode: &str, text: &str) -> String {
    let raw: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            let rid = resolve_request_id_raw(None);
            return serde_json::json!({
                "id": serde_json::Value::Null,
                "request_id": rid,
                "code": CODE_BAD_REQUEST,
                "msg": format!("invalid JSON frame: {e}"),
                "data": serde_json::Value::Null,
            })
            .to_string();
        }
    };
    let id_val = raw.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let id_str = match &id_val {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    let req_id = resolve_request_id_raw(id_str.as_deref());
    let mut meta = ReqMeta::from_http("WS", "/ws", None, &HeaderMap::new(), None, Some(text.len()));
    meta.request_id = req_id.clone();

    let method = raw
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let biz = raw
        .get("biz_content")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let snap = ctx.runtime.load();
    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let res = snap
        .dataset
        .dispatch(Call::from_app(app, method.clone(), biz.clone()));
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);

    match res {
        Ok(data) => {
            let reply = serde_json::json!({
                "id": id_val,
                "request_id": req_id,
                "code": CODE_OK,
                "msg": MSG_OK,
                "data": data,
            })
            .to_string();
            ctx.logger.record(build_log_entry(
                ctx.front,
                ctx.port,
                meta,
                LogOutcome {
                    status: 200,
                    code: Some(serde_json::json!(CODE_OK)),
                    duration_core: Some(core_ms),
                    duration_latency: Some(lat_ms),
                    client_id: Some(app.client_id.clone()),
                    bytes_out: Some(reply.len()),
                    auth: Some(auth_mode.to_string()),
                    rpc: Some(method),
                    input: biz,
                    out: data,
                    ..LogOutcome::default()
                },
            ));
            reply
        }
        Err(err) => {
            let code = map_ws_code(&err);
            let reply = serde_json::json!({
                "id": id_val,
                "request_id": req_id,
                "code": code,
                "msg": err.message(),
                "data": serde_json::Value::Null,
            })
            .to_string();
            ctx.logger.record(build_log_entry(
                ctx.front,
                ctx.port,
                meta,
                LogOutcome {
                    status: 400,
                    code: Some(serde_json::json!(code)),
                    duration_core: Some(core_ms),
                    duration_latency: Some(lat_ms),
                    client_id: Some(app.client_id.clone()),
                    bytes_out: Some(reply.len()),
                    auth: Some(auth_mode.to_string()),
                    rpc: Some(method),
                    error: Some(err.message().to_string()),
                    error_kind: Some(err.kind.as_str().to_string()),
                    input: biz,
                    ..LogOutcome::default()
                },
            ));
            reply
        }
    }
}

fn map_ws_code(err: &CoreError) -> i32 {
    match err.kind {
        CoreErrorKind::BadRequest(_) => CODE_BAD_REQUEST,
        CoreErrorKind::UnknownMethod => CODE_UNKNOWN_METHOD,
        CoreErrorKind::Forbidden => CODE_FORBIDDEN,
        CoreErrorKind::NotFound(_) => CODE_NOT_FOUND,
        CoreErrorKind::Unprocessable(_) => CODE_UNPROCESSABLE,
        CoreErrorKind::Internal(_) => CODE_INTERNAL,
    }
}

async fn job_submit_ep(
    State(ctx): State<FrontCtx>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let meta = ReqMeta::from_http(
        "POST",
        uri.path(),
        uri.query(),
        &headers,
        None,
        Some(body.len()),
    );
    let req_id = meta.request_id.clone();
    let snap = ctx.runtime.load();
    let caller = match verify_http_request(
        &ctx,
        &snap,
        "POST",
        uri.path(),
        uri.query(),
        &headers,
        &body,
    ) {
        Ok(c) => c,
        Err(f) => return rest_auth_fail(&ctx, meta, &req_id, f.error, f.duration_ms),
    };
    let parsed: JobSubmitReq = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return rest_error(
                &ctx,
                meta,
                &req_id,
                StatusCode::BAD_REQUEST,
                "bad_request",
                &format!("invalid JSON: {e}"),
                Some(&caller.app.client_id),
            );
        }
    };
    let m = parsed.method.trim();
    if !is_known_method(m) || !snap.dataset.methods.is_enabled(m) {
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::BAD_REQUEST,
            "unknown_method",
            "unknown method",
            Some(&caller.app.client_id),
        );
    }
    if !caller.app.methods.contains(m) {
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::FORBIDDEN,
            "forbidden",
            "method not granted",
            Some(&caller.app.client_id),
        );
    }
    if !parsed.biz_content.is_object() {
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::BAD_REQUEST,
            "bad_request",
            "biz_content must be an object",
            Some(&caller.app.client_id),
        );
    }

    let job_id = ctx.jobs.submit(
        &caller.app,
        m.to_string(),
        parsed.biz_content.clone(),
        req_id.clone(),
    );
    let out = serde_json::json!({ "job_id": job_id, "status": "pending" });
    let mut resp = (StatusCode::ACCEPTED, axum::Json(out.clone())).into_response();
    attach_std_headers(&mut resp, &req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 202,
            duration_auth: Some(caller.duration_ms),
            client_id: Some(caller.app.client_id),
            auth: Some(caller.method.as_str().to_string()),
            rpc: Some(m.to_string()),
            job_id: Some(job_id),
            input: parsed.biz_content,
            out,
            ..LogOutcome::default()
        },
    ));
    resp
}

async fn job_poll_ep(
    State(ctx): State<FrontCtx>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let meta = ReqMeta::from_http("GET", uri.path(), uri.query(), &headers, None, Some(0));
    let req_id = meta.request_id.clone();
    let snap = ctx.runtime.load();
    let caller =
        match verify_http_request(&ctx, &snap, "GET", uri.path(), uri.query(), &headers, &[]) {
            Ok(c) => c,
            Err(f) => return rest_auth_fail(&ctx, meta, &req_id, f.error, f.duration_ms),
        };

    let Some(view) = ctx
        .jobs
        .poll(&id, &caller.app.client_id, snap.settings.job_expire_seconds)
    else {
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::NOT_FOUND,
            "not_found",
            "job not found",
            Some(&caller.app.client_id),
        );
    };
    let out = serde_json::to_value(&view).unwrap_or(serde_json::Value::Null);
    let mut resp = (StatusCode::OK, axum::Json(out.clone())).into_response();
    attach_std_headers(&mut resp, &req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 200,
            duration_auth: Some(caller.duration_ms),
            client_id: Some(caller.app.client_id),
            auth: Some(caller.method.as_str().to_string()),
            job_id: Some(id),
            out,
            ..LogOutcome::default()
        },
    ));
    resp
}

async fn envelope_ep(
    State(ctx): State<FrontCtx>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> Response {
    let Ok(body) = axum::body::to_bytes(req.into_body(), ENVELOPE_BODY_MAX).await else {
        let meta = ReqMeta::from_http(
            method.as_str(),
            uri.path(),
            uri.query(),
            &headers,
            None,
            None,
        );
        let req_id = meta.request_id.clone();
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "envelope request body exceeds 1 MiB limit",
            None,
        );
    };

    let meta = ReqMeta::from_http(
        method.as_str(),
        uri.path(),
        uri.query(),
        &headers,
        None,
        Some(body.len()),
    );
    let req_id = meta.request_id.clone();
    let snap = ctx.runtime.load();
    let lat_ms = simulate_latency(&snap.settings).await;

    let profiles = snap
        .compiled_envelopes
        .get(&ctx.front)
        .map_or(&[][..], Vec::as_slice);
    let hdr_vec: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
        .collect();

    let t_core = Instant::now();
    let out = drive(
        &DriveCtx {
            front: ctx.front,
            profiles,
            tokens: ctx
                .front
                .is_token_envelope()
                .then_some(ctx.sec.envelope_tokens.as_ref()),
            nonces: ctx.sec.sign_nonces.as_ref(),
            sign_window_seconds: snap.settings.sign_window_seconds,
        },
        &Incoming {
            method: method.as_str().to_string(),
            path: uri.path().to_string(),
            query: uri.query().unwrap_or("").to_string(),
            headers: hdr_vec,
            body: body.to_vec(),
        },
    );
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);

    let st = StatusCode::from_u16(out.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp = st.into_response();
    *resp.body_mut() = axum::body::Body::from(out.body.clone());
    if let Ok(hv) = HeaderValue::from_str(out.content_type) {
        resp.headers_mut().insert(header::CONTENT_TYPE, hv);
    }
    attach_std_headers(&mut resp, &req_id, None);

    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: out.status,
            duration_core: Some(core_ms),
            duration_latency: Some(lat_ms),
            bytes_out: Some(out.body.len()),
            auth: Some(out.auth.to_string()),
            error: out.error.clone(),
            error_kind: out.error,
            input: out.log_in,
            out: out.log_out,
            ..LogOutcome::default()
        },
    ));
    resp
}
