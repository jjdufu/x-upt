//! Embedded `web/dist` SPA static server with strict `/api/*` & `/assets/*` 404 isolation and caching.

use axum::body::Body;
use axum::extract::OriginalUri;
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use sha2::{Digest, Sha256};
use x_upt_core::error::AppError;

use crate::state::ApiError;

#[cfg(frontend_embedded)]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
struct FrontendAssets;

#[cfg(frontend_embedded)]
fn load_embedded(rel: &str) -> Option<Vec<u8>> {
    FrontendAssets::get(rel).map(|f| f.data.into_owned())
}

#[cfg(not(frontend_embedded))]
fn load_embedded(_rel: &str) -> Option<Vec<u8>> {
    None
}

/// Fallback handler serving embedded SPA assets while keeping `/api/*` and `/health` strictly JSON-only.
pub async fn spa_or_not_found(
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    if path == "/health" || path == "/api" || path.starts_with("/api/") {
        return ApiError(AppError::NotFound(format!(
            "API endpoint not found: {path}"
        )))
        .into_response();
    }

    if !matches!(method, Method::GET | Method::HEAD) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let trimmed = path.trim_start_matches('/');
    if path.starts_with("/assets/") {
        return match load_embedded(trimmed) {
            Some(bytes) => asset_response(&method, trimmed, bytes, true),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }

    if !trimmed.is_empty() {
        if let Some(bytes) = load_embedded(trimmed) {
            return asset_response(&method, trimmed, bytes, false);
        }
        if has_file_extension(trimmed) {
            return StatusCode::NOT_FOUND.into_response();
        }
    }

    match load_embedded("index.html") {
        Some(bytes) => index_html_response(&method, &headers, bytes),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "WebUI bundle not embedded in this debug build",
        )
            .into_response(),
    }
}

fn has_file_extension(rel: &str) -> bool {
    rel.rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
}

fn asset_response(method: &Method, rel: &str, bytes: Vec<u8>, immutable: bool) -> Response {
    let mime = guess_mime(rel);
    let cache = if immutable {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    };
    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp
}

fn index_html_response(method: &Method, req_headers: &HeaderMap, bytes: Vec<u8>) -> Response {
    let digest = hex::encode(Sha256::digest(&bytes));
    let etag = format!("\"{}\"", digest.get(..32).unwrap_or(&digest));

    if let Some(inm) = req_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && inm.split(',').any(|t| t.trim() == etag)
    {
        let mut not_mod = StatusCode::NOT_MODIFIED.into_response();
        if let Ok(hv) = HeaderValue::from_str(&etag) {
            not_mod.headers_mut().insert(header::ETAG, hv);
        }
        not_mod
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        return not_mod;
    }

    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(hv) = HeaderValue::from_str(&etag) {
        resp.headers_mut().insert(header::ETAG, hv);
    }
    resp
}

fn guess_mime(rel: &str) -> &'static str {
    let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}
