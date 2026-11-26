//! Shared control-plane application state and HTTP JSON error mapping.

use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::Serialize;
use sha2::{Digest, Sha256};
use x_upt_core::error::AppError;
use x_upt_fronts::supervisor::FrontSupervisor;
use x_upt_store::Store;

/// Shared state injected into all `/health` and `/api/v1/*` handlers.
#[derive(Clone, Debug)]
pub struct AdminState {
    inner: Arc<AdminInner>,
}

#[derive(Debug)]
struct AdminInner {
    version: &'static str,
    admin_listen: String,
    admin_port: u16,
    admin_token: Option<String>,
    cookie_tag: Option<String>,
    store: Arc<Store>,
    supervisor: FrontSupervisor,
}

impl AdminState {
    /// Constructs a new [`AdminState`].
    #[must_use]
    pub fn new(
        admin_listen: String,
        admin_port: u16,
        admin_token: Option<String>,
        store: Arc<Store>,
        supervisor: FrontSupervisor,
    ) -> Self {
        let cookie_tag = admin_token.as_ref().map(|tok| {
            let mut hasher = Sha256::new();
            hasher.update(b"x-upt-admin-session-v1:");
            hasher.update(tok.as_bytes());
            hex::encode(hasher.finalize())
        });
        Self {
            inner: Arc::new(AdminInner {
                version: env!("CARGO_PKG_VERSION"),
                admin_listen,
                admin_port,
                admin_token,
                cookie_tag,
                store,
                supervisor,
            }),
        }
    }

    /// Binary version.
    #[must_use]
    pub fn version(&self) -> &'static str {
        self.inner.version
    }

    /// Control-plane listen address string.
    #[must_use]
    pub fn admin_listen(&self) -> &str {
        &self.inner.admin_listen
    }

    /// Control-plane listen port.
    #[must_use]
    pub fn admin_port(&self) -> u16 {
        self.inner.admin_port
    }

    /// Optional management token.
    #[must_use]
    pub fn admin_token(&self) -> Option<&str> {
        self.inner.admin_token.as_deref()
    }

    /// Derived session cookie tag when `admin_token` is enabled.
    #[must_use]
    pub fn cookie_tag(&self) -> Option<&str> {
        self.inner.cookie_tag.as_deref()
    }

    /// Underlying `SQLite` store.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    /// Upstream listener supervisor.
    #[must_use]
    pub fn supervisor(&self) -> &FrontSupervisor {
        &self.inner.supervisor
    }

    /// Rebuilds the in-memory runtime snapshot from `SQLite` and atomically publishes it.
    ///
    /// # Errors
    /// Returns [`AppError`] if snapshot loading fails.
    pub fn refresh_snapshot(&self) -> Result<(), AppError> {
        let snap = self.inner.store.build_snapshot()?;
        self.inner.supervisor.runtime().store(snap);
        Ok(())
    }
}

/// HTTP response wrapper converting [`AppError`] into JSON `{error, message, missing?}`.
#[derive(Debug)]
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(value: AppError) -> Self {
        Self(value)
    }
}

#[derive(Serialize)]
struct ErrorPayload<'a> {
    error: &'static str,
    message: &'a str,
    #[serde(skip_serializing_if = "slice_is_empty")]
    missing: &'a [String],
}

fn slice_is_empty(s: &[String]) -> bool {
    s.is_empty()
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            AppError::InvalidInput(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Conflict { .. } => StatusCode::CONFLICT,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorPayload {
            error: self.0.code(),
            message: self.0.message(),
            missing: self.0.missing(),
        };
        (status, axum::Json(body)).into_response()
    }
}
