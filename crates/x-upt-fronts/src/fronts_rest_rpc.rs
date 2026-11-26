//! Routers for `rest` (GET/HEAD/POST JSON/Form/Multipart) and `jsonrpc` (single, notification, batch).

use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Multipart, OriginalUri, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use base64::Engine;
use http::{HeaderMap, Method, StatusCode, header};
use x_upt_core::domain::AuthMethod;
use x_upt_core::error::{CoreError, CoreErrorKind};
use x_upt_core::services::Call;
use x_upt_crypto::http_auth::AuthError;

use crate::fronts_gateway::oauth_token_ep;
use crate::http_guard::{FrontCtx, VerifiedCaller, attach_std_headers, verify_http_request};
use crate::logger::{LogOutcome, ReqMeta, build_log_entry};
use crate::snapshot::simulate_latency;

/// Builds the `rest` front `axum::Router`.
pub fn build_rest_router(ctx: FrontCtx) -> Router {
    Router::new()
        .route("/oauth/token", post(oauth_token_ep))
        .route("/gettoken", get(rest_gettoken))
        .route("/posts/search", any(rest_posts))
        .route("/photos/render", any(rest_photos))
        .route("/imgdef/compare", any(rest_imgdef))
        .fallback(rest_not_found)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(ctx)
}

/// Builds the `jsonrpc` front `axum::Router`.
pub fn build_jsonrpc_router(ctx: FrontCtx) -> Router {
    Router::new()
        .route("/oauth/token", post(oauth_token_ep))
        .route("/rpc", post(jsonrpc_ep))
        .fallback(rest_not_found)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(ctx)
}

async fn rest_gettoken(
    State(ctx): State<FrontCtx>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let meta = ReqMeta::from_http("GET", uri.path(), uri.query(), &headers, None, Some(0));
    let req_id = meta.request_id.clone();
    let snap = ctx.runtime.load();
    let allowed_modes = snap
        .fronts
        .get(&ctx.front)
        .map_or_else(Vec::new, |f| f.auth.clone());
    if !allowed_modes.contains(&AuthMethod::Gettoken) {
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::NOT_FOUND,
            "not_found",
            "gettoken disabled",
            None,
        );
    }

    let mut app_id = String::new();
    let mut secret = String::new();
    for (k, v) in url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()) {
        match k.as_ref() {
            "app_id" => app_id = v.into_owned(),
            "secret" => secret = v.into_owned(),
            _ => {}
        }
    }
    let Some(app) = snap
        .apps
        .get(&app_id)
        .filter(|a| x_upt_core::domain::secrets_equal(&a.client_secret, &secret))
    else {
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid app_id or secret",
            None,
        );
    };

    let expire_mins = snap
        .fronts
        .get(&ctx.front)
        .map_or(snap.settings.token_expire_minutes, |f| f.expire_minutes);
    let granted = x_upt_core::domain::granted_methods(&app.methods, snap.dataset.methods);
    let (tok, exp) = ctx.sec.tokens.issue(
        &app.client_id,
        granted,
        expire_mins,
        snap.settings.token_max_per_client,
    );
    let mut resp = (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "access_token": tok,
            "expires_in": exp,
        })),
    )
        .into_response();
    attach_std_headers(&mut resp, &req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 200,
            client_id: Some(app.client_id.clone()),
            auth: Some("gettoken".to_string()),
            ..LogOutcome::default()
        },
    ));
    resp
}

async fn rest_posts(
    State(ctx): State<FrontCtx>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    handle_rest_common(ctx, method, uri, headers, body, "x_upt.posts.search").await
}

async fn rest_photos(
    State(ctx): State<FrontCtx>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    handle_rest_common(ctx, method, uri, headers, body, "x_upt.photos.render").await
}

async fn rest_imgdef(
    State(ctx): State<FrontCtx>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> Response {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if method == Method::POST && ct.starts_with("multipart/form-data") {
        return handle_rest_imgdef_multipart(ctx, uri, headers, req).await;
    }
    let Ok(body) = axum::body::to_bytes(req.into_body(), 32 * 1024 * 1024).await else {
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
            "bad_request",
            "payload too large",
            None,
        );
    };
    handle_rest_common(ctx, method, uri, headers, body, "x_upt.imgdef.compare").await
}

async fn handle_rest_imgdef_multipart(
    ctx: FrontCtx,
    uri: http::Uri,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> Response {
    let meta = ReqMeta::from_http("POST", uri.path(), uri.query(), &headers, None, None);
    let req_id = meta.request_id.clone();
    let snap = ctx.runtime.load();
    let caller =
        match verify_http_request(&ctx, &snap, "POST", uri.path(), uri.query(), &headers, &[]) {
            Ok(c) => c,
            Err(f) => return rest_auth_fail(&ctx, meta, &req_id, f.error, f.duration_ms),
        };
    let Ok(mut mp) = <Multipart as axum::extract::FromRequest<()>>::from_request(req, &()).await
    else {
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::BAD_REQUEST,
            "bad_request",
            "invalid multipart payload",
            None,
        );
    };

    let mut filename = None;
    let mut threshold = None;
    let mut img_b64 = None;
    while let Ok(Some(field)) = mp.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "filename" => {
                if let Ok(t) = field.text().await {
                    filename = Some(t);
                }
            }
            "threshold" => {
                if let Ok(t) = field.text().await {
                    threshold = t.trim().parse::<u64>().ok();
                }
            }
            "image" => {
                if let Ok(b) = field.bytes().await {
                    img_b64 = Some(base64::engine::general_purpose::STANDARD.encode(&b));
                }
            }
            _ => {}
        }
    }
    let mut obj = serde_json::Map::new();
    if let Some(f) = filename {
        obj.insert("filename".to_string(), serde_json::Value::String(f));
    }
    if let Some(t) = threshold {
        obj.insert("threshold".to_string(), serde_json::json!(t));
    }
    if let Some(b64) = img_b64 {
        obj.insert("image".to_string(), serde_json::Value::String(b64));
    }
    dispatch_rest_biz(
        &ctx,
        &snap,
        meta,
        &req_id,
        caller,
        "x_upt.imgdef.compare",
        serde_json::Value::Object(obj),
    )
    .await
}

async fn handle_rest_common(
    ctx: FrontCtx,
    method: Method,
    uri: http::Uri,
    headers: HeaderMap,
    body: Bytes,
    biz_method: &'static str,
) -> Response {
    if !matches!(method, Method::GET | Method::HEAD | Method::POST) {
        let meta = ReqMeta::from_http(
            method.as_str(),
            uri.path(),
            uri.query(),
            &headers,
            None,
            Some(body.len()),
        );
        let req_id = meta.request_id.clone();
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::METHOD_NOT_ALLOWED,
            "bad_request",
            "method not allowed",
            None,
        );
    }
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
    let caller = match verify_http_request(
        &ctx,
        &snap,
        method.as_str(),
        uri.path(),
        uri.query(),
        &headers,
        &body,
    ) {
        Ok(c) => c,
        Err(f) => return rest_auth_fail(&ctx, meta, &req_id, f.error, f.duration_ms),
    };
    if method == Method::HEAD {
        let mut resp = StatusCode::OK.into_response();
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
                rpc: Some(biz_method.to_string()),
                ..LogOutcome::default()
            },
        ));
        return resp;
    }

    let biz = match parse_rest_biz(&method, uri.query(), &headers, &body) {
        Ok(v) => v,
        Err((st, code, msg)) => {
            return rest_error(
                &ctx,
                meta,
                &req_id,
                st,
                code,
                &msg,
                Some(&caller.app.client_id),
            );
        }
    };
    dispatch_rest_biz(&ctx, &snap, meta, &req_id, caller, biz_method, biz).await
}

fn parse_rest_biz(
    method: &Method,
    query: Option<&str>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<serde_json::Value, (StatusCode, &'static str, String)> {
    if *method == Method::GET {
        return Ok(kv_pairs_to_biz(url::form_urlencoded::parse(
            query.unwrap_or("").as_bytes(),
        )));
    }
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ct.contains("json") {
        return serde_json::from_slice(body).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                "bad_request",
                format!("invalid JSON: {e}"),
            )
        });
    }
    if ct.starts_with("application/x-www-form-urlencoded") {
        if body.len() > 2 * 1024 * 1024 {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                "bad_request",
                "form body > 2 MiB".to_string(),
            ));
        }
        return Ok(kv_pairs_to_biz(url::form_urlencoded::parse(body)));
    }
    Err((
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
        "unsupported Content-Type".to_string(),
    ))
}

fn kv_pairs_to_biz<'a>(
    pairs: impl Iterator<Item = (std::borrow::Cow<'a, str>, std::borrow::Cow<'a, str>)>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        let key = k.as_ref();
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "api_key" | "x-api-key" | "access_token"
        ) {
            continue;
        }
        let val = if matches!(
            key,
            "user_id" | "page" | "page_size" | "width" | "height" | "threshold"
        ) {
            v.parse::<u64>().map_or_else(
                |_| serde_json::Value::String(v.into_owned()),
                |n| serde_json::json!(n),
            )
        } else {
            serde_json::Value::String(v.into_owned())
        };
        map.insert(key.to_string(), val);
    }
    serde_json::Value::Object(map)
}

async fn dispatch_rest_biz(
    ctx: &FrontCtx,
    snap: &x_upt_store::dto::RuntimeSnapshotData,
    meta: ReqMeta,
    req_id: &str,
    caller: VerifiedCaller,
    biz_method: &str,
    biz: serde_json::Value,
) -> Response {
    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let res = snap.dataset.dispatch(Call::from_app(
        &caller.app,
        biz_method.to_string(),
        biz.clone(),
    ));
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);
    match res {
        Ok(out) => {
            let mut resp = (StatusCode::OK, axum::Json(out.clone())).into_response();
            attach_std_headers(&mut resp, req_id, None);
            ctx.logger.record(build_log_entry(
                ctx.front,
                ctx.port,
                meta,
                LogOutcome {
                    status: 200,
                    duration_auth: Some(caller.duration_ms),
                    duration_core: Some(core_ms),
                    duration_latency: Some(lat_ms),
                    client_id: Some(caller.app.client_id),
                    auth: Some(caller.method.as_str().to_string()),
                    rpc: Some(biz_method.to_string()),
                    input: biz,
                    out,
                    ..LogOutcome::default()
                },
            ));
            resp
        }
        Err(err) => {
            let st = map_rest_status(&err);
            rest_error(
                ctx,
                meta,
                req_id,
                st,
                err.kind.as_str(),
                err.message(),
                Some(&caller.app.client_id),
            )
        }
    }
}

fn map_rest_status(err: &CoreError) -> StatusCode {
    match err.kind {
        CoreErrorKind::BadRequest(_) | CoreErrorKind::UnknownMethod => StatusCode::BAD_REQUEST,
        CoreErrorKind::Forbidden => StatusCode::FORBIDDEN,
        CoreErrorKind::NotFound(_) => StatusCode::NOT_FOUND,
        CoreErrorKind::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
        CoreErrorKind::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub(crate) fn rest_auth_fail(
    ctx: &FrontCtx,
    meta: ReqMeta,
    req_id: &str,
    err: AuthError,
    auth_ms: u64,
) -> Response {
    let (code, msg, challenges) = match err {
        AuthError::Missing | AuthError::InvalidFormat => {
            ("unauthorized", "missing or malformed credentials", None)
        }
        AuthError::InvalidToken => ("invalid_token", "invalid or expired token", None),
        AuthError::DigestChallenge(ch) => {
            ("unauthorized", "digest authentication required", Some(ch))
        }
    };
    let mut resp = (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({
            "error": code,
            "message": msg,
            "request_id": req_id,
        })),
    )
        .into_response();
    attach_std_headers(&mut resp, req_id, challenges.as_deref());
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 401,
            duration_auth: Some(auth_ms),
            error: Some(msg.to_string()),
            error_kind: Some(code.to_string()),
            ..LogOutcome::default()
        },
    ));
    resp
}

pub(crate) fn rest_error(
    ctx: &FrontCtx,
    meta: ReqMeta,
    req_id: &str,
    status: StatusCode,
    code: &str,
    message: &str,
    client_id: Option<&str>,
) -> Response {
    let mut resp = (
        status,
        axum::Json(serde_json::json!({
            "error": code,
            "message": message,
            "request_id": req_id,
        })),
    )
        .into_response();
    attach_std_headers(&mut resp, req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: status.as_u16(),
            client_id: client_id.map(ToString::to_string),
            error: Some(message.to_string()),
            error_kind: Some(code.to_string()),
            ..LogOutcome::default()
        },
    ));
    resp
}

pub(crate) async fn rest_not_found(
    State(ctx): State<FrontCtx>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let meta = ReqMeta::from_http(
        method.as_str(),
        uri.path(),
        uri.query(),
        &headers,
        None,
        Some(0),
    );
    let req_id = meta.request_id.clone();
    rest_error(
        &ctx,
        meta,
        &req_id,
        StatusCode::NOT_FOUND,
        "not_found",
        "not found",
        None,
    )
}

async fn jsonrpc_ep(
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

    let raw_val: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            let mut resp = (
                StatusCode::BAD_REQUEST,
                axum::Json(rpc_err_obj(
                    serde_json::Value::Null,
                    -32700,
                    "Parse error",
                    &req_id,
                )),
            )
                .into_response();
            attach_std_headers(&mut resp, &req_id, None);
            ctx.logger.record(build_log_entry(
                ctx.front,
                ctx.port,
                meta,
                LogOutcome {
                    status: 400,
                    code: Some(serde_json::json!(-32700)),
                    error: Some("Parse error".to_string()),
                    error_kind: Some("parse_error".to_string()),
                    ..LogOutcome::default()
                },
            ));
            return resp;
        }
    };

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
        Err(fail) => {
            if let AuthError::DigestChallenge(ch) = fail.error {
                return rest_auth_fail(
                    &ctx,
                    meta,
                    &req_id,
                    AuthError::DigestChallenge(ch),
                    fail.duration_ms,
                );
            }
            let id_val = raw_val
                .get("id")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let mut resp = (
                StatusCode::OK,
                axum::Json(rpc_err_obj(id_val, -32001, "unauthorized", &req_id)),
            )
                .into_response();
            attach_std_headers(&mut resp, &req_id, None);
            ctx.logger.record(build_log_entry(
                ctx.front,
                ctx.port,
                meta,
                LogOutcome {
                    status: 200,
                    code: Some(serde_json::json!(-32001)),
                    duration_auth: Some(fail.duration_ms),
                    error: Some("unauthorized".to_string()),
                    error_kind: Some("unauthorized".to_string()),
                    ..LogOutcome::default()
                },
            ));
            return resp;
        }
    };

    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let (status, out_val, rpc_name) = execute_jsonrpc_value(&snap, &caller.app, &raw_val, &req_id);
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);

    let mut resp = if status == StatusCode::NO_CONTENT {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::OK, axum::Json(out_val.clone())).into_response()
    };
    attach_std_headers(&mut resp, &req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: status.as_u16(),
            duration_auth: Some(caller.duration_ms),
            duration_core: Some(core_ms),
            duration_latency: Some(lat_ms),
            client_id: Some(caller.app.client_id),
            auth: Some(caller.method.as_str().to_string()),
            rpc: rpc_name,
            input: raw_val,
            out: out_val,
            ..LogOutcome::default()
        },
    ));
    resp
}

fn execute_jsonrpc_value(
    snap: &x_upt_store::dto::RuntimeSnapshotData,
    app: &x_upt_core::domain::AuthedApp,
    val: &serde_json::Value,
    req_id: &str,
) -> (StatusCode, serde_json::Value, Option<String>) {
    if let Some(arr) = val.as_array() {
        if arr.is_empty() {
            return (
                StatusCode::OK,
                rpc_err_obj(serde_json::Value::Null, -32600, "Invalid Request", req_id),
                None,
            );
        }
        let mut results = Vec::new();
        for item in arr {
            if let Some(r) = execute_single_rpc(snap, app, item, req_id).0 {
                results.push(r);
            }
        }
        if results.is_empty() {
            return (
                StatusCode::NO_CONTENT,
                serde_json::Value::Null,
                Some("batch".to_string()),
            );
        }
        return (
            StatusCode::OK,
            serde_json::Value::Array(results),
            Some("batch".to_string()),
        );
    }
    let (opt, name) = execute_single_rpc(snap, app, val, req_id);
    match opt {
        Some(r) => (StatusCode::OK, r, name),
        None => (StatusCode::NO_CONTENT, serde_json::Value::Null, name),
    }
}

fn execute_single_rpc(
    snap: &x_upt_store::dto::RuntimeSnapshotData,
    app: &x_upt_core::domain::AuthedApp,
    item: &serde_json::Value,
    req_id: &str,
) -> (Option<serde_json::Value>, Option<String>) {
    let Some(obj) = item.as_object() else {
        return (
            Some(rpc_err_obj(
                serde_json::Value::Null,
                -32600,
                "Invalid Request",
                req_id,
            )),
            None,
        );
    };
    let is_notif = !obj.contains_key("id");
    let id_val = obj.get("id").cloned().unwrap_or(serde_json::Value::Null);
    if obj.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0") {
        return (
            Some(rpc_err_obj(id_val, -32600, "Invalid Request", req_id)),
            None,
        );
    }
    let Some(method) = obj.get("method").and_then(serde_json::Value::as_str) else {
        return (
            Some(rpc_err_obj(id_val, -32600, "Invalid Request", req_id)),
            None,
        );
    };
    let params = obj
        .get("params")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    if !params.is_object() {
        return (
            (!is_notif).then(|| rpc_err_obj(id_val, -32602, "params must be an object", req_id)),
            Some(method.to_string()),
        );
    }
    let res = snap
        .dataset
        .dispatch(Call::from_app(app, method.to_string(), params));
    if is_notif {
        return (None, Some(method.to_string()));
    }
    let out = match res {
        Ok(data) => serde_json::json!({
            "jsonrpc": "2.0",
            "result": data,
            "id": id_val,
        }),
        Err(err) => {
            let code = match err.kind {
                CoreErrorKind::UnknownMethod => -32601,
                CoreErrorKind::BadRequest(_) | CoreErrorKind::Unprocessable(_) => -32602,
                CoreErrorKind::Forbidden => -32003,
                CoreErrorKind::NotFound(_) => -32004,
                CoreErrorKind::Internal(_) => -32603,
            };
            rpc_err_obj(id_val, code, err.message(), req_id)
        }
    };
    (Some(out), Some(method.to_string()))
}

fn rpc_err_obj(id: serde_json::Value, code: i32, msg: &str, req_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "error": {
            "code": code,
            "message": msg,
            "data": { "request_id": req_id },
        },
        "id": id,
    })
}
