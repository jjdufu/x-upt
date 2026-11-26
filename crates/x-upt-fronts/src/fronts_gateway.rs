//! Routers for `oauth_gateway` (`/v3/oauth/token`, `/v3/gateway`) and `sign_gateway` (`/gateway.do`).

use std::collections::BTreeMap;
use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, OriginalUri, State};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use http::{HeaderMap, Method, StatusCode};
use serde::Deserialize;
use sha2::Sha256;
use x_upt_core::domain::{
    AuthedApp, CODE_BAD_REQUEST, CODE_FORBIDDEN, CODE_INTERNAL, CODE_INVALID_TOKEN, CODE_NOT_FOUND,
    CODE_OK, CODE_UNAUTHORIZED, CODE_UNKNOWN_METHOD, CODE_UNPROCESSABLE, GatewayEnvelope, MSG_OK,
    granted_methods, secrets_equal_bytes,
};
use x_upt_core::error::{CoreError, CoreErrorKind};
use x_upt_core::services::Call;
use x_upt_crypto::http_auth::AuthError;

use crate::http_guard::{FrontCtx, attach_std_headers, verify_http_request};
use crate::logger::{LogOutcome, ReqMeta, build_log_entry};
use crate::snapshot::simulate_latency;
use crate::token::handle_oauth_token;

type HmacSha256 = Hmac<Sha256>;

const SIGN_FIELDS: [&str; 9] = [
    "app_id",
    "biz_content",
    "charset",
    "method",
    "nonce",
    "sign",
    "sign_type",
    "timestamp",
    "version",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewayRequest {
    method: String,
    biz_content: serde_json::Value,
}

struct GatewayFail<'a> {
    status: StatusCode,
    code: i32,
    msg: String,
    client_id: Option<&'a str>,
    auth: Option<&'a str>,
    rpc: Option<String>,
    err_kind: &'a str,
    input: serde_json::Value,
}

/// Builds the `oauth_gateway` `axum::Router`.
pub fn build_oauth_gateway_router(ctx: FrontCtx) -> Router {
    Router::new()
        .route("/v3/oauth/token", post(oauth_token_ep))
        .route("/v3/gateway", post(oauth_gateway_ep))
        .fallback(oauth_not_found)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(ctx)
}

/// Builds the `sign_gateway` `axum::Router`.
pub fn build_sign_gateway_router(ctx: FrontCtx) -> Router {
    Router::new()
        .route("/gateway.do", post(sign_gateway_ep))
        .fallback(sign_not_found)
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .with_state(ctx)
}

pub(crate) async fn oauth_token_ep(
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
    let (status, mut resp, client_id) =
        handle_oauth_token(ctx.front, &ctx.runtime, &ctx.sec.tokens, &headers, &body);
    attach_std_headers(&mut resp, &req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: status.as_u16(),
            client_id,
            auth: Some("oauth_cc".to_string()),
            ..LogOutcome::default()
        },
    ));
    resp
}

async fn oauth_gateway_ep(
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
        Err(fail) => return oauth_auth_fail(&ctx, meta, &req_id, fail.error, fail.duration_ms),
    };

    let parsed: GatewayRequest = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return finish_gateway_fail(
                &ctx,
                meta,
                &req_id,
                GatewayFail {
                    status: StatusCode::BAD_REQUEST,
                    code: CODE_BAD_REQUEST,
                    msg: format!("invalid JSON body: {e}"),
                    client_id: Some(&caller.app.client_id),
                    auth: Some(caller.method.as_str()),
                    rpc: None,
                    err_kind: "bad_request",
                    input: serde_json::Value::Null,
                },
            );
        }
    };

    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let res = snap.dataset.dispatch(Call::from_app(
        &caller.app,
        parsed.method.clone(),
        parsed.biz_content.clone(),
    ));
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);

    match res {
        Ok(data) => {
            let mut resp = (
                StatusCode::OK,
                axum::Json(GatewayEnvelope {
                    code: CODE_OK,
                    msg: MSG_OK.to_string(),
                    request_id: req_id.clone(),
                    data: data.clone(),
                }),
            )
                .into_response();
            attach_std_headers(&mut resp, &req_id, None);
            ctx.logger.record(build_log_entry(
                ctx.front,
                ctx.port,
                meta,
                LogOutcome {
                    status: 200,
                    code: Some(serde_json::json!(CODE_OK)),
                    duration_auth: Some(caller.duration_ms),
                    duration_core: Some(core_ms),
                    duration_latency: Some(lat_ms),
                    client_id: Some(caller.app.client_id),
                    rpc: Some(parsed.method),
                    auth: Some(caller.method.as_str().to_string()),
                    input: parsed.biz_content,
                    out: data,
                    ..LogOutcome::default()
                },
            ));
            resp
        }
        Err(err) => {
            let (http_st, biz_code) = map_gateway_err(&err);
            finish_gateway_fail(
                &ctx,
                meta,
                &req_id,
                GatewayFail {
                    status: http_st,
                    code: biz_code,
                    msg: err.message().to_string(),
                    client_id: Some(&caller.app.client_id),
                    auth: Some(caller.method.as_str()),
                    rpc: Some(parsed.method),
                    err_kind: err.kind.as_str(),
                    input: parsed.biz_content,
                },
            )
        }
    }
}

fn oauth_auth_fail(
    ctx: &FrontCtx,
    meta: ReqMeta,
    req_id: &str,
    err: AuthError,
    auth_ms: u64,
) -> Response {
    let (code, msg, challenges) = match err {
        AuthError::Missing | AuthError::InvalidFormat => {
            (CODE_UNAUTHORIZED, "missing or malformed credentials", None)
        }
        AuthError::InvalidToken => (CODE_INVALID_TOKEN, "invalid or expired token", None),
        AuthError::DigestChallenge(ch) => (CODE_UNAUTHORIZED, "digest challenge", Some(ch)),
    };
    let mut resp = (
        StatusCode::UNAUTHORIZED,
        axum::Json(GatewayEnvelope {
            code,
            msg: msg.to_string(),
            request_id: req_id.to_string(),
            data: serde_json::Value::Null,
        }),
    )
        .into_response();
    attach_std_headers(&mut resp, req_id, challenges.as_deref());
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 401,
            code: Some(serde_json::json!(code)),
            duration_auth: Some(auth_ms),
            error: Some(msg.to_string()),
            error_kind: Some("unauthorized".to_string()),
            ..LogOutcome::default()
        },
    ));
    resp
}

fn map_gateway_err(err: &CoreError) -> (StatusCode, i32) {
    match err.kind {
        CoreErrorKind::BadRequest(_) => (StatusCode::BAD_REQUEST, CODE_BAD_REQUEST),
        CoreErrorKind::UnknownMethod => (StatusCode::BAD_REQUEST, CODE_UNKNOWN_METHOD),
        CoreErrorKind::Forbidden => (StatusCode::FORBIDDEN, CODE_FORBIDDEN),
        CoreErrorKind::NotFound(_) => (StatusCode::NOT_FOUND, CODE_NOT_FOUND),
        CoreErrorKind::Unprocessable(_) => (StatusCode::UNPROCESSABLE_ENTITY, CODE_UNPROCESSABLE),
        CoreErrorKind::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, CODE_INTERNAL),
    }
}

fn finish_gateway_fail(
    ctx: &FrontCtx,
    meta: ReqMeta,
    req_id: &str,
    fail: GatewayFail<'_>,
) -> Response {
    let mut resp = (
        fail.status,
        axum::Json(GatewayEnvelope {
            code: fail.code,
            msg: fail.msg.clone(),
            request_id: req_id.to_string(),
            data: serde_json::Value::Null,
        }),
    )
        .into_response();
    attach_std_headers(&mut resp, req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: fail.status.as_u16(),
            code: Some(serde_json::json!(fail.code)),
            client_id: fail.client_id.map(ToString::to_string),
            auth: fail.auth.map(ToString::to_string),
            rpc: fail.rpc,
            error: Some(fail.msg),
            error_kind: Some(fail.err_kind.to_string()),
            input: fail.input,
            ..LogOutcome::default()
        },
    ));
    resp
}

async fn oauth_not_found(
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
    finish_gateway_fail(
        &ctx,
        meta,
        &req_id,
        GatewayFail {
            status: StatusCode::NOT_FOUND,
            code: CODE_NOT_FOUND,
            msg: "not found".to_string(),
            client_id: None,
            auth: None,
            rpc: None,
            err_kind: "not_found",
            input: serde_json::Value::Null,
        },
    )
}

async fn sign_not_found(
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
    let mut resp = (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "error": "not_found",
            "message": "not found",
            "request_id": req_id,
        })),
    )
        .into_response();
    attach_std_headers(&mut resp, &req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 404,
            error: Some("not found".to_string()),
            error_kind: Some("not_found".to_string()),
            ..LogOutcome::default()
        },
    ));
    resp
}

async fn sign_gateway_ep(
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

    let params = match parse_sign_form(&body) {
        Ok(m) => m,
        Err((sub_code, sub_msg)) => {
            return sign_fail(&ctx, meta, &req_id, sub_code, &sub_msg, None);
        }
    };

    let app_id = params.get("app_id").map_or("", String::as_str);
    let Some(app_rec) = snap.apps.get(app_id) else {
        return sign_fail(
            &ctx,
            meta,
            &req_id,
            "isv.invalid-app-id",
            "unknown app_id",
            None,
        );
    };

    if let Err((sub_code, sub_msg)) =
        verify_sign_envelope(&ctx, &snap, &params, &app_rec.client_secret)
    {
        return sign_fail(&ctx, meta, &req_id, sub_code, &sub_msg, Some(app_id));
    }

    let method = params.get("method").cloned().unwrap_or_default();
    let biz_str = params.get("biz_content").map_or("", String::as_str);
    let biz_val: serde_json::Value = match serde_json::from_str(biz_str) {
        Ok(v) => v,
        Err(e) => {
            return sign_fail(
                &ctx,
                meta,
                &req_id,
                "isv.invalid-parameter",
                &format!("invalid biz_content JSON: {e}"),
                Some(app_id),
            );
        }
    };

    let authed = AuthedApp {
        client_id: app_rec.client_id.clone(),
        methods: granted_methods(&app_rec.methods, snap.dataset.methods),
    };
    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let res = snap
        .dataset
        .dispatch(Call::from_app(&authed, method.clone(), biz_val.clone()));
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);

    match res {
        Ok(data) => build_sign_success(
            &ctx,
            meta,
            &req_id,
            &app_rec.client_id,
            &app_rec.client_secret,
            &method,
            (lat_ms, core_ms, biz_val, data),
        ),
        Err(err) => {
            let sub_code = match err.kind {
                CoreErrorKind::UnknownMethod => "isv.invalid-method",
                CoreErrorKind::Forbidden => "isv.insufficient-isv-permissions",
                CoreErrorKind::NotFound(_) => "isv.resource-not-exist",
                CoreErrorKind::BadRequest(_) | CoreErrorKind::Unprocessable(_) => {
                    "isv.invalid-parameter"
                }
                CoreErrorKind::Internal(_) => "isp.unknow-error",
            };
            sign_fail(
                &ctx,
                meta,
                &req_id,
                sub_code,
                err.message(),
                Some(&app_rec.client_id),
            )
        }
    }
}

fn parse_sign_form(body: &[u8]) -> Result<BTreeMap<String, String>, (&'static str, String)> {
    let mut map = BTreeMap::new();
    for (k, v) in url::form_urlencoded::parse(body) {
        let key = k.into_owned();
        let val = v.into_owned();
        if !SIGN_FIELDS.contains(&key.as_str()) {
            return Err(("isv.invalid-parameter", format!("unexpected field: {key}")));
        }
        if val.trim().is_empty() {
            return Err(("isv.missing-parameter", format!("empty field: {key}")));
        }
        if map.insert(key.clone(), val).is_some() {
            return Err(("isv.invalid-parameter", format!("duplicate field: {key}")));
        }
    }
    for req_f in SIGN_FIELDS {
        if !map.contains_key(req_f) {
            return Err(("isv.missing-parameter", format!("missing field: {req_f}")));
        }
    }
    Ok(map)
}

fn verify_sign_envelope(
    ctx: &FrontCtx,
    snap: &x_upt_store::dto::RuntimeSnapshotData,
    params: &BTreeMap<String, String>,
    secret: &str,
) -> Result<(), (&'static str, String)> {
    let charset = params.get("charset").map_or("", String::as_str);
    if !charset.eq_ignore_ascii_case("UTF-8") {
        return Err(("isv.invalid-parameter", "charset must be UTF-8".to_string()));
    }
    if params.get("version").map_or("", String::as_str) != "1.0" {
        return Err(("isv.invalid-parameter", "version must be 1.0".to_string()));
    }
    if params.get("sign_type").map_or("", String::as_str) != "HMAC-SHA256" {
        return Err((
            "isv.not-supported-sign-type",
            "sign_type must be HMAC-SHA256".to_string(),
        ));
    }

    let ts = params
        .get("timestamp")
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or((
            "isv.invalid-timestamp",
            "timestamp must be integer seconds".to_string(),
        ))?;
    let window = i64::try_from(snap.settings.sign_window_seconds).unwrap_or(300);
    if (Utc::now().timestamp() - ts).abs() > window {
        return Err((
            "isv.invalid-timestamp",
            "timestamp outside allowed window".to_string(),
        ));
    }

    let nonce = params.get("nonce").map_or("", String::as_str);
    if !(8..=64).contains(&nonce.len())
        || !nonce
            .bytes()
            .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    {
        return Err(("isv.invalid-nonce", "invalid nonce format".to_string()));
    }

    let canonical = params
        .iter()
        .filter(|(k, v)| k.as_str() != "sign" && !v.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let provided_sig = base64::engine::general_purpose::STANDARD
        .decode(params.get("sign").map_or("", String::as_str))
        .map_err(|_| ("isv.invalid-signature", "sign must be base64".to_string()))?;
    let expected = hmac_sha256_bytes(secret.as_bytes(), canonical.as_bytes());
    if !secrets_equal_bytes(&expected, &provided_sig) {
        return Err((
            "isv.invalid-signature",
            "signature verification failed".to_string(),
        ));
    }

    let app_id = params.get("app_id").map_or("", String::as_str);
    let nonce_key = format!("signgw:{app_id}:{nonce}");
    if !ctx.sec.sign_nonces.insert(&nonce_key) {
        return Err(("isv.invalid-nonce", "replayed nonce".to_string()));
    }
    Ok(())
}

fn build_sign_success(
    ctx: &FrontCtx,
    meta: ReqMeta,
    req_id: &str,
    client_id: &str,
    secret: &str,
    method: &str,
    timing: (u64, u64, serde_json::Value, serde_json::Value),
) -> Response {
    let (lat_ms, core_ms, biz_in, data) = timing;
    let mut inner = serde_json::Map::new();
    inner.insert(
        "code".to_string(),
        serde_json::Value::String("10000".to_string()),
    );
    inner.insert(
        "msg".to_string(),
        serde_json::Value::String("Success".to_string()),
    );
    inner.insert(
        "request_id".to_string(),
        serde_json::Value::String(req_id.to_string()),
    );
    if let Some(obj) = data.as_object() {
        for (k, v) in obj {
            inner.insert(k.clone(), v.clone());
        }
    }
    let inner_val = serde_json::Value::Object(inner);
    let compact = serde_json::to_string(&inner_val).unwrap_or_default();
    let resp_sig = base64::engine::general_purpose::STANDARD
        .encode(hmac_sha256_bytes(secret.as_bytes(), compact.as_bytes()));

    let resp_key = format!("{}_response", method.replace('.', "_"));
    let mut top = serde_json::Map::new();
    top.insert(resp_key, inner_val);
    top.insert("sign".to_string(), serde_json::Value::String(resp_sig));
    let out_body = serde_json::Value::Object(top);

    let mut resp = (StatusCode::OK, axum::Json(out_body.clone())).into_response();
    attach_std_headers(&mut resp, req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 200,
            code: Some(serde_json::json!("10000")),
            duration_core: Some(core_ms),
            duration_latency: Some(lat_ms),
            client_id: Some(client_id.to_string()),
            auth: Some("hmac_sha256".to_string()),
            rpc: Some(method.to_string()),
            input: biz_in,
            out: data,
            ..LogOutcome::default()
        },
    ));
    resp
}

fn sign_fail(
    ctx: &FrontCtx,
    meta: ReqMeta,
    req_id: &str,
    sub_code: &str,
    sub_msg: &str,
    client_id: Option<&str>,
) -> Response {
    let body = serde_json::json!({
        "error_response": {
            "code": "40004",
            "msg": "Business Failed",
            "sub_code": sub_code,
            "sub_msg": sub_msg,
            "request_id": req_id,
        },
        "sign": "",
    });
    let mut resp = (StatusCode::OK, axum::Json(body)).into_response();
    attach_std_headers(&mut resp, req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 200,
            code: Some(serde_json::json!(sub_code)),
            client_id: client_id.map(ToString::to_string),
            auth: Some("hmac_sha256".to_string()),
            error: Some(sub_msg.to_string()),
            error_kind: Some(sub_code.to_string()),
            ..LogOutcome::default()
        },
    ));
    resp
}

fn hmac_sha256_bytes(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        return Vec::new();
    };
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}
