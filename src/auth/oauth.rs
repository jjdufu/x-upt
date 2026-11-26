use std::collections::HashSet;

use axum::Json;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::auth::credentials_from_headers_or_form;
use crate::auth::token::{IssueError, TokenManager};
use crate::config::{MethodsConfig, granted_methods};
use crate::logger::context::LogContext;
use crate::service::apps::{AppRegistry, secrets_equal};
use crate::types::gateway::is_known_method;
use crate::types::token::{OAuthError, TokenForm, TokenResponse};

pub async fn issue_token(
    ctx: &LogContext,
    apps: &AppRegistry,
    tokens: &TokenManager,
    methods: MethodsConfig,
    headers: &HeaderMap,
    form: &TokenForm,
) -> Result<TokenResponse, Response> {
    ctx.set_in(serde_json::json!({
        "grant_type": form.grant_type,
        "scope": form.scope,
    }));

    if form.grant_type != "client_credentials" {
        ctx.set_fail(
            "unsupported_grant_type",
            "grant_type must be client_credentials",
            serde_json::json!("unsupported_grant_type"),
        );
        return Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type must be client_credentials",
        ));
    }

    let Some((client_id, client_secret)) = credentials_from_headers_or_form(
        headers,
        form.client_id.as_deref(),
        form.client_secret.as_deref(),
    ) else {
        ctx.set_fail(
            "invalid_client",
            "client authentication required",
            serde_json::json!("invalid_client"),
        );
        return Err(oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client authentication required",
        ));
    };

    ctx.set_client_id(client_id.clone());
    ctx.set_in(serde_json::json!({
        "grant_type": form.grant_type,
        "client_id": client_id,
        "scope": form.scope,
    }));

    let Some(app) = apps.get(&client_id) else {
        ctx.set_fail(
            "invalid_client",
            "unknown client_id",
            serde_json::json!("invalid_client"),
        );
        return Err(oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "unknown client_id",
        ));
    };

    if !secrets_equal(&app.client_secret, &client_secret) {
        ctx.set_fail(
            "invalid_client",
            "invalid client_secret",
            serde_json::json!("invalid_client"),
        );
        return Err(oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "invalid client_secret",
        ));
    }

    let allowed = granted_methods(&app.methods, methods);
    let granted = match resolve_scope(form.scope.as_deref(), &allowed) {
        Ok(methods) => methods,
        Err(desc) => {
            ctx.set_fail(
                "invalid_scope",
                desc.clone(),
                serde_json::json!("invalid_scope"),
            );
            return Err(oauth_error(StatusCode::BAD_REQUEST, "invalid_scope", &desc));
        }
    };

    match tokens.create_token(client_id, granted).await {
        Ok(resp) => {
            ctx.set_out(serde_json::json!({
                "token_type": resp.token_type,
                "expires_in": resp.expires_in,
                "scope": resp.scope,
            }));
            Ok(resp)
        }
        Err(IssueError::TooMany) => {
            ctx.set_fail(
                "invalid_request",
                "too many tokens for this client",
                serde_json::json!("invalid_request"),
            );
            Err(oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "too many tokens for this client",
            ))
        }
    }
}

#[must_use]
pub fn token_success(resp: TokenResponse) -> Response {
    let mut response = Json(resp).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

#[must_use]
pub fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    let mut response = (
        status,
        Json(OAuthError {
            error: error.to_string(),
            error_description: description.to_string(),
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Basic realm="upupt""#),
        );
    }
    response
}

fn resolve_scope(
    scope: Option<&str>,
    allowed: &HashSet<String>,
) -> Result<HashSet<String>, String> {
    let Some(scope) = scope.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(allowed.clone());
    };

    let mut methods = HashSet::new();
    for item in scope.split_whitespace() {
        if !is_known_method(item) || !allowed.contains(item) {
            return Err(format!("scope {item} is not granted to this client"));
        }
        methods.insert(item.to_string());
    }
    if methods.is_empty() {
        return Err("scope must list at least one method".to_string());
    }
    Ok(methods)
}
