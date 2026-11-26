use std::sync::Arc;

use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

use crate::auth::oauth::{issue_token, token_success};
use crate::auth::token::TokenManager;
use crate::auth::{AuthInput, AuthPolicy, authenticate, policy_from};
use crate::config::{AppConfig, AuthMethod, apply_latency};
use crate::core::{Call, Core, dispatch_extra, dispatch_logged};
use crate::front::access::{AccessMeta, access_log};
use crate::front::util::jsonrpc_code;
use crate::logger::context::{ExtraCall, LogContext};
use crate::types::token::TokenForm;

#[derive(Clone)]
struct RpcState {
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Option<Arc<TokenManager>>,
    policy: AuthPolicy,
}

pub fn router(
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Option<Arc<TokenManager>>,
    meta: AccessMeta,
) -> Router {
    let policy = policy_from(
        config.fronts.jsonrpc.common.auth.clone(),
        tokens.clone(),
        Arc::clone(&config.apps),
        config.fronts.jsonrpc.common.port,
    );
    let state = RpcState {
        config: Arc::clone(&config),
        core,
        tokens,
        policy,
    };
    let mut app = Router::new().route("/rpc", post(handle_rpc));
    if state.policy.allows(AuthMethod::OauthCc) {
        app = app.route("/oauth/token", post(handle_token));
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
    Extension(ctx): Extension<LogContext>,
    State(state): State<RpcState>,
    headers: HeaderMap,
    axum::Form(form): axum::Form<TokenForm>,
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

async fn handle_rpc(
    Extension(ctx): Extension<LogContext>,
    State(state): State<RpcState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> Response {
    let text = std::str::from_utf8(&body).unwrap_or("");
    let parsed: Result<Value, _> = serde_json::from_str(text);
    let Ok(value) = parsed else {
        ctx.set_fail("parse_error", "parse error", serde_json::json!(-32700));
        return (
            StatusCode::BAD_REQUEST,
            Json(rpc_err(
                Value::Null,
                -32700,
                "Parse error",
                &ctx.request_id(),
            )),
        )
            .into_response();
    };
    let query = uri.query();
    if let Some(arr) = value.as_array() {
        ctx.set_in(serde_json::json!({ "batch": arr.len() }));
        ctx.set_out(Value::Null);
        ctx.clear_rpc();
        let mut out = Vec::new();
        for item in arr {
            if let Some(v) =
                call_one(&ctx, &state, &headers, query, &body, item.clone(), true).await
            {
                out.push(v);
            }
        }
        if out.is_empty() {
            return StatusCode::NO_CONTENT.into_response();
        }
        return Json(Value::Array(out)).into_response();
    }
    match call_one(&ctx, &state, &headers, query, &body, value, false).await {
        Some(v) => Json(v).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

fn maybe_respond(notification: bool, value: Value) -> Option<Value> {
    if notification { None } else { Some(value) }
}

async fn call_one(
    ctx: &LogContext,
    state: &RpcState,
    headers: &HeaderMap,
    query: Option<&str>,
    body: &[u8],
    value: Value,
    extra: bool,
) -> Option<Value> {
    let rid = ctx.request_id();
    let notification = value.as_object().is_some_and(|obj| !obj.contains_key("id"));
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return maybe_respond(
            notification,
            rpc_err(id_of(&value), -32600, "Invalid Request", &rid),
        );
    }
    let id = value.get("id").cloned();
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    if method.is_empty() {
        return maybe_respond(
            notification,
            rpc_err(id.unwrap_or(Value::Null), -32600, "Invalid Request", &rid),
        );
    }
    if params.is_array() {
        return maybe_respond(
            notification,
            rpc_err(
                id.unwrap_or(Value::Null),
                -32602,
                "params must be an object",
                &rid,
            ),
        );
    }

    let input = AuthInput {
        headers,
        query,
        method: "POST",
        path: "/rpc",
        body,
        query_overrides_header: false,
    };
    let start_auth = std::time::Instant::now();
    let app = match authenticate(&state.policy, &input).await {
        Ok(ok) => {
            ctx.set_client_id(ok.app.client_id.clone());
            ctx.set_auth(ok.method.as_str());
            ctx.add_duration_auth(crate::logger::elapsed_ms(start_auth));
            ok.app
        }
        Err(_) => {
            ctx.add_duration_auth(crate::logger::elapsed_ms(start_auth));
            if extra {
                ctx.push_extra(ExtraCall {
                    rpc: Some(method),
                    input: params,
                    output: Value::Null,
                    error: Some("authentication failed".to_string()),
                    error_kind: Some("unauthorized".to_string()),
                    code: Some(serde_json::json!(-32001)),
                    job_id: None,
                    duration_core: None,
                });
            } else {
                ctx.set_fail(
                    "unauthorized",
                    "authentication failed",
                    serde_json::json!(-32001),
                );
            }
            return maybe_respond(
                notification,
                rpc_err(
                    id.unwrap_or(Value::Null),
                    -32001,
                    "authentication failed",
                    &rid,
                ),
            );
        }
    };

    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let call = Call::from_app(&app, method, params);
    maybe_respond(
        notification,
        dispatch_rpc(ctx, state, id, call, extra).await,
    )
}

async fn dispatch_rpc(
    ctx: &LogContext,
    state: &RpcState,
    id: Option<Value>,
    call: Call,
    extra: bool,
) -> Value {
    let id = id.unwrap_or(Value::Null);
    let rid = ctx.request_id();
    let dispatched = if extra {
        dispatch_extra(&state.core, ctx, call)
    } else {
        dispatch_logged(&state.core, ctx, call)
    };
    match dispatched {
        Ok(result) => serde_json::json!({"jsonrpc":"2.0","result": result, "id": id}),
        Err(err) => {
            let code = jsonrpc_code(&err);
            if extra {
                ctx.set_last_extra_code(serde_json::json!(code));
            } else {
                ctx.set_code(serde_json::json!(code));
            }
            rpc_err(id, code, err.message(), &rid)
        }
    }
}

fn id_of(value: &Value) -> Value {
    value.get("id").cloned().unwrap_or(Value::Null)
}

fn rpc_err(id: Value, code: i64, message: &str, request_id: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "error": {"code": code, "message": message, "data": {"request_id": request_id}},
        "id": id,
    })
}
