//! Unary HTTP/2 gRPC router (`/x_upt.v1.XUpt/{SearchPosts,RenderPhoto,CompareImage}`).

use std::time::Instant;

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use base64::Engine;
use bytes::Bytes;
use http::HeaderMap;
use prost::Message;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::{Code, Status};
use x_upt_core::domain::{METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH};
use x_upt_core::error::{CoreError, CoreErrorKind};
use x_upt_core::services::Call;

use crate::http_guard::{FrontCtx, verify_http_request};
use crate::logger::{LogOutcome, ReqMeta, build_log_entry};
use crate::snapshot::simulate_latency;

#[derive(Clone, PartialEq, Message)]
struct SearchPostsReq {
    #[prost(string, tag = "1")]
    keyword: String,
    #[prost(uint64, tag = "2")]
    user_id: u64,
    #[prost(uint32, tag = "3")]
    page: u32,
    #[prost(uint32, tag = "4")]
    page_size: u32,
    #[prost(string, tag = "5")]
    sort: String,
}

#[derive(Clone, PartialEq, Message)]
struct PostMsg {
    #[prost(uint64, tag = "1")]
    user_id: u64,
    #[prost(uint64, tag = "2")]
    id: u64,
    #[prost(string, tag = "3")]
    title: String,
    #[prost(string, tag = "4")]
    body: String,
}

#[derive(Clone, PartialEq, Message)]
struct SearchPostsResp {
    #[prost(message, repeated, tag = "1")]
    items: Vec<PostMsg>,
    #[prost(uint32, tag = "2")]
    total: u32,
    #[prost(uint32, tag = "3")]
    page: u32,
    #[prost(uint32, tag = "4")]
    page_size: u32,
}

#[derive(Clone, PartialEq, Message)]
struct RenderPhotoReq {
    #[prost(string, tag = "1")]
    scene: String,
    #[prost(uint32, tag = "2")]
    width: u32,
    #[prost(uint32, tag = "3")]
    height: u32,
    #[prost(string, tag = "4")]
    format: String,
}

#[derive(Clone, PartialEq, Message)]
struct RenderPhotoResp {
    #[prost(string, tag = "1")]
    scene: String,
    #[prost(uint32, tag = "2")]
    width: u32,
    #[prost(uint32, tag = "3")]
    height: u32,
    #[prost(string, tag = "4")]
    format: String,
    #[prost(string, tag = "5")]
    filename: String,
    #[prost(string, tag = "6")]
    mime: String,
    #[prost(uint64, tag = "7")]
    size: u64,
    #[prost(bytes = "vec", tag = "8")]
    image_bytes: Vec<u8>,
    #[prost(string, tag = "9")]
    image_base64: String,
}

#[derive(Clone, PartialEq, Message)]
struct CompareImageReq {
    #[prost(string, tag = "1")]
    filename: String,
    #[prost(string, tag = "2")]
    image: String,
    #[prost(uint32, tag = "3")]
    threshold: u32,
}

#[derive(Clone, PartialEq, Message)]
struct CompareImageResp {
    #[prost(bool, tag = "1")]
    matched: bool,
    #[prost(string, tag = "2")]
    filename: String,
    #[prost(uint32, tag = "3")]
    distance: u32,
    #[prost(uint32, tag = "4")]
    threshold: u32,
}

#[derive(Default)]
struct RawBytesCodec;

struct RawBytesEncoder;
struct RawBytesDecoder;

impl Encoder for RawBytesEncoder {
    type Item = Bytes;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        use bytes::BufMut;
        dst.put_slice(&item);
        Ok(())
    }
}

impl Decoder for RawBytesDecoder {
    type Item = Bytes;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        use bytes::Buf;
        let len = src.remaining();
        if len == 0 {
            return Ok(Some(Bytes::new()));
        }
        Ok(Some(src.copy_to_bytes(len)))
    }
}

impl Codec for RawBytesCodec {
    type Encode = Bytes;
    type Decode = Bytes;
    type Encoder = RawBytesEncoder;
    type Decoder = RawBytesDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        RawBytesEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        RawBytesDecoder
    }
}

struct UnaryRpcSvc {
    ctx: FrontCtx,
    op: &'static str,
    headers: HeaderMap,
}

impl tonic::server::UnaryService<Bytes> for UnaryRpcSvc {
    type Response = Bytes;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<tonic::Response<Bytes>, Status>> + Send>,
    >;

    fn call(&mut self, request: tonic::Request<Bytes>) -> Self::Future {
        let ctx = self.ctx.clone();
        let op = self.op;
        let headers = self.headers.clone();
        Box::pin(async move { execute_grpc_op(ctx, op, headers, request.into_inner()).await })
    }
}

/// Builds the `grpc` front `axum::Router`.
pub fn build_grpc_router(ctx: FrontCtx) -> Router {
    Router::new()
        .route("/x_upt.v1.XUpt/SearchPosts", post(grpc_search_posts))
        .route("/x_upt.v1.XUpt/RenderPhoto", post(grpc_render_photo))
        .route("/x_upt.v1.XUpt/CompareImage", post(grpc_compare_image))
        .fallback(grpc_unimplemented)
        .with_state(ctx)
}

async fn grpc_search_posts(State(ctx): State<FrontCtx>, req: axum::extract::Request) -> Response {
    serve_unary(ctx, "SearchPosts", req).await
}

async fn grpc_render_photo(State(ctx): State<FrontCtx>, req: axum::extract::Request) -> Response {
    serve_unary(ctx, "RenderPhoto", req).await
}

async fn grpc_compare_image(State(ctx): State<FrontCtx>, req: axum::extract::Request) -> Response {
    serve_unary(ctx, "CompareImage", req).await
}

async fn grpc_unimplemented(State(ctx): State<FrontCtx>, req: axum::extract::Request) -> Response {
    serve_unary(ctx, "Unknown", req).await
}

async fn serve_unary(ctx: FrontCtx, op: &'static str, req: axum::extract::Request) -> Response {
    let headers = req.headers().clone();
    let svc = UnaryRpcSvc { ctx, op, headers };
    let mut grpc = tonic::server::Grpc::new(RawBytesCodec);
    let http_resp = grpc.unary(svc, req).await;
    let (parts, body) = http_resp.into_parts();
    Response::from_parts(parts, axum::body::Body::new(body))
}

async fn execute_grpc_op(
    ctx: FrontCtx,
    op: &'static str,
    headers: HeaderMap,
    raw: Bytes,
) -> Result<tonic::Response<Bytes>, Status> {
    let path = format!("/x_upt.v1.XUpt/{op}");
    let meta = ReqMeta::from_http("POST", &path, None, &headers, None, Some(raw.len()));
    let req_id = meta.request_id.clone();
    if op == "Unknown" {
        return Err(Status::unimplemented("unknown gRPC method"));
    }

    let snap = ctx.runtime.load();
    let caller =
        verify_http_request(&ctx, &snap, "POST", &path, None, &headers, &[]).map_err(|f| {
            ctx.logger.record(build_log_entry(
                ctx.front,
                ctx.port,
                meta.clone(),
                LogOutcome {
                    status: 401,
                    duration_auth: Some(f.duration_ms),
                    error: Some("unauthenticated".to_string()),
                    error_kind: Some("unauthorized".to_string()),
                    ..LogOutcome::default()
                },
            ));
            Status::unauthenticated("invalid or missing gRPC credentials")
        })?;

    let (method, biz) = decode_grpc_req(op, &raw)?;
    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let out_val = snap
        .dataset
        .dispatch(Call::from_app(&caller.app, method.to_string(), biz.clone()))
        .map_err(|err| {
            ctx.logger.record(build_log_entry(
                ctx.front,
                ctx.port,
                meta.clone(),
                LogOutcome {
                    status: 400,
                    duration_auth: Some(caller.duration_ms),
                    duration_latency: Some(lat_ms),
                    client_id: Some(caller.app.client_id.clone()),
                    auth: Some(caller.method.as_str().to_string()),
                    rpc: Some(method.to_string()),
                    error: Some(err.message().to_string()),
                    error_kind: Some(err.kind.as_str().to_string()),
                    input: biz.clone(),
                    ..LogOutcome::default()
                },
            ));
            map_grpc_err(&err)
        })?;
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);
    let encoded = encode_grpc_resp(op, &out_val)?;

    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 200,
            code: Some(serde_json::json!(0)),
            duration_auth: Some(caller.duration_ms),
            duration_core: Some(core_ms),
            duration_latency: Some(lat_ms),
            client_id: Some(caller.app.client_id),
            bytes_out: Some(encoded.len()),
            auth: Some(caller.method.as_str().to_string()),
            rpc: Some(method.to_string()),
            input: biz,
            out: out_val,
            ..LogOutcome::default()
        },
    ));

    let mut resp = tonic::Response::new(Bytes::from(encoded));
    if let Ok(mv) = req_id.parse() {
        resp.metadata_mut().insert("x-request-id", mv);
    }
    Ok(resp)
}

fn decode_grpc_req(op: &str, raw: &[u8]) -> Result<(&'static str, serde_json::Value), Status> {
    match op {
        "SearchPosts" => {
            let m = SearchPostsReq::decode(raw)
                .map_err(|e| Status::invalid_argument(format!("decode error: {e}")))?;
            Ok((
                METHOD_POSTS_SEARCH,
                serde_json::json!({
                    "keyword": m.keyword,
                    "user_id": m.user_id,
                    "page": m.page,
                    "page_size": m.page_size,
                    "sort": m.sort,
                }),
            ))
        }
        "RenderPhoto" => {
            let m = RenderPhotoReq::decode(raw)
                .map_err(|e| Status::invalid_argument(format!("decode error: {e}")))?;
            Ok((
                METHOD_PHOTOS_RENDER,
                serde_json::json!({
                    "scene": m.scene,
                    "width": m.width,
                    "height": m.height,
                    "format": m.format,
                }),
            ))
        }
        "CompareImage" => {
            let m = CompareImageReq::decode(raw)
                .map_err(|e| Status::invalid_argument(format!("decode error: {e}")))?;
            Ok((
                METHOD_IMGDEF_COMPARE,
                serde_json::json!({
                    "filename": m.filename,
                    "image": m.image,
                    "threshold": m.threshold,
                }),
            ))
        }
        _ => Err(Status::unimplemented("unknown op")),
    }
}

fn encode_grpc_resp(op: &str, val: &serde_json::Value) -> Result<Vec<u8>, Status> {
    match op {
        "SearchPosts" => {
            let items = val
                .get("items")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|it| PostMsg {
                    user_id: it
                        .get("userId")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    id: it
                        .get("id")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    title: it
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    body: it
                        .get("body")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
                .collect();
            Ok(SearchPostsResp {
                items,
                total: u32_field(val, "total"),
                page: u32_field(val, "page"),
                page_size: u32_field(val, "page_size"),
            }
            .encode_to_vec())
        }
        "RenderPhoto" => {
            let img = val.get("image").unwrap_or(&serde_json::Value::Null);
            let b64 = img
                .get("base64")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let raw_bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap_or_default();
            Ok(RenderPhotoResp {
                scene: str_field(val, "scene"),
                width: u32_field(val, "width"),
                height: u32_field(val, "height"),
                format: str_field(val, "format"),
                filename: str_field(img, "filename"),
                mime: str_field(img, "mime"),
                size: img
                    .get("size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                image_bytes: raw_bytes,
                image_base64: String::new(),
            }
            .encode_to_vec())
        }
        "CompareImage" => Ok(CompareImageResp {
            matched: val
                .get("matched")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            filename: str_field(val, "filename"),
            distance: u32_field(val, "distance"),
            threshold: u32_field(val, "threshold"),
        }
        .encode_to_vec()),
        _ => Err(Status::internal("unknown op")),
    }
}

fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn u32_field(v: &serde_json::Value, key: &str) -> u32 {
    v.get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

fn map_grpc_err(err: &CoreError) -> Status {
    let code = match err.kind {
        CoreErrorKind::BadRequest(_) | CoreErrorKind::Unprocessable(_) => Code::InvalidArgument,
        CoreErrorKind::UnknownMethod => Code::Unimplemented,
        CoreErrorKind::Forbidden => Code::PermissionDenied,
        CoreErrorKind::NotFound(_) => Code::NotFound,
        CoreErrorKind::Internal(_) => Code::Internal,
    };
    Status::new(code, err.message())
}
