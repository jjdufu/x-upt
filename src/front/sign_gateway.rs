use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::rejection::FormRejection;
use axum::extract::{Extension, Form, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;

use crate::auth::nonce::NonceStore;
use crate::auth::sign::{sign_hmac, sign_source, verify_hmac};
use crate::config::{AppConfig, apply_latency};
use crate::core::{Call, Core, CoreErrorKind, dispatch_logged};
use crate::front::access::{AccessMeta, access_log};
use crate::logger::context::LogContext;
use crate::service::apps::AppRegistry;
use crate::types::token::AuthedApp;

#[derive(Clone)]
struct SignState {
    apps: Arc<AppRegistry>,
    core: Arc<Core>,
    config: Arc<AppConfig>,
    nonces: Arc<NonceStore>,
}

pub fn router(config: Arc<AppConfig>, core: Arc<Core>, meta: AccessMeta) -> Router {
    let state = SignState {
        apps: Arc::clone(&config.apps),
        core,
        nonces: Arc::new(NonceStore::new(config.sign.nonce_ttl_seconds)),
        config,
    };
    Router::new()
        .route("/gateway.do", post(handle_gateway))
        .fallback(http_404)
        .layer(middleware::from_fn(access_log))
        .layer(axum::Extension(meta))
        .with_state(state)
}

async fn http_404(Extension(ctx): Extension<LogContext>) -> Response {
    ctx.set_fail("not_found", "not found", serde_json::json!("not_found"));
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn handle_gateway(
    Extension(ctx): Extension<LogContext>,
    State(state): State<SignState>,
    form: Result<Form<HashMap<String, String>>, FormRejection>,
) -> Response {
    ctx.set_auth("hmac");
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let request_id = ctx.request_id();
    let Form(fields) = match form {
        Ok(form) => form,
        Err(_) => {
            return sign_fail(&ctx, &request_id, "isv.missing-parameter", "invalid form");
        }
    };
    let known = [
        "app_id",
        "method",
        "charset",
        "version",
        "timestamp",
        "nonce",
        "sign_type",
        "sign",
        "biz_content",
    ];
    for key in fields.keys() {
        if !known.contains(&key.as_str()) {
            return sign_fail(&ctx, &request_id, "isv.missing-parameter", "unknown field");
        }
    }
    for key in known {
        if fields.get(key).is_none_or(String::is_empty) {
            return sign_fail(
                &ctx,
                &request_id,
                "isv.missing-parameter",
                "missing parameter",
            );
        }
    }

    let charset = fields.get("charset").map(String::as_str).unwrap_or("");
    if !charset.eq_ignore_ascii_case("utf-8")
        || fields.get("version").map(String::as_str) != Some("1.0")
    {
        return sign_fail(
            &ctx,
            &request_id,
            "isv.missing-parameter",
            "invalid charset or version",
        );
    }

    let timestamp = fields.get("timestamp").and_then(|s| s.parse::<i64>().ok());
    let Some(ts) = timestamp else {
        return sign_fail(
            &ctx,
            &request_id,
            "isv.invalid-timestamp",
            "invalid timestamp",
        );
    };
    let now = Utc::now().timestamp();
    let window = i64::try_from(state.config.sign.window_seconds).unwrap_or(i64::MAX);
    if (now - ts).abs() > window {
        return sign_fail(
            &ctx,
            &request_id,
            "isv.invalid-timestamp",
            "timestamp out of window",
        );
    }

    let nonce = fields["nonce"].as_str();
    if !valid_nonce(nonce) || state.nonces.contains(nonce) {
        return sign_fail(&ctx, &request_id, "isv.invalid-nonce", "invalid nonce");
    }

    let app_id = fields["app_id"].as_str();
    ctx.set_client_id(app_id.to_string());
    let Some(app) = state.apps.get(app_id) else {
        return sign_fail(&ctx, &request_id, "isv.invalid-app-id", "invalid app_id");
    };

    if fields["sign_type"].as_str() != "HMAC-SHA256" {
        return sign_fail(
            &ctx,
            &request_id,
            "isv.not-supported-sign-type",
            "sign_type not supported",
        );
    }

    let source = sign_source(&fields);
    if !verify_hmac(&app.client_secret, &source, &fields["sign"]) {
        return sign_fail(
            &ctx,
            &request_id,
            "isv.invalid-signature",
            "invalid signature",
        );
    }
    if !state.nonces.insert(nonce) {
        return sign_fail(&ctx, &request_id, "isv.invalid-nonce", "invalid nonce");
    }

    let method = fields["method"].clone();
    ctx.set_rpc(&method);
    let biz: serde_json::Value = match serde_json::from_str(&fields["biz_content"]) {
        Ok(v) => v,
        Err(_) => {
            return sign_fail(
                &ctx,
                &request_id,
                "isv.invalid-parameter",
                "invalid biz_content",
            );
        }
    };

    let authed = AuthedApp::from(app);
    let call = Call::from_app(&authed, method.clone(), biz);
    match dispatch_logged(&state.core, &ctx, call) {
        Ok(value) => {
            let key = format!("{}_response", method.replace('.', "_"));
            let mut obj = serde_json::Map::new();
            ctx.set_code(serde_json::json!("10000"));
            obj.insert("code".into(), serde_json::json!("10000"));
            obj.insert("msg".into(), serde_json::json!("Success"));
            obj.insert("request_id".into(), serde_json::json!(request_id));
            if let Some(map) = value.as_object() {
                for (k, v) in map {
                    obj.insert(k.clone(), v.clone());
                }
            }
            let inner = serde_json::Value::Object(obj);
            let sig = sign_hmac(&app.client_secret, &inner);
            Json(serde_json::json!({ key: inner, "sign": sig })).into_response()
        }
        Err(err) => {
            let (sub, msg) = match err.kind {
                CoreErrorKind::UnknownMethod => ("isv.invalid-method", err.message().to_string()),
                CoreErrorKind::Forbidden => (
                    "isv.insufficient-isv-permissions",
                    err.message().to_string(),
                ),
                CoreErrorKind::BadRequest(_) | CoreErrorKind::Unprocessable(_) => {
                    ("isv.invalid-parameter", err.message().to_string())
                }
                CoreErrorKind::NotFound(_) => ("isv.resource-not-exist", err.message().to_string()),
                CoreErrorKind::Internal(_) => ("isp.unknow-error", err.message().to_string()),
            };
            ctx.set_fail(sub, &msg, serde_json::json!(sub));
            ctx.set_rpc(&method);
            Json(sign_error_json(&request_id, sub, &msg)).into_response()
        }
    }
}

fn valid_nonce(nonce: &str) -> bool {
    let len = nonce.len();
    (8..=64).contains(&len)
        && nonce
            .bytes()
            .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

fn sign_fail(ctx: &LogContext, request_id: &str, sub_code: &str, sub_msg: &str) -> Response {
    ctx.set_fail(sub_code, sub_msg, serde_json::json!(sub_code));
    Json(sign_error_json(request_id, sub_code, sub_msg)).into_response()
}

fn sign_error_json(request_id: &str, sub_code: &str, sub_msg: &str) -> serde_json::Value {
    let inner = serde_json::json!({
        "code": "40004",
        "msg": "Business Failed",
        "sub_code": sub_code,
        "sub_msg": sub_msg,
        "request_id": request_id,
    });
    serde_json::json!({
        "error_response": inner,
        "sign": "",
    })
}
