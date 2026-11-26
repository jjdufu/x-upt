use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::any;

use crate::auth::nonce::NonceStore;
use crate::auth::token::TokenManager;
use crate::config::{AppConfig, FrontName};
use crate::envelope::pipeline::{self, PipeOut};
use crate::envelope::profile::Profile;
use crate::envelope::slots::{BODY_LIMIT, Incoming};
use crate::envelope::token_store::EnvelopeTokenStore;
use crate::front::access::{AccessMeta, access_log};
use crate::logger::context::LogContext;

#[derive(Clone)]
struct FrontState {
    front: FrontName,
    profiles: Vec<Profile>,
    config: Arc<AppConfig>,
    tokens: Option<Arc<EnvelopeTokenStore>>,
    nonces: Arc<NonceStore>,
    hub_bucket: Option<Arc<TokenManager>>,
}

pub fn router(
    config: Arc<AppConfig>,
    front: FrontName,
    profiles: Vec<Profile>,
    hub_bucket: Option<Arc<TokenManager>>,
    meta: AccessMeta,
) -> Router {
    let tokens = hub_bucket.as_ref().map(|_| {
        Arc::new(EnvelopeTokenStore::new(
            config.token.expire_minutes,
            config.token.cleanup_interval_seconds,
        ))
    });
    let state = FrontState {
        front,
        profiles,
        nonces: Arc::new(NonceStore::new(config.sign.nonce_ttl_seconds)),
        tokens,
        hub_bucket,
        config,
    };
    Router::new()
        .fallback(any(handle))
        .layer(middleware::from_fn(access_log))
        .layer(axum::Extension(meta))
        .with_state(state)
}

async fn handle(
    axum::extract::State(state): axum::extract::State<FrontState>,
    axum::extract::Extension(ctx): axum::extract::Extension<LogContext>,
    request: Request,
) -> Response {
    if state.front_needs_bucket() && state.hub_bucket.is_none() {
        ctx.set_auth(Profile::auth_label(state.front));
        ctx.set_error("missing token bucket");
        return (StatusCode::INTERNAL_SERVER_ERROR, "missing token bucket").into_response();
    }
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or("").to_string();
    let headers = request
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_string(), text.to_string()))
        })
        .collect();
    let body = match axum::body::to_bytes(request.into_body(), BODY_LIMIT + 1).await {
        Ok(bytes) => bytes.to_vec(),
        Err(_) => vec![0; BODY_LIMIT + 1],
    };
    let incoming = Incoming {
        method,
        path,
        query,
        headers,
        body,
    };
    let out = pipeline::drive(
        state.front,
        &state.profiles,
        state.tokens.as_deref(),
        &state.nonces,
        &state.config.sign,
        &state.config.latency,
        &incoming,
    )
    .await;
    write_log(&ctx, &out);
    response_from(out)
}

impl FrontState {
    fn front_needs_bucket(&self) -> bool {
        matches!(
            self.front,
            FrontName::EnvelopeTokenPlain | FrontName::EnvelopeTokenCrypt
        )
    }
}

fn write_log(ctx: &LogContext, out: &PipeOut) {
    ctx.set_auth(out.auth);
    ctx.set_in(out.log_in.clone());
    ctx.set_out(out.log_out.clone());
    ctx.add_duration_latency(out.latency_ms);
    if let Some(error) = &out.error {
        ctx.set_error(error.clone());
    }
}

fn response_from(out: PipeOut) -> Response {
    let status = StatusCode::from_u16(out.status).unwrap_or(StatusCode::BAD_REQUEST);
    let mut response = Response::builder()
        .status(status)
        .body(Body::from(out.body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    if let Ok(value) = HeaderValue::from_str(out.content_type) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    response
}
