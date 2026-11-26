//! Control-plane health, system status, settings, method switches, and front lifecycle handlers.

use axum::Json;
use axum::extract::{Path, State};
use x_upt_core::domain::{BusinessMethod, FrontName, GlobalSettings};
use x_upt_core::error::AppError;
use x_upt_core::preconditions::MethodStatus;
use x_upt_fronts::supervisor::build_tls_acceptor;
use x_upt_store::dto::{FrontStatusView, FrontUpdatePayload, SystemStatusView};

use crate::state::{AdminState, ApiError};

/// `GET /health` readiness endpoint.
pub async fn get_health(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let running = state.supervisor().running_set().len();
    Json(serde_json::json!({
        "status": "ok",
        "version": state.version(),
        "running_fronts": running,
    }))
}

/// `GET /api/v1/status` full console status endpoint.
///
/// # Errors
/// Returns [`ApiError`] on `SQLite` query failure.
pub async fn get_system_status(
    State(state): State<AdminState>,
) -> Result<Json<SystemStatusView>, ApiError> {
    let running = state.supervisor().running_set();
    let status = state.store().system_status(
        state.version(),
        state.admin_listen(),
        state.admin_port(),
        &running,
    )?;
    Ok(Json(status))
}

/// `GET /api/v1/settings` global simulation parameters.
///
/// # Errors
/// Returns [`ApiError`] on `SQLite` read failure.
pub async fn get_settings(
    State(state): State<AdminState>,
) -> Result<Json<GlobalSettings>, ApiError> {
    Ok(Json(state.store().get_settings()?))
}

/// `PUT /api/v1/settings` update global simulation parameters.
///
/// # Errors
/// Returns [`ApiError`] on validation or `SQLite` write failure.
pub async fn update_settings(
    State(state): State<AdminState>,
    Json(payload): Json<GlobalSettings>,
) -> Result<Json<GlobalSettings>, ApiError> {
    let saved = state.store().update_settings(payload)?;
    state.refresh_snapshot()?;
    Ok(Json(saved))
}

/// `POST /api/v1/methods/{method}/enable` handler.
///
/// # Errors
/// Returns `409 Conflict` if required images or posts are missing.
pub async fn enable_method(
    State(state): State<AdminState>,
    Path(raw_method): Path<String>,
) -> Result<Json<MethodStatus>, ApiError> {
    set_method_common(&state, &raw_method, true)
}

/// `POST /api/v1/methods/{method}/disable` handler.
///
/// # Errors
/// Returns `409 Conflict` if active core fronts depend on this method.
pub async fn disable_method(
    State(state): State<AdminState>,
    Path(raw_method): Path<String>,
) -> Result<Json<MethodStatus>, ApiError> {
    set_method_common(&state, &raw_method, false)
}

fn set_method_common(
    state: &AdminState,
    raw_method: &str,
    enable: bool,
) -> Result<Json<MethodStatus>, ApiError> {
    let method = BusinessMethod::parse(raw_method)
        .ok_or_else(|| AppError::NotFound(format!("未知业务子接口: {raw_method}")))?;
    let status = state.store().set_method(method, enable)?;
    state.refresh_snapshot()?;
    Ok(Json(status))
}

/// `GET /api/v1/fronts` handler.
///
/// # Errors
/// Returns [`ApiError`] on `SQLite` read failure.
pub async fn list_fronts(
    State(state): State<AdminState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let running = state.supervisor().running_set();
    let items = state.store().list_fronts(
        state.version(),
        state.admin_listen(),
        state.admin_port(),
        &running,
    )?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// `PUT /api/v1/fronts/{front}` handler.
///
/// # Errors
/// Returns [`ApiError`] on invalid parameters, invalid TLS PEM, or port conflict.
pub async fn update_front(
    State(state): State<AdminState>,
    Path(raw_front): Path<String>,
    Json(payload): Json<FrontUpdatePayload>,
) -> Result<Json<FrontStatusView>, ApiError> {
    let front = parse_front_name(&raw_front)?;
    let updated = state
        .store()
        .update_front(state.admin_port(), front, payload)?;
    if updated.tls.enabled {
        drop(build_tls_acceptor(&updated)?);
    }
    state.refresh_snapshot()?;
    if updated.enabled {
        state.supervisor().rebind_if_needed(updated).await?;
    }
    let running = state.supervisor().running_set();
    Ok(Json(state.store().front_status(
        front,
        state.version(),
        state.admin_listen(),
        state.admin_port(),
        &running,
    )?))
}

/// `POST /api/v1/fronts/{front}/enable` handler.
///
/// # Errors
/// Returns `409 Conflict` when preconditions are missing or listener bind fails.
pub async fn enable_front(
    State(state): State<AdminState>,
    Path(raw_front): Path<String>,
) -> Result<Json<FrontStatusView>, ApiError> {
    let front = parse_front_name(&raw_front)?;
    let mut rec = state
        .store()
        .check_front_can_enable(state.admin_port(), front)?;
    if rec.tls.enabled {
        drop(build_tls_acceptor(&rec).map_err(|e| {
            AppError::precondition(
                format!("接入面 {} TLS 证书校验未通过", front.as_str()),
                vec![e.to_string()],
            )
        })?);
    }

    rec.enabled = true;
    state.store().set_front_enabled(front, true)?;
    state.refresh_snapshot()?;

    if let Err(bind_err) = state.supervisor().start_front(rec).await {
        drop(state.store().set_front_enabled(front, false));
        drop(state.refresh_snapshot());
        return Err(ApiError(bind_err));
    }

    let running = state.supervisor().running_set();
    Ok(Json(state.store().front_status(
        front,
        state.version(),
        state.admin_listen(),
        state.admin_port(),
        &running,
    )?))
}

/// `POST /api/v1/fronts/{front}/disable` handler.
///
/// # Errors
/// Returns [`ApiError`] if `front` is unknown or `SQLite` write fails.
pub async fn disable_front(
    State(state): State<AdminState>,
    Path(raw_front): Path<String>,
) -> Result<Json<FrontStatusView>, ApiError> {
    let front = parse_front_name(&raw_front)?;
    state.supervisor().stop_front(front).await;
    state.store().set_front_enabled(front, false)?;
    state.refresh_snapshot()?;
    let running = state.supervisor().running_set();
    Ok(Json(state.store().front_status(
        front,
        state.version(),
        state.admin_listen(),
        state.admin_port(),
        &running,
    )?))
}

pub(crate) fn parse_front_name(raw: &str) -> Result<FrontName, AppError> {
    FrontName::parse(raw).ok_or_else(|| AppError::NotFound(format!("未知接入面: {raw}")))
}
