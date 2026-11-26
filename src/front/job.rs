use std::sync::Arc;

use axum::extract::{Extension, Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::auth::oauth::{issue_token, token_success};
use crate::auth::token::TokenManager;
use crate::auth::{AuthPolicy, authenticate_request, policy_from};
use crate::config::{AppConfig, AuthMethod, apply_latency};
use crate::front::access::{AccessMeta, access_log};
use crate::front::util::{RestJson, job_call_error, rest_auth_error, rest_error};
use crate::jobstore::{JobLogMeta, JobStore};
use crate::logger::context::LogContext;
use crate::types::token::{AuthedApp, TokenForm};

#[derive(Clone)]
struct JobState {
    config: Arc<AppConfig>,
    tokens: Option<Arc<TokenManager>>,
    jobs: Arc<JobStore>,
    policy: AuthPolicy,
}

pub fn router(
    config: Arc<AppConfig>,
    tokens: Option<Arc<TokenManager>>,
    jobs: Arc<JobStore>,
    meta: AccessMeta,
) -> Router {
    let policy = policy_from(
        config.fronts.job.common.auth.clone(),
        tokens.clone(),
        Arc::clone(&config.apps),
        config.fronts.job.common.port,
    );
    let oauth = policy.allows(AuthMethod::OauthCc);
    let state = JobState {
        config,
        tokens,
        jobs,
        policy,
    };
    let authed = Router::new()
        .route("/jobs", post(handle_submit))
        .route("/jobs/{id}", get(handle_get))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_mw));
    let mut app = Router::new();
    if oauth {
        app = app.route("/oauth/token", post(handle_token));
    }
    app.merge(authed)
        .fallback(handle_not_found)
        .layer(middleware::from_fn(access_log))
        .layer(axum::Extension(meta))
        .with_state(state)
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

async fn auth_mw(
    State(state): State<JobState>,
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

async fn handle_token(
    Extension(ctx): Extension<LogContext>,
    State(state): State<JobState>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobBody {
    method: String,
    biz_content: serde_json::Value,
}

async fn handle_submit(
    Extension(ctx): Extension<LogContext>,
    Extension(app): Extension<AuthedApp>,
    State(state): State<JobState>,
    RestJson(body): RestJson<JobBody>,
) -> Response {
    if let Some((status, code, msg)) =
        job_call_error(&app, state.config.methods, &body.method, &body.biz_content)
    {
        ctx.set_fail(code, &msg, serde_json::json!(code));
        return rest_error(status, code, &msg, &ctx.request_id());
    }
    ctx.set_rpc(&body.method);
    ctx.set_in(body.biz_content.clone());
    let snap = ctx.snapshot();
    let id = state
        .jobs
        .submit(
            app,
            body.method,
            body.biz_content,
            ctx.request_id(),
            JobLogMeta {
                ip: snap.ip,
                query: snap.query,
                user_agent: snap.user_agent,
                referer: snap.referer,
                content_type: snap.content_type,
                auth: snap.auth,
            },
        )
        .await;
    ctx.set_job_id(&id);
    ctx.set_out(serde_json::json!({"job_id": id, "status": "pending"}));
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"job_id": id, "status": "pending"})),
    )
        .into_response()
}

async fn handle_get(
    Extension(ctx): Extension<LogContext>,
    Extension(app): Extension<AuthedApp>,
    State(state): State<JobState>,
    Path(id): Path<String>,
) -> Response {
    match state.jobs.get(&id, &app.client_id).await {
        Some(view) => {
            ctx.set_job_id(&view.job_id);
            let body = serde_json::json!({
                "job_id": view.job_id,
                "status": view.status,
                "result": view.result,
                "error": view.error,
                "message": view.message,
            });
            ctx.set_out(body.clone());
            Json(body).into_response()
        }
        None => {
            ctx.set_fail("not_found", "job not found", serde_json::json!("not_found"));
            rest_error(
                StatusCode::NOT_FOUND,
                "not_found",
                "job not found",
                &ctx.request_id(),
            )
        }
    }
}
