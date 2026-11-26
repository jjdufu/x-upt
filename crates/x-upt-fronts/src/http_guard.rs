//! Shared HTTP authentication adapter, `X-Request-Id` header helper, and runtime context.

use std::sync::Arc;
use std::time::Instant;

use axum::response::Response;
use http::{HeaderMap, HeaderValue};
use x_upt_core::domain::{AuthMethod, AuthedApp, FrontName, granted_methods};
use x_upt_crypto::http_auth::{AuthError, AuthInput, AuthOk, AuthPolicy, authenticate};
use x_upt_store::dto::RuntimeSnapshotData;

use crate::jobstore::JobStore;
use crate::logger::AccessLogger;
use crate::snapshot::SharedRuntime;
use crate::token::FrontSecurityState;

/// Shared state injected into every protocol router spawned for one front.
#[derive(Debug, Clone)]
pub struct FrontCtx {
    /// Front identifier.
    pub front: FrontName,
    /// Bound port.
    pub port: u16,
    /// Shared runtime snapshot handle.
    pub runtime: SharedRuntime,
    /// Isolated token and nonce stores for this front.
    pub sec: FrontSecurityState,
    /// Shared structured access logger.
    pub logger: AccessLogger,
    /// Shared async job queue (`job` front & `/api/v1/jobs`).
    pub jobs: Arc<JobStore>,
}

/// Verified HTTP caller with elapsed auth duration.
#[derive(Debug, Clone)]
pub struct VerifiedCaller {
    /// Caller identity with methods intersected with globally enabled methods.
    pub app: AuthedApp,
    /// Matched authentication mode.
    pub method: AuthMethod,
    /// Authentication check duration in milliseconds.
    pub duration_ms: u64,
}

/// Failed HTTP authentication details.
#[derive(Debug, Clone)]
pub struct AuthFailure {
    /// Categorized error.
    pub error: AuthError,
    /// Elapsed milliseconds.
    pub duration_ms: u64,
}

/// Authenticates an HTTP request against the front's configured `auth` list in `snap`.
///
/// # Errors
/// Returns [`AuthFailure`] if credentials are missing, malformed, or invalid.
pub fn verify_http_request(
    ctx: &FrontCtx,
    snap: &RuntimeSnapshotData,
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<VerifiedCaller, AuthFailure> {
    let t0 = Instant::now();
    let allowed_modes = snap
        .fronts
        .get(&ctx.front)
        .map_or_else(|| ctx.front.default_auth(), |f| f.auth.clone());

    let policy = AuthPolicy {
        methods: allowed_modes,
        apps: Arc::new(snap.apps.clone()),
        digest_nonces: Arc::clone(&ctx.sec.digest_nonces),
        hawk_nonces: Arc::clone(&ctx.sec.hawk_nonces),
        listen_port: ctx.port,
    };
    let input = AuthInput {
        method,
        path,
        query,
        headers,
        body,
        query_overrides_header: ctx.front == FrontName::Ws,
    };
    let lookup = |tok: &str| ctx.sec.tokens.verify(tok);
    match authenticate(&policy, &input, Some(&lookup)) {
        Ok(AuthOk { mut app, method: m }) => {
            if m == AuthMethod::None {
                app.methods = x_upt_core::domain::BusinessMethod::all()
                    .into_iter()
                    .filter(|bm| snap.dataset.methods.get(*bm))
                    .map(|bm| bm.as_str().to_string())
                    .collect();
            } else {
                let raw: Vec<String> = app.methods.iter().cloned().collect();
                app.methods = granted_methods(&raw, snap.dataset.methods);
            }
            let elapsed = u64::try_from(t0.elapsed().as_millis()).unwrap_or(0);
            Ok(VerifiedCaller {
                app,
                method: m,
                duration_ms: elapsed,
            })
        }
        Err(error) => Err(AuthFailure {
            error,
            duration_ms: u64::try_from(t0.elapsed().as_millis()).unwrap_or(0),
        }),
    }
}

/// Attaches `X-Request-Id` and optional Digest `WWW-Authenticate` challenge headers to `resp`.
pub fn attach_std_headers(
    resp: &mut Response,
    request_id: &str,
    digest_challenges: Option<&[String]>,
) {
    if let Ok(hv) = HeaderValue::from_str(request_id) {
        resp.headers_mut().insert("x-request-id", hv);
    }
    if let Some(challenges) = digest_challenges {
        for ch in challenges {
            if let Ok(hv) = HeaderValue::from_str(ch) {
                resp.headers_mut()
                    .append(http::header::WWW_AUTHENTICATE, hv);
            }
        }
    }
}
