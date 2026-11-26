use std::sync::Arc;

use axum::extract::{Extension, FromRequest, Multipart, Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::Deserialize;

use crate::auth::oauth::{issue_token, token_success};
use crate::auth::token::TokenManager;
use crate::auth::{AuthPolicy, authenticate_request, policy_from};
use crate::config::{AppConfig, apply_latency};
use crate::core::{Call, Core, dispatch_logged};
use crate::front::access::{AccessMeta, access_log};
use crate::front::util::{content_type, query_to_biz, rest_auth_error, rest_error, rest_short};
use crate::logger::context::LogContext;
use crate::service::apps::secrets_equal;
use crate::types::gateway::{METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH};
use crate::types::token::{AuthedApp, TokenForm};

#[derive(Clone)]
struct RestState {
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
        config.fronts.rest.common.auth.clone(),
        tokens.clone(),
        Arc::clone(&config.apps),
        config.fronts.rest.common.port,
    );
    let state = RestState {
        config: Arc::clone(&config),
        core,
        tokens,
        policy,
    };

    let mut app = Router::new();
    if state.policy.allows(crate::config::AuthMethod::OauthCc) {
        app = app.route("/oauth/token", post(handle_token));
    }
    if state.policy.allows(crate::config::AuthMethod::Gettoken) {
        app = app.route("/gettoken", get(handle_gettoken));
    }
    let authed = Router::new()
        .route(
            "/posts/search",
            get(handle_posts).post(handle_posts).head(handle_posts),
        )
        .route(
            "/photos/render",
            get(handle_photos).post(handle_photos).head(handle_photos),
        )
        .route(
            "/imgdef/compare",
            get(handle_imgdef).post(handle_imgdef).head(handle_imgdef),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_mw));

    app.merge(authed)
        .fallback(handle_not_found)
        .layer(middleware::from_fn(access_log))
        .layer(axum::Extension(meta))
        .with_state(state)
}

async fn auth_mw(
    State(state): State<RestState>,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let ctx = req.extensions().get::<LogContext>().cloned();
    let start = std::time::Instant::now();
    match authenticate_request(&state.policy, req, false).await {
        Ok((ok, mut req)) => {
            if let Some(ref ctx) = ctx {
                ctx.set_client_id(ok.app.client_id.clone());
                ctx.set_auth(ok.method.as_str());
                ctx.add_duration_auth(crate::logger::elapsed_ms(start));
            }
            req.extensions_mut().insert(ok.app);
            Ok(next.run(req).await)
        }
        Err(err) => {
            if let Some(ref ctx) = ctx {
                ctx.add_duration_auth(crate::logger::elapsed_ms(start));
            }
            Err(rest_auth_error(&ctx, &err))
        }
    }
}

async fn handle_not_found(Extension(ctx): Extension<LogContext>) -> Response {
    ctx.set_fail("not_found", "not found", serde_json::json!("not_found"));
    rest_error(
        StatusCode::NOT_FOUND,
        "not_found",
        "not found",
        &ctx.request_id(),
    )
}

async fn handle_token(
    Extension(ctx): Extension<LogContext>,
    State(state): State<RestState>,
    headers: HeaderMap,
    axum::Form(form): axum::Form<TokenForm>,
) -> Response {
    ctx.set_auth("oauth_cc");
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let Some(tokens) = &state.tokens else {
        ctx.set_fail(
            "unauthorized",
            "oauth is not enabled",
            serde_json::json!("unauthorized"),
        );
        return rest_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "oauth is not enabled",
            &ctx.request_id(),
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

#[derive(Deserialize)]
struct GetTokenQuery {
    app_id: Option<String>,
    secret: Option<String>,
}

async fn handle_gettoken(
    Extension(ctx): Extension<LogContext>,
    State(state): State<RestState>,
    Query(q): Query<GetTokenQuery>,
) -> Response {
    ctx.set_auth("gettoken");
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let Some(tokens) = &state.tokens else {
        ctx.set_fail(
            "unauthorized",
            "gettoken is not enabled",
            serde_json::json!("unauthorized"),
        );
        return rest_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "gettoken is not enabled",
            &ctx.request_id(),
        );
    };
    let Some(app_id) = q.app_id.filter(|s| !s.is_empty()) else {
        ctx.set_fail(
            "unauthorized",
            "app_id required",
            serde_json::json!("unauthorized"),
        );
        return rest_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "app_id required",
            &ctx.request_id(),
        );
    };
    let Some(secret) = q.secret.filter(|s| !s.is_empty()) else {
        ctx.set_fail(
            "unauthorized",
            "secret required",
            serde_json::json!("unauthorized"),
        );
        return rest_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "secret required",
            &ctx.request_id(),
        );
    };
    ctx.set_client_id(app_id.clone());
    ctx.set_in(serde_json::json!({ "app_id": app_id }));
    let Some(app) = state.config.apps.get(&app_id) else {
        ctx.set_fail(
            "unauthorized",
            "unknown client_id",
            serde_json::json!("unauthorized"),
        );
        return rest_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "unknown client_id",
            &ctx.request_id(),
        );
    };
    if !secrets_equal(&app.client_secret, &secret) {
        ctx.set_fail(
            "unauthorized",
            "invalid secret",
            serde_json::json!("unauthorized"),
        );
        return rest_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid secret",
            &ctx.request_id(),
        );
    }
    let methods = crate::config::granted_methods(&app.methods, state.config.methods);
    match tokens.create_token(app_id, methods).await {
        Ok(resp) => {
            ctx.set_out(serde_json::json!({ "expires_in": resp.expires_in }));
            Json(serde_json::json!({
                "access_token": resp.access_token,
                "expires_in": resp.expires_in,
            }))
            .into_response()
        }
        Err(_) => {
            ctx.set_fail(
                "bad_request",
                "too many tokens for this client",
                serde_json::json!("bad_request"),
            );
            rest_error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "too many tokens for this client",
                &ctx.request_id(),
            )
        }
    }
}

async fn handle_posts(
    Extension(ctx): Extension<LogContext>,
    Extension(app): Extension<AuthedApp>,
    State(state): State<RestState>,
    req: Request,
) -> Response {
    dispatch_http(&ctx, &app, &state, METHOD_POSTS_SEARCH, req, false).await
}

async fn handle_photos(
    Extension(ctx): Extension<LogContext>,
    Extension(app): Extension<AuthedApp>,
    State(state): State<RestState>,
    req: Request,
) -> Response {
    dispatch_http(&ctx, &app, &state, METHOD_PHOTOS_RENDER, req, false).await
}

async fn handle_imgdef(
    Extension(ctx): Extension<LogContext>,
    Extension(app): Extension<AuthedApp>,
    State(state): State<RestState>,
    req: Request,
) -> Response {
    dispatch_http(&ctx, &app, &state, METHOD_IMGDEF_COMPARE, req, true).await
}

async fn dispatch_http(
    ctx: &LogContext,
    app: &AuthedApp,
    state: &RestState,
    method: &str,
    req: Request,
    allow_multipart: bool,
) -> Response {
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    if req.method() == Method::HEAD {
        return StatusCode::OK.into_response();
    }
    let biz = match read_biz(req, allow_multipart).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let call = Call::from_app(app, method.to_string(), biz);
    match dispatch_logged(&state.core, ctx, call) {
        Ok(value) => Json(value).into_response(),
        Err(err) => {
            let (status, code) = rest_short(&err);
            ctx.set_code(serde_json::json!(code));
            rest_error(status, code, err.message(), &ctx.request_id())
        }
    }
}

async fn read_biz(req: Request, allow_multipart: bool) -> Result<serde_json::Value, Response> {
    let ctx = req.extensions().get::<LogContext>().cloned();
    let rid = ctx.as_ref().map_or(String::new(), LogContext::request_id);
    if req.method() == Method::GET {
        let q = req.uri().query().unwrap_or("");
        return Ok(query_to_biz(q));
    }
    let ct = content_type(req.headers());
    if allow_multipart && ct.starts_with("multipart/form-data") {
        return read_multipart(req, &ctx, &rid).await;
    }
    if ct.contains("json") {
        return read_json_body(req, &ctx, &rid).await;
    }
    if ct.starts_with("application/x-www-form-urlencoded") {
        return read_form_body(req, &ctx, &rid).await;
    }
    if let Some(ctx) = &ctx {
        ctx.set_fail(
            "unsupported_media_type",
            "unsupported media type",
            serde_json::json!("unsupported_media_type"),
        );
    }
    Err(rest_error(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
        "unsupported media type",
        &rid,
    ))
}

async fn read_json_body(
    req: Request,
    ctx: &Option<LogContext>,
    rid: &str,
) -> Result<serde_json::Value, Response> {
    let bytes = axum::body::to_bytes(req.into_body(), 32 * 1024 * 1024)
        .await
        .map_err(|e| {
            if let Some(ctx) = ctx {
                ctx.set_fail(
                    "bad_request",
                    "invalid body",
                    serde_json::json!("bad_request"),
                );
            }
            rest_error(StatusCode::BAD_REQUEST, "bad_request", &e.to_string(), rid)
        })?;
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) => Ok(v),
        Err(err) => {
            if let Some(ctx) = ctx {
                ctx.set_fail(
                    "bad_request",
                    "invalid json",
                    serde_json::json!("bad_request"),
                );
            }
            Err(rest_error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                &err.to_string(),
                rid,
            ))
        }
    }
}

async fn read_form_body(
    req: Request,
    ctx: &Option<LogContext>,
    rid: &str,
) -> Result<serde_json::Value, Response> {
    let bytes = axum::body::to_bytes(req.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|_| rest_error(StatusCode::BAD_REQUEST, "bad_request", "invalid body", rid))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        if let Some(ctx) = ctx {
            ctx.set_fail(
                "bad_request",
                "invalid form",
                serde_json::json!("bad_request"),
            );
        }
        rest_error(StatusCode::BAD_REQUEST, "bad_request", "invalid form", rid)
    })?;
    Ok(query_to_biz(text))
}

async fn read_multipart(
    req: Request,
    ctx: &Option<LogContext>,
    rid: &str,
) -> Result<serde_json::Value, Response> {
    match read_multipart_inner(req).await {
        Ok(v) => Ok(v),
        Err(msg) => {
            if let Some(ctx) = ctx {
                ctx.set_fail("bad_request", &msg, serde_json::json!("bad_request"));
            }
            Err(rest_error(
                StatusCode::BAD_REQUEST,
                "bad_request",
                &msg,
                rid,
            ))
        }
    }
}

async fn read_multipart_inner(req: Request) -> Result<serde_json::Value, String> {
    let mut mp = Multipart::from_request(req, &())
        .await
        .map_err(|e| e.body_text())?;
    let mut filename = None;
    let mut threshold = None;
    let mut image = None;
    while let Some(field) = mp.next_field().await.map_err(|e| e.body_text())? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "filename" => filename = Some(field.text().await.map_err(|e| e.body_text())?),
            "threshold" => {
                let t = field.text().await.map_err(|e| e.body_text())?;
                threshold = Some(
                    t.parse::<u32>()
                        .map_err(|_| "threshold must be an integer".to_string())?,
                );
            }
            "image" => {
                let bytes = field.bytes().await.map_err(|e| e.body_text())?;
                image = Some(base64::engine::general_purpose::STANDARD.encode(&bytes));
            }
            _ => {}
        }
    }
    Ok(serde_json::json!({
        "filename": filename.ok_or_else(|| "filename required".to_string())?,
        "threshold": threshold.ok_or_else(|| "threshold required".to_string())?,
        "image": image.ok_or_else(|| "image required".to_string())?,
    }))
}
