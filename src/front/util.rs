use axum::Json;
use axum::extract::{FromRequest, Request};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use crate::auth::AuthError;
use crate::config::MethodsConfig;
use crate::core::{CoreError, CoreErrorKind};
use crate::logger::context::LogContext;
use crate::types::envelope::{
    CODE_BAD_REQUEST, CODE_FORBIDDEN, CODE_INTERNAL, CODE_INVALID_TOKEN, CODE_NOT_FOUND, CODE_OK,
    CODE_UNAUTHORIZED, CODE_UNKNOWN_METHOD, CODE_UNPROCESSABLE, Envelope, MSG_OK,
};
use crate::types::gateway::is_known_method;
use crate::types::token::AuthedApp;

pub struct RestJson<T>(pub T);

impl<S, T> FromRequest<S> for RestJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = req.extensions().get::<LogContext>().cloned();
        let request_id = ctx.as_ref().map_or(String::new(), LogContext::request_id);
        let ct = content_type(req.headers());
        if !ct.contains("json") {
            if let Some(ctx) = &ctx {
                ctx.set_fail(
                    "unsupported_media_type",
                    "unsupported media type",
                    serde_json::json!("unsupported_media_type"),
                );
            }
            return Err(rest_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "unsupported media type",
                &request_id,
            ));
        }
        let bytes = axum::body::to_bytes(req.into_body(), 32 * 1024 * 1024)
            .await
            .map_err(|e| {
                if let Some(ctx) = &ctx {
                    ctx.set_fail(
                        "bad_request",
                        "invalid body",
                        serde_json::json!("bad_request"),
                    );
                }
                rest_error(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    &e.to_string(),
                    &request_id,
                )
            })?;
        match serde_json::from_slice::<T>(&bytes) {
            Ok(value) => Ok(Self(value)),
            Err(err) => {
                let message = err.to_string();
                if let Some(ctx) = &ctx {
                    ctx.set_fail(
                        "unprocessable",
                        &message,
                        serde_json::json!("unprocessable"),
                    );
                }
                Err(rest_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "unprocessable",
                    &message,
                    &request_id,
                ))
            }
        }
    }
}

#[must_use]
pub fn content_type(headers: &HeaderMap) -> String {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub fn envelope_ok(
    request_id: String,
    data: serde_json::Value,
) -> Json<Envelope<serde_json::Value>> {
    Json(Envelope {
        code: CODE_OK,
        msg: MSG_OK.to_string(),
        request_id,
        data,
    })
}

#[must_use]
pub fn envelope_err(status: StatusCode, code: i32, message: &str, request_id: &str) -> Response {
    (
        status,
        Json(Envelope {
            code,
            msg: message.to_string(),
            request_id: request_id.to_string(),
            data: serde_json::Value::Null,
        }),
    )
        .into_response()
}

#[must_use]
pub fn core_to_envelope(err: &CoreError) -> (StatusCode, i32) {
    match &err.kind {
        CoreErrorKind::BadRequest(_) => (StatusCode::BAD_REQUEST, CODE_BAD_REQUEST),
        CoreErrorKind::UnknownMethod => (StatusCode::BAD_REQUEST, CODE_UNKNOWN_METHOD),
        CoreErrorKind::Forbidden => (StatusCode::FORBIDDEN, CODE_FORBIDDEN),
        CoreErrorKind::Unprocessable(_) => (StatusCode::UNPROCESSABLE_ENTITY, CODE_UNPROCESSABLE),
        CoreErrorKind::NotFound(_) => (StatusCode::NOT_FOUND, CODE_NOT_FOUND),
        CoreErrorKind::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, CODE_INTERNAL),
    }
}

#[must_use]
pub fn auth_to_envelope(err: &AuthError) -> (StatusCode, i32, &'static str) {
    match err {
        AuthError::Missing | AuthError::DigestChallenge(_) => (
            StatusCode::UNAUTHORIZED,
            CODE_UNAUTHORIZED,
            "Missing authorization token",
        ),
        AuthError::InvalidFormat => (
            StatusCode::UNAUTHORIZED,
            CODE_UNAUTHORIZED,
            "Invalid authorization header format",
        ),
        AuthError::InvalidToken => (
            StatusCode::UNAUTHORIZED,
            CODE_INVALID_TOKEN,
            "Invalid or expired token",
        ),
    }
}

#[must_use]
pub fn rest_short(err: &CoreError) -> (StatusCode, &'static str) {
    match &err.kind {
        CoreErrorKind::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
        CoreErrorKind::UnknownMethod => (StatusCode::BAD_REQUEST, "unknown_method"),
        CoreErrorKind::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
        CoreErrorKind::Unprocessable(_) => (StatusCode::UNPROCESSABLE_ENTITY, "unprocessable"),
        CoreErrorKind::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        CoreErrorKind::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

#[must_use]
pub fn auth_kind(err: &AuthError) -> &'static str {
    match err {
        AuthError::Missing | AuthError::DigestChallenge(_) => "unauthorized",
        AuthError::InvalidFormat => "invalid_format",
        AuthError::InvalidToken => "invalid_token",
    }
}

#[must_use]
pub fn auth_to_rest(err: &AuthError) -> (StatusCode, &'static str, &'static str) {
    match err {
        AuthError::Missing | AuthError::DigestChallenge(_) => (
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing authorization token",
        ),
        AuthError::InvalidFormat => (
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid authorization header format",
        ),
        AuthError::InvalidToken => (
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "Invalid or expired token",
        ),
    }
}

#[must_use]
pub fn rest_error(status: StatusCode, error: &str, message: &str, request_id: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": error,
            "message": message,
            "request_id": request_id,
        })),
    )
        .into_response()
}

#[must_use]
pub fn rest_auth_error(ctx: &Option<LogContext>, err: &AuthError) -> Response {
    let (status, code, message) = auth_to_rest(err);
    if let Some(ctx) = ctx {
        ctx.set_fail(auth_kind(err), message, serde_json::json!(code));
    }
    let rid = ctx.as_ref().map_or(String::new(), LogContext::request_id);
    let mut response = rest_error(status, code, message, &rid);
    if let AuthError::DigestChallenge(challenges) = err {
        crate::auth::attach_digest_headers(response.headers_mut(), challenges);
    }
    response
}

#[must_use]
pub fn envelope_auth_error(ctx: &Option<LogContext>, err: &AuthError) -> Response {
    let (status, code, message) = auth_to_envelope(err);
    if let Some(ctx) = ctx {
        ctx.set_fail(auth_kind(err), message, serde_json::json!(code));
    }
    let request_id = ctx.as_ref().map_or(String::new(), LogContext::request_id);
    let mut response = envelope_err(status, code, message, &request_id);
    if let AuthError::DigestChallenge(challenges) = err {
        crate::auth::attach_digest_headers(response.headers_mut(), challenges);
    }
    response
}

#[must_use]
pub fn job_call_error(
    app: &AuthedApp,
    methods: MethodsConfig,
    method: &str,
    biz: &serde_json::Value,
) -> Option<(StatusCode, &'static str, String)> {
    let method = method.trim();
    if method.is_empty() {
        return Some((
            StatusCode::BAD_REQUEST,
            "bad_request",
            "method must be non-empty".to_string(),
        ));
    }
    if !is_known_method(method) || !methods.is_enabled(method) {
        return Some((
            StatusCode::BAD_REQUEST,
            "unknown_method",
            "unknown method".to_string(),
        ));
    }
    if !app.methods.contains(method) {
        return Some((
            StatusCode::FORBIDDEN,
            "forbidden",
            "method is not granted to this client".to_string(),
        ));
    }
    if !biz.is_object() {
        return Some((
            StatusCode::BAD_REQUEST,
            "bad_request",
            "biz_content must be an object".to_string(),
        ));
    }
    None
}

#[must_use]
pub fn jsonrpc_code(err: &CoreError) -> i64 {
    match &err.kind {
        CoreErrorKind::UnknownMethod => -32601,
        CoreErrorKind::BadRequest(_) | CoreErrorKind::Unprocessable(_) => -32602,
        CoreErrorKind::Forbidden => -32003,
        CoreErrorKind::NotFound(_) => -32004,
        CoreErrorKind::Internal(_) => -32603,
    }
}

const INT_KEYS: &[&str] = &[
    "user_id",
    "page",
    "page_size",
    "width",
    "height",
    "threshold",
];
const AUTH_QUERY_KEYS: &[&str] = &["api_key", "x-api-key", "access_token", "authorization"];

#[must_use]
pub fn pairs_to_biz(pairs: impl IntoIterator<Item = (String, String)>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (key, raw) in pairs {
        if AUTH_QUERY_KEYS.iter().any(|k| key.eq_ignore_ascii_case(k)) {
            continue;
        }
        map.insert(key.clone(), coerce_field(&key, &raw));
    }
    serde_json::Value::Object(map)
}

#[must_use]
pub fn query_to_biz(query: &str) -> serde_json::Value {
    pairs_to_biz(
        url::form_urlencoded::parse(query.as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned())),
    )
}

fn coerce_field(key: &str, raw: &str) -> serde_json::Value {
    if INT_KEYS.contains(&key)
        && let Ok(n) = raw.parse::<u64>()
    {
        return serde_json::Value::from(n);
    }
    serde_json::Value::String(raw.to_string())
}
