//! Unified error types for `x-upt` core domain and control plane.

use std::fmt;

use serde::Serialize;

/// Unified application error across domain, storage, and management endpoints.
#[derive(Debug, Clone)]
pub enum AppError {
    /// Malformed or out-of-range user input (`400 Bad Request`).
    InvalidInput(String),
    /// Missing authentication credentials (`401 Unauthorized`).
    Unauthorized(String),
    /// Operation disallowed (`403 Forbidden`).
    Forbidden(String),
    /// Requested entity does not exist (`404 Not Found`).
    NotFound(String),
    /// Switch precondition or active-dependency lock violation (`409 Conflict`).
    Conflict {
        /// Short machine-readable error code.
        code: &'static str,
        /// Human-readable explanation.
        message: String,
        /// Specific missing preconditions or blocking dependencies.
        missing: Vec<String>,
    },
    /// Unexpected storage or runtime error (`500 Internal Server Error`).
    Internal(String),
}

impl AppError {
    /// Creates a precondition conflict error with structured missing items.
    #[must_use]
    pub fn precondition(message: impl Into<String>, missing: Vec<String>) -> Self {
        Self::Conflict {
            code: "conflict_precondition",
            message: message.into(),
            missing,
        }
    }

    /// Creates an active-dependency conflict error when a resource is in use.
    #[must_use]
    pub fn in_use(message: impl Into<String>, blocking: Vec<String>) -> Self {
        Self::Conflict {
            code: "conflict_in_use",
            message: message.into(),
            missing: blocking,
        }
    }

    /// Machine-readable short error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict { code, .. } => code,
            Self::Internal(_) => "internal",
        }
    }

    /// Human-readable message text.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidInput(msg)
            | Self::Unauthorized(msg)
            | Self::Forbidden(msg)
            | Self::NotFound(msg)
            | Self::Conflict { message: msg, .. }
            | Self::Internal(msg) => msg,
        }
    }

    /// Missing precondition details if applicable.
    #[must_use]
    pub fn missing(&self) -> &[String] {
        match self {
            Self::Conflict { missing, .. } => missing,
            _ => &[],
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for AppError {}

/// Error kind returned by deterministic upstream business dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CoreErrorKind {
    /// Request shape or parameter bounds error.
    BadRequest(String),
    /// Method does not exist or is currently disabled.
    UnknownMethod,
    /// Authenticated caller is not granted the requested method.
    Forbidden,
    /// Business payload failed schema deserialization.
    Unprocessable(String),
    /// Referenced asset (such as image filename) does not exist.
    NotFound(String),
    /// Internal image/JSON processing failure.
    Internal(String),
}

impl CoreErrorKind {
    /// Short snake_case identifier recorded in `access_log`.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::UnknownMethod => "unknown_method",
            Self::Forbidden => "forbidden",
            Self::Unprocessable(_) => "unprocessable",
            Self::NotFound(_) => "not_found",
            Self::Internal(_) => "internal",
        }
    }
}

/// Upstream business dispatch error wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    /// Categorized failure reason.
    pub kind: CoreErrorKind,
}

impl CoreError {
    /// Constructs a new [`CoreError`].
    #[must_use]
    pub fn new(kind: CoreErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the error description suitable for protocol responses.
    #[must_use]
    pub fn message(&self) -> &str {
        match &self.kind {
            CoreErrorKind::BadRequest(s)
            | CoreErrorKind::Unprocessable(s)
            | CoreErrorKind::NotFound(s)
            | CoreErrorKind::Internal(s) => s,
            CoreErrorKind::UnknownMethod => "unknown method",
            CoreErrorKind::Forbidden => "method is not granted to this client",
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CoreError {}
