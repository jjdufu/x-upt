use std::sync::Arc;

use axum::http::HeaderMap;
use base64::Engine;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tonic::{Request as TonicRequest, Response as TonicResponse, Status};

use crate::auth::token::TokenManager;
use crate::auth::{AuthInput, AuthPolicy, authenticate, policy_from};
use crate::config::{AppConfig, apply_latency};
use crate::core::{Call, Core, CoreErrorKind};
use crate::front::access::{AccessMeta, access_log};
use crate::front::http_listen::{http_mode, load_tls, spawn_http};
use crate::logger;
use crate::logger::context::LogContext;
use crate::types::gateway::{METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH};
use crate::types::token::AuthedApp;

pub mod proto {
    tonic::include_proto!("upupt.v1");
}

#[derive(Clone)]
struct GrpcState {
    core: Arc<Core>,
    config: Arc<AppConfig>,
    policy: AuthPolicy,
    meta: AccessMeta,
}

struct UpuptSvc {
    state: GrpcState,
}

#[tonic::async_trait]
impl proto::upupt_server::Upupt for UpuptSvc {
    async fn search_posts(
        &self,
        req: TonicRequest<proto::SearchPostsRequest>,
    ) -> Result<TonicResponse<proto::SearchPostsResponse>, Status> {
        let (app, ctx) = self.auth(&req).await?;
        ctx.add_duration_latency(apply_latency(&self.state.config.latency).await);
        let inner = req.into_inner();
        let biz = serde_json::json!({
            "keyword": inner.keyword,
            "user_id": inner.user_id,
            "page": inner.page,
            "page_size": inner.page_size,
            "sort": inner.sort,
        });
        map_result(
            &ctx,
            crate::core::dispatch_logged(
                &self.state.core,
                &ctx,
                Call::from_app(&app, METHOD_POSTS_SEARCH.to_string(), biz),
            ),
            decode_posts,
        )
    }

    async fn render_photo(
        &self,
        req: TonicRequest<proto::RenderPhotoRequest>,
    ) -> Result<TonicResponse<proto::RenderPhotoResponse>, Status> {
        let (app, ctx) = self.auth(&req).await?;
        ctx.add_duration_latency(apply_latency(&self.state.config.latency).await);
        let inner = req.into_inner();
        let biz = serde_json::json!({
            "scene": inner.scene,
            "width": inner.width,
            "height": inner.height,
            "format": inner.format,
        });
        map_result(
            &ctx,
            crate::core::dispatch_logged(
                &self.state.core,
                &ctx,
                Call::from_app(&app, METHOD_PHOTOS_RENDER.to_string(), biz),
            ),
            decode_photo,
        )
    }

    async fn compare_image(
        &self,
        req: TonicRequest<proto::CompareImageRequest>,
    ) -> Result<TonicResponse<proto::CompareImageResponse>, Status> {
        let (app, ctx) = self.auth(&req).await?;
        ctx.add_duration_latency(apply_latency(&self.state.config.latency).await);
        let inner = req.into_inner();
        let biz = serde_json::json!({
            "filename": inner.filename,
            "image": inner.image,
            "threshold": inner.threshold,
        });
        map_result(
            &ctx,
            crate::core::dispatch_logged(
                &self.state.core,
                &ctx,
                Call::from_app(&app, METHOD_IMGDEF_COMPARE.to_string(), biz),
            ),
            decode_imgdef,
        )
    }
}

impl UpuptSvc {
    async fn auth<T>(&self, req: &TonicRequest<T>) -> Result<(AuthedApp, LogContext), Status> {
        let ctx = req
            .extensions()
            .get::<LogContext>()
            .cloned()
            .unwrap_or_else(|| {
                let rid = req
                    .metadata()
                    .get("x-request-id")
                    .and_then(|v| v.to_str().ok());
                LogContext::new(
                    logger::resolve_request_id_raw(rid),
                    self.state.meta.front.as_str().to_string(),
                    self.state.meta.port,
                )
            });
        let headers = metadata_headers(req.metadata());
        let input = AuthInput {
            headers: &headers,
            query: None,
            method: "POST",
            path: "/",
            body: &[],
            query_overrides_header: false,
        };
        let start_auth = std::time::Instant::now();
        match authenticate(&self.state.policy, &input).await {
            Ok(ok) => {
                ctx.set_client_id(ok.app.client_id.clone());
                ctx.set_auth(ok.method.as_str());
                ctx.add_duration_auth(crate::logger::elapsed_ms(start_auth));
                Ok((ok.app, ctx))
            }
            Err(_) => {
                ctx.add_duration_auth(crate::logger::elapsed_ms(start_auth));
                ctx.set_fail(
                    "unauthenticated",
                    "unauthenticated",
                    serde_json::json!("unauthenticated"),
                );
                Err(status_with_rid(
                    &ctx,
                    Status::unauthenticated("unauthenticated"),
                ))
            }
        }
    }
}

fn map_result<T>(
    ctx: &LogContext,
    result: Result<serde_json::Value, crate::core::CoreError>,
    map: impl FnOnce(&serde_json::Value) -> T,
) -> Result<TonicResponse<T>, Status> {
    match result {
        Ok(value) => Ok(response_with_rid(ctx, TonicResponse::new(map(&value)))),
        Err(err) => {
            ctx.set_code(serde_json::json!(err.kind.as_str()));
            Err(status_with_rid(ctx, core_status(&err.kind, err.message())))
        }
    }
}

fn response_with_rid<T>(ctx: &LogContext, mut resp: TonicResponse<T>) -> TonicResponse<T> {
    if let Ok(value) = ctx.request_id().parse() {
        resp.metadata_mut().insert("x-request-id", value);
    }
    resp
}

fn status_with_rid(ctx: &LogContext, mut status: Status) -> Status {
    if let Ok(value) = ctx.request_id().parse() {
        status.metadata_mut().insert("x-request-id", value);
    }
    status
}

fn metadata_headers(md: &tonic::metadata::MetadataMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(v) = md.get("authorization").and_then(|v| v.to_str().ok())
        && let Ok(value) = axum::http::HeaderValue::from_str(v)
    {
        headers.insert(axum::http::header::AUTHORIZATION, value);
    }
    if let Some(v) = md.get("x-api-key").and_then(|v| v.to_str().ok())
        && let Ok(value) = axum::http::HeaderValue::from_str(v)
    {
        headers.insert("x-api-key", value);
    }
    headers
}

fn core_status(kind: &CoreErrorKind, message: &str) -> Status {
    match kind {
        CoreErrorKind::Forbidden => Status::permission_denied(message),
        CoreErrorKind::UnknownMethod | CoreErrorKind::NotFound(_) => Status::not_found(message),
        CoreErrorKind::BadRequest(_) | CoreErrorKind::Unprocessable(_) => {
            Status::invalid_argument(message)
        }
        CoreErrorKind::Internal(_) => Status::internal(message),
    }
}

fn decode_posts(value: &serde_json::Value) -> proto::SearchPostsResponse {
    let items = value
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|p| proto::Post {
            user_id: p
                .get("userId")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            id: p.get("id").and_then(serde_json::Value::as_u64).unwrap_or(0),
            title: p
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            body: p
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .collect();
    proto::SearchPostsResponse {
        items,
        total: u32::try_from(
            value
                .get("total")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(u32::MAX),
        page: u32::try_from(
            value
                .get("page")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
        page_size: u32::try_from(
            value
                .get("page_size")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
    }
}

fn decode_photo(value: &serde_json::Value) -> proto::RenderPhotoResponse {
    let image = &value["image"];
    let b64 = image.get("base64").and_then(|v| v.as_str()).unwrap_or("");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap_or_default();
    proto::RenderPhotoResponse {
        scene: value
            .get("scene")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        width: u32::try_from(
            value
                .get("width")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
        height: u32::try_from(
            value
                .get("height")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
        format: value
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        filename: image
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        mime: image
            .get("mime")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        size: image
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        image_bytes: bytes,
        image_base64: String::new(),
    }
}

fn decode_imgdef(value: &serde_json::Value) -> proto::CompareImageResponse {
    proto::CompareImageResponse {
        matched: value
            .get("matched")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        filename: value
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        distance: u32::try_from(
            value
                .get("distance")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
        threshold: u32::try_from(
            value
                .get("threshold")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
    }
}

pub fn spawn(
    listener: TcpListener,
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Option<Arc<TokenManager>>,
    shutdown: CancellationToken,
    meta: AccessMeta,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let front = &config.fronts.grpc;
    let policy = policy_from(
        front.common.auth.clone(),
        tokens,
        Arc::clone(&config.apps),
        front.common.port,
    );
    let state = GrpcState {
        core,
        config: Arc::clone(&config),
        policy,
        meta,
    };
    let svc = UpuptSvc { state };
    let routes = tonic::service::Routes::new(proto::upupt_server::UpuptServer::new(svc));
    let router = routes
        .into_axum_router()
        .layer(axum::middleware::from_fn(access_log))
        .layer(axum::Extension(meta));
    let tls = load_tls(&front.common, true)?;
    Ok(spawn_http(
        listener,
        tls,
        http_mode(&front.common, true),
        router,
        meta,
        shutdown,
    ))
}
