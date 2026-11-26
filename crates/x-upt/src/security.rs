//! Origin/Host CSRF & DNS-rebinding guard and optional management token authentication.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, Method, header};
use serde::{Deserialize, Serialize};
use x_upt_core::domain::secrets_equal;
use x_upt_core::error::AppError;

use crate::state::{AdminState, ApiError};

const SESSION_COOKIE: &str = "x_upt_session";

/// Response payload for `GET /api/v1/auth/session`.
#[derive(Debug, Serialize)]
pub struct AuthSessionDto {
    /// Whether `--admin-token` / `X_UPT_ADMIN_TOKEN` is configured.
    pub auth_required: bool,
    /// Whether the current request holds valid credentials.
    pub authenticated: bool,
}

/// Request payload for `POST /api/v1/auth/login`.
#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    /// Management token string.
    pub token: String,
}

/// Middleware enforcing valid `Host` header and matching `Origin` on mutating requests.
pub async fn origin_and_host_guard(req: Request, next: Next) -> Response {
    let headers = req.headers();
    let Some(host_raw) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return ApiError(AppError::Forbidden("missing Host header".to_string())).into_response();
    };
    let host_trimmed = host_raw.trim();
    if host_trimmed.is_empty() || host_trimmed.parse::<http::uri::Authority>().is_err() {
        return ApiError(AppError::Forbidden("invalid Host header".to_string())).into_response();
    }

    if matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    ) && let Some(origin_raw) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
    {
        let Ok(uri) = origin_raw.parse::<http::Uri>() else {
            return ApiError(AppError::Forbidden("invalid Origin header".to_string()))
                .into_response();
        };
        let Some(origin_auth) = uri.authority() else {
            return ApiError(AppError::Forbidden("missing Origin authority".to_string()))
                .into_response();
        };
        if !origin_auth.as_str().eq_ignore_ascii_case(host_trimmed) {
            return ApiError(AppError::Forbidden(
                "cross-origin mutation rejected".to_string(),
            ))
            .into_response();
        }
    }

    next.run(req).await
}

/// Middleware enforcing `--admin-token` on protected `/api/v1/*` routes when configured.
pub async fn admin_auth_guard(
    State(state): State<AdminState>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let is_protected = path.starts_with("/api/")
        && path != "/api/v1/auth/session"
        && path != "/api/v1/auth/login"
        && path != "/api/v1/auth/logout";

    if is_protected && !is_request_authenticated(&state, req.headers()) {
        return ApiError(AppError::Unauthorized("需要有效的管理口令".to_string())).into_response();
    }
    next.run(req).await
}

fn is_request_authenticated(state: &AdminState, headers: &HeaderMap) -> bool {
    let Some(expected_tok) = state.admin_token() else {
        return true;
    };
    if let Some(authz) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(bearer) = authz.strip_prefix("Bearer ")
        && secrets_equal(bearer.trim(), expected_tok)
    {
        return true;
    }
    if let Some(expected_cookie) = state.cookie_tag()
        && let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())
    {
        for part in cookie_hdr.split(';') {
            if let Some((k, v)) = part.trim().split_once('=')
                && k.trim() == SESSION_COOKIE
                && secrets_equal(v.trim(), expected_cookie)
            {
                return true;
            }
        }
    }
    false
}

/// `GET /api/v1/auth/session` handler.
pub async fn get_auth_session(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> axum::Json<AuthSessionDto> {
    axum::Json(AuthSessionDto {
        auth_required: state.admin_token().is_some(),
        authenticated: is_request_authenticated(&state, &headers),
    })
}

/// `POST /api/v1/auth/login` handler.
///
/// # Errors
/// Returns `401 Unauthorized` when `payload.token` does not match `admin_token`.
pub async fn login_admin(
    State(state): State<AdminState>,
    axum::Json(payload): axum::Json<LoginPayload>,
) -> Result<Response, ApiError> {
    let Some(expected) = state.admin_token() else {
        return Ok(axum::Json(serde_json::json!({ "authenticated": true })).into_response());
    };
    if !secrets_equal(payload.token.trim(), expected) {
        return Err(ApiError(AppError::Unauthorized("管理口令错误".to_string())));
    }
    let tag = state.cookie_tag().unwrap_or("");
    let cookie_val = format!("{SESSION_COOKIE}={tag}; HttpOnly; SameSite=Strict; Path=/");
    let mut resp = axum::Json(serde_json::json!({ "authenticated": true })).into_response();
    if let Ok(hv) = HeaderValue::from_str(&cookie_val) {
        resp.headers_mut().insert(header::SET_COOKIE, hv);
    }
    Ok(resp)
}

/// `POST /api/v1/auth/logout` handler.
pub async fn logout_admin(State(state): State<AdminState>) -> Response {
    let mut resp = axum::Json(serde_json::json!({
        "authenticated": state.admin_token().is_none(),
    }))
    .into_response();
    if let Ok(hv) = HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}=; Max-Age=0; HttpOnly; SameSite=Strict; Path=/"
    )) {
        resp.headers_mut().insert(header::SET_COOKIE, hv);
    }
    resp
}
