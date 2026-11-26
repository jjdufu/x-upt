use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Form, Json, Router};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::auth::oauth::{issue_token, token_success};
use crate::auth::token::TokenManager;
use crate::auth::{AuthPolicy, authenticate_request, policy_from};
use crate::config::{AppConfig, apply_latency};
use crate::core::{Call, Core, dispatch_logged};
use crate::front::access::{AccessMeta, access_log};
use crate::front::util::{envelope_auth_error, envelope_err, envelope_ok};
use crate::logger::context::LogContext;
use crate::service::apps::AppRegistry;
use crate::types::envelope::{CODE_BAD_REQUEST, CODE_NOT_FOUND, CODE_OK, CODE_UNPROCESSABLE};
use crate::types::token::{AuthedApp, TokenForm};

#[derive(Clone)]
struct OauthState {
    apps: Arc<AppRegistry>,
    tokens: Arc<TokenManager>,
    core: Arc<Core>,
    config: Arc<AppConfig>,
    policy: AuthPolicy,
}

pub fn router(
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Arc<TokenManager>,
    meta: AccessMeta,
) -> Router {
    let policy = policy_from(
        config.fronts.oauth_gateway.common.auth.clone(),
        Some(Arc::clone(&tokens)),
        Arc::clone(&config.apps),
        config.fronts.oauth_gateway.common.port,
    );
    let state = OauthState {
        apps: Arc::clone(&config.apps),
        tokens,
        core,
        config,
        policy,
    };

    let gateway = Router::new()
        .route("/v3/gateway", post(handle_gateway))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .route("/v3/oauth/token", post(handle_token))
        .merge(gateway)
        .fallback(handle_not_found)
        .layer(middleware::from_fn(access_log))
        .layer(axum::Extension(meta))
        .with_state(state)
}

async fn handle_token(
    Extension(ctx): Extension<LogContext>,
    State(state): State<OauthState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    ctx.set_auth("oauth_cc");
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    match issue_token(
        &ctx,
        &state.apps,
        &state.tokens,
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

async fn auth_middleware(
    State(state): State<OauthState>,
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
            let mut response = envelope_auth_error(&ctx, &err);
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static(r#"Bearer realm="upupt""#),
            );
            Err(response)
        }
    }
}

async fn handle_gateway(
    Extension(ctx): Extension<LogContext>,
    Extension(app): Extension<AuthedApp>,
    State(state): State<OauthState>,
    EnvelopeJson(body): EnvelopeJson<GatewayBody>,
) -> Response {
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let request_id = ctx.request_id();
    let call = Call::from_app(&app, body.method, body.biz_content);
    match dispatch_logged(&state.core, &ctx, call) {
        Ok(value) => {
            ctx.set_code(serde_json::json!(CODE_OK));
            envelope_ok(request_id, value).into_response()
        }
        Err(err) => {
            let (status, code) = crate::front::util::core_to_envelope(&err);
            ctx.set_code(serde_json::json!(code));
            envelope_err(status, code, err.message(), &request_id)
        }
    }
}

async fn handle_not_found(
    uri: axum::http::Uri,
    Extension(ctx): Extension<LogContext>,
) -> impl IntoResponse {
    ctx.set_fail("not_found", "not found", serde_json::json!(CODE_NOT_FOUND));
    ctx.set_in(serde_json::json!({ "uri": uri.path() }));
    envelope_err(
        StatusCode::NOT_FOUND,
        CODE_NOT_FOUND,
        "not found",
        &ctx.request_id(),
    )
}

struct EnvelopeJson<T>(T);

struct EnvelopeJsonRejection {
    status: StatusCode,
    code: i32,
    message: String,
    request_id: String,
}

impl IntoResponse for EnvelopeJsonRejection {
    fn into_response(self) -> Response {
        envelope_err(self.status, self.code, &self.message, &self.request_id)
    }
}

impl<S, T> FromRequest<S> for EnvelopeJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = EnvelopeJsonRejection;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let ctx = req.extensions().get::<LogContext>().cloned();
        let request_id = ctx.as_ref().map(LogContext::request_id).unwrap_or_default();
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => {
                let message = json_rejection_message(&rejection);
                if let Some(ctx) = &ctx {
                    ctx.set_fail(
                        if rejection.status() == StatusCode::UNPROCESSABLE_ENTITY {
                            "unprocessable"
                        } else {
                            "bad_request"
                        },
                        &message,
                        serde_json::json!(
                            if rejection.status() == StatusCode::UNPROCESSABLE_ENTITY {
                                CODE_UNPROCESSABLE
                            } else {
                                CODE_BAD_REQUEST
                            }
                        ),
                    );
                }
                let status = rejection.status();
                let code = if status == StatusCode::UNPROCESSABLE_ENTITY {
                    CODE_UNPROCESSABLE
                } else {
                    CODE_BAD_REQUEST
                };
                Err(EnvelopeJsonRejection {
                    status,
                    code,
                    message,
                    request_id,
                })
            }
        }
    }
}

fn json_rejection_message(rejection: &JsonRejection) -> String {
    rejection.body_text()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GatewayBody {
    method: String,
    biz_content: serde_json::Value,
}
