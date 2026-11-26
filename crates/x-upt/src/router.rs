//! Control-plane `axum::Router` wiring all 27 OpenAPI paths, security guards, and embedded SPA.

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, post, put};

use crate::api_assets::{
    create_app, create_post, delete_app, delete_envelope_secret, delete_image, delete_post,
    download_contract, get_envelope_profiles, get_front_docs, get_image_raw, get_recent_logs,
    import_images, import_posts, list_apps, list_envelope_secrets, list_images, list_jobs,
    list_posts, replace_envelope_profiles, update_app, update_post, upsert_envelope_secret,
};
use crate::api_control::{
    disable_front, disable_method, enable_front, enable_method, get_health, get_settings,
    get_system_status, list_fronts, update_front, update_settings,
};
use crate::security::{
    admin_auth_guard, get_auth_session, login_admin, logout_admin, origin_and_host_guard,
};
use crate::state::AdminState;
use crate::static_files::spa_or_not_found;

/// Builds the complete control-plane router with CSRF/Host guards, optional token auth, and SPA assets.
pub fn build_admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/health", get(get_health))
        .route("/api/v1/auth/session", get(get_auth_session))
        .route("/api/v1/auth/login", post(login_admin))
        .route("/api/v1/auth/logout", post(logout_admin))
        .route("/api/v1/status", get(get_system_status))
        .route("/api/v1/settings", get(get_settings).put(update_settings))
        .route("/api/v1/methods/{method}/enable", post(enable_method))
        .route("/api/v1/methods/{method}/disable", post(disable_method))
        .route("/api/v1/fronts", get(list_fronts))
        .route("/api/v1/fronts/{front}", put(update_front))
        .route("/api/v1/fronts/{front}/enable", post(enable_front))
        .route("/api/v1/fronts/{front}/disable", post(disable_front))
        .route("/api/v1/images", get(list_images).post(import_images))
        .route("/api/v1/images/{id}/raw", get(get_image_raw))
        .route("/api/v1/images/{id}", delete(delete_image))
        .route("/api/v1/posts", get(list_posts).post(create_post))
        .route("/api/v1/posts/import", post(import_posts))
        .route("/api/v1/posts/{id}", put(update_post).delete(delete_post))
        .route("/api/v1/apps", get(list_apps).post(create_app))
        .route(
            "/api/v1/apps/{client_id}",
            put(update_app).delete(delete_app),
        )
        .route(
            "/api/v1/envelopes/secrets",
            get(list_envelope_secrets).post(upsert_envelope_secret),
        )
        .route(
            "/api/v1/envelopes/secrets/{name}",
            delete(delete_envelope_secret),
        )
        .route(
            "/api/v1/envelopes/{front}/profiles",
            get(get_envelope_profiles).put(replace_envelope_profiles),
        )
        .route("/api/v1/docs/fronts", get(get_front_docs))
        .route("/api/v1/contracts/{name}", get(download_contract))
        .route("/api/v1/logs/recent", get(get_recent_logs))
        .route("/api/v1/jobs", get(list_jobs))
        .fallback(spa_or_not_found)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            admin_auth_guard,
        ))
        .layer(middleware::from_fn(origin_and_host_guard))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use http::{Request, StatusCode, header};
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;
    use x_upt_fronts::jobstore::JobStore;
    use x_upt_fronts::logger::AccessLogger;
    use x_upt_fronts::snapshot::SharedRuntime;
    use x_upt_fronts::supervisor::FrontSupervisor;
    use x_upt_store::Store;

    use crate::router::build_admin_router;
    use crate::state::AdminState;

    fn test_router(
        admin_token: Option<String>,
    ) -> Result<(axum::Router, std::path::PathBuf), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("x-upt-admin-test-{}", uuid::Uuid::new_v4()));
        let store = Arc::new(Store::open(&dir)?);
        let snap = store.build_snapshot()?;
        let runtime = SharedRuntime::new(snap);
        let logger = AccessLogger::in_memory();
        let jobs = Arc::new(JobStore::new());
        let sup = FrontSupervisor::new(runtime, logger, jobs, CancellationToken::new());
        let state = AdminState::new("127.0.0.1:18768".into(), 18768, admin_token, store, sup);
        Ok((build_admin_router(state), dir))
    }

    #[tokio::test]
    async fn api_404_isolation_and_blank_precondition_conflict()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, dir) = test_router(None)?;

        // Unknown /api/* route MUST return 404 JSON, never HTML.
        let req_404 = Request::builder()
            .method("GET")
            .uri("/api/v1/does-not-exist")
            .header(header::HOST, "127.0.0.1:18768")
            .body(axum::body::Body::empty())?;
        let resp_404 = app.clone().oneshot(req_404).await?;
        assert_eq!(resp_404.status(), StatusCode::NOT_FOUND);
        let ct = resp_404
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("application/json"));

        // Missing /assets/* MUST return 404.
        let req_asset = Request::builder()
            .method("GET")
            .uri("/assets/nonexistent-hash.js")
            .header(header::HOST, "127.0.0.1:18768")
            .body(axum::body::Body::empty())?;
        let resp_asset = app.clone().oneshot(req_asset).await?;
        assert_eq!(resp_asset.status(), StatusCode::NOT_FOUND);

        // Enabling photos.render without imported images MUST return 409 Conflict.
        let req_enable = Request::builder()
            .method("POST")
            .uri("/api/v1/methods/x_upt.photos.render/enable")
            .header(header::HOST, "127.0.0.1:18768")
            .header(header::ORIGIN, "http://127.0.0.1:18768")
            .body(axum::body::Body::empty())?;
        let resp_enable = app.clone().oneshot(req_enable).await?;
        assert_eq!(resp_enable.status(), StatusCode::CONFLICT);

        // Cross-origin mutation MUST return 403 Forbidden.
        let req_csrf = Request::builder()
            .method("POST")
            .uri("/api/v1/methods/x_upt.photos.render/enable")
            .header(header::HOST, "127.0.0.1:18768")
            .header(header::ORIGIN, "http://evil.example.com")
            .body(axum::body::Body::empty())?;
        let resp_csrf = app.oneshot(req_csrf).await?;
        assert_eq!(resp_csrf.status(), StatusCode::FORBIDDEN);

        drop(std::fs::remove_dir_all(&dir));
        Ok(())
    }
}
