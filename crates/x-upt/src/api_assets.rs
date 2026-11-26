//! Control-plane asset import (`images`, `posts`, `apps`, `envelopes`), live docs, contracts, and logs handlers.

use axum::Json;
use axum::extract::{Multipart, Path, Query, Request, State};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::Deserialize;
use x_upt_core::docs_model::build_front_docs;
use x_upt_core::domain::{EnvelopeProfileSpec, FrontName, FrontRecord};
use x_upt_core::error::AppError;
use x_upt_core::imgs::{ImageMeta, PhotoScene};
use x_upt_core::services::Post;
use x_upt_fronts::contracts::{graphql_sdl, grpc_fds, grpc_proto, soap_wsdl, upstream_openapi};
use x_upt_fronts::logger::LogQuery;
use x_upt_store::dto::{
    AppMetaView, AppUpsertPayload, EnvelopeProfilesView, PostInput, PostsPageView,
    SecretUpsertPayload,
};

use crate::api_control::parse_front_name;
use crate::state::{AdminState, ApiError};

const ADMIN_OPENAPI_RAW: &str = include_str!("../admin-openapi.json");

#[derive(Deserialize, Default)]
pub struct PostsQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
}

#[derive(Deserialize)]
struct JsonImageItem {
    scene: String,
    filename: String,
    base64: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JsonImagesPayload {
    Single(JsonImageItem),
    Batch { items: Vec<JsonImageItem> },
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum PostsImportBody {
    Wrapped { items: Vec<PostInput> },
    Bare(Vec<PostInput>),
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum ProfilesReplaceBody {
    Wrapped { profiles: Vec<EnvelopeProfileSpec> },
    Bare(Vec<EnvelopeProfileSpec>),
}

#[derive(Deserialize, Default)]
pub struct ContractQuery {
    pub download: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct RecentLogsQuery {
    pub front: Option<String>,
    pub status: Option<String>,
    pub request_id: Option<String>,
    pub limit: Option<usize>,
}

/// `GET /api/v1/images` handler.
pub async fn list_images(
    State(state): State<AdminState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let items = state.store().list_images()?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// `POST /api/v1/images` handler supporting both `multipart/form-data` and `application/json`.
pub async fn import_images(
    State(state): State<AdminState>,
    headers: HeaderMap,
    req: Request,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let imported = if ct.starts_with("multipart/form-data") {
        import_images_multipart(&state, req).await?
    } else {
        let bytes = axum::body::to_bytes(req.into_body(), 32 * 1024 * 1024)
            .await
            .map_err(|e| AppError::InvalidInput(format!("read image body failed: {e}")))?;
        import_images_json(&state, &bytes)?
    };

    state.refresh_snapshot()?;
    Ok(Json(serde_json::json!({ "items": imported })))
}

async fn import_images_multipart(
    state: &AdminState,
    req: Request,
) -> Result<Vec<ImageMeta>, AppError> {
    let mut mp = <Multipart as axum::extract::FromRequest<()>>::from_request(req, &())
        .await
        .map_err(|e| AppError::InvalidInput(format!("invalid multipart body: {e}")))?;

    let mut scene = PhotoScene::ArticleCover;
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|e| AppError::InvalidInput(format!("multipart field error: {e}")))?
    {
        let fname_opt = field.file_name().map(ToString::to_string);
        let field_name = field.name().unwrap_or("").to_string();
        if field_name == "scene" && fname_opt.is_none() {
            let txt = field
                .text()
                .await
                .map_err(|e| AppError::InvalidInput(format!("invalid scene text: {e}")))?;
            scene = PhotoScene::parse(&txt).map_err(AppError::InvalidInput)?;
        } else if fname_opt.is_some() || field_name == "file" || field_name == "files" {
            let name = fname_opt.unwrap_or_else(|| "upload.png".to_string());
            let data = field
                .bytes()
                .await
                .map_err(|e| AppError::InvalidInput(format!("file read error: {e}")))?;
            files.push((name, data.to_vec()));
        }
    }

    if files.is_empty() {
        return Err(AppError::InvalidInput("未检测到上传的图片文件".to_string()));
    }
    let mut out = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        out.push(state.store().import_image(scene, &name, &bytes)?);
    }
    Ok(out)
}

fn import_images_json(state: &AdminState, body: &[u8]) -> Result<Vec<ImageMeta>, AppError> {
    let payload: JsonImagesPayload = serde_json::from_slice(body)
        .map_err(|e| AppError::InvalidInput(format!("invalid JSON image payload: {e}")))?;
    let list = match payload {
        JsonImagesPayload::Single(item) => vec![item],
        JsonImagesPayload::Batch { items } => items,
    };
    if list.is_empty() {
        return Err(AppError::InvalidInput("图片导入列表不能为空".to_string()));
    }
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let scene = PhotoScene::parse(&item.scene).map_err(AppError::InvalidInput)?;
        let raw_b64 = item
            .base64
            .trim()
            .split_once(";base64,")
            .map_or(item.base64.trim(), |(_, r)| r);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw_b64)
            .map_err(|e| AppError::InvalidInput(format!("invalid base64 image: {e}")))?;
        out.push(state.store().import_image(scene, &item.filename, &bytes)?);
    }
    Ok(out)
}

/// `GET /api/v1/images/{id}/raw` handler.
pub async fn get_image_raw(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let (meta, bytes) = state.store().get_image_raw(&id)?;
    let mut resp = (StatusCode::OK, bytes).into_response();
    if let Ok(hv) = HeaderValue::from_str(&meta.mime) {
        resp.headers_mut().insert(header::CONTENT_TYPE, hv);
    }
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(resp)
}

/// `DELETE /api/v1/images/{id}` handler.
pub async fn delete_image(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.store().delete_image(&id)?;
    state.refresh_snapshot()?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /api/v1/posts` handler.
pub async fn list_posts(
    State(state): State<AdminState>,
    Query(q): Query<PostsQuery>,
) -> Result<Json<PostsPageView>, ApiError> {
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(50);
    let kw = q.keyword.unwrap_or_default();
    Ok(Json(state.store().list_posts(page, page_size, &kw)?))
}

/// `POST /api/v1/posts` handler.
pub async fn create_post(
    State(state): State<AdminState>,
    Json(input): Json<PostInput>,
) -> Result<Json<Post>, ApiError> {
    let saved = state.store().create_post(input)?;
    state.refresh_snapshot()?;
    Ok(Json(saved))
}

/// `POST /api/v1/posts/import` handler.
pub async fn import_posts(
    State(state): State<AdminState>,
    Json(body): Json<PostsImportBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let items = match body {
        PostsImportBody::Wrapped { items } | PostsImportBody::Bare(items) => items,
    };
    let count = state.store().import_posts(items)?;
    state.refresh_snapshot()?;
    Ok(Json(serde_json::json!({ "imported": count })))
}

/// `PUT /api/v1/posts/{id}` handler.
pub async fn update_post(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Json(input): Json<PostInput>,
) -> Result<Json<Post>, ApiError> {
    let saved = state.store().update_post(id, input)?;
    state.refresh_snapshot()?;
    Ok(Json(saved))
}

/// `DELETE /api/v1/posts/{id}` handler.
pub async fn delete_post(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.store().delete_post(id)?;
    state.refresh_snapshot()?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /api/v1/apps` handler.
pub async fn list_apps(
    State(state): State<AdminState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let items = state.store().list_apps()?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// `POST /api/v1/apps` handler.
pub async fn create_app(
    State(state): State<AdminState>,
    Json(payload): Json<AppUpsertPayload>,
) -> Result<Json<AppMetaView>, ApiError> {
    let meta = state.store().save_app(None, payload)?;
    state.refresh_snapshot()?;
    Ok(Json(meta))
}

/// `PUT /api/v1/apps/{client_id}` handler.
pub async fn update_app(
    State(state): State<AdminState>,
    Path(client_id): Path<String>,
    Json(payload): Json<AppUpsertPayload>,
) -> Result<Json<AppMetaView>, ApiError> {
    let meta = state.store().save_app(Some(&client_id), payload)?;
    state.refresh_snapshot()?;
    Ok(Json(meta))
}

/// `DELETE /api/v1/apps/{client_id}` handler.
pub async fn delete_app(
    State(state): State<AdminState>,
    Path(client_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.store().delete_app(&client_id)?;
    state.refresh_snapshot()?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /api/v1/envelopes/secrets` handler.
pub async fn list_envelope_secrets(
    State(state): State<AdminState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let items = state.store().list_secrets()?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// `POST /api/v1/envelopes/secrets` handler.
pub async fn upsert_envelope_secret(
    State(state): State<AdminState>,
    Json(payload): Json<SecretUpsertPayload>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let items = state.store().upsert_secret(payload)?;
    state.refresh_snapshot()?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// `DELETE /api/v1/envelopes/secrets/{name}` handler.
pub async fn delete_envelope_secret(
    State(state): State<AdminState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.store().delete_secret(&name)?;
    state.refresh_snapshot()?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// `GET /api/v1/envelopes/{front}/profiles` handler.
pub async fn get_envelope_profiles(
    State(state): State<AdminState>,
    Path(raw_front): Path<String>,
) -> Result<Json<EnvelopeProfilesView>, ApiError> {
    let front = parse_front_name(&raw_front)?;
    Ok(Json(state.store().get_profiles(front)?))
}

/// `PUT /api/v1/envelopes/{front}/profiles` handler.
pub async fn replace_envelope_profiles(
    State(state): State<AdminState>,
    Path(raw_front): Path<String>,
    Json(body): Json<ProfilesReplaceBody>,
) -> Result<Json<EnvelopeProfilesView>, ApiError> {
    let front = parse_front_name(&raw_front)?;
    let profiles = match body {
        ProfilesReplaceBody::Wrapped { profiles } | ProfilesReplaceBody::Bare(profiles) => profiles,
    };
    let view = state.store().replace_profiles(front, profiles)?;
    state.refresh_snapshot()?;
    Ok(Json(view))
}

/// `GET /api/v1/docs/fronts` handler.
pub async fn get_front_docs(
    State(state): State<AdminState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snap = state.supervisor().runtime().load();
    let items = build_front_docs(&snap.front_list, &snap.profiles_by_front);
    Ok(Json(serde_json::json!({ "items": items })))
}

/// `GET /api/v1/contracts/{name}` handler.
pub async fn download_contract(
    State(state): State<AdminState>,
    Path(name): Path<String>,
    Query(q): Query<ContractQuery>,
) -> Result<Response, ApiError> {
    let snap = state.supervisor().runtime().load();
    let (content_type, filename, bytes): (&'static str, &'static str, Vec<u8>) = match name.as_str()
    {
        "grpc.proto" => (
            "text/plain; charset=utf-8",
            "x_upt.proto",
            grpc_proto().as_bytes().to_vec(),
        ),
        "grpc.fds" => ("application/octet-stream", "x_upt.fds", grpc_fds().to_vec()),
        "soap.wsdl" => {
            let soap_rec = snap
                .fronts
                .get(&FrontName::Soap)
                .cloned()
                .unwrap_or_else(|| FrontRecord::default_for(FrontName::Soap));
            (
                "application/xml; charset=utf-8",
                "x_upt.wsdl",
                soap_wsdl(&soap_rec).into_bytes(),
            )
        }
        "graphql.sdl" => {
            let sdl = graphql_sdl().map_err(AppError::Internal)?;
            (
                "text/plain; charset=utf-8",
                "schema.graphql",
                sdl.into_bytes(),
            )
        }
        "upstream-openapi.json" => {
            let json = serde_json::to_vec_pretty(&upstream_openapi(&snap))
                .map_err(|e| AppError::Internal(format!("openapi serialize failed: {e}")))?;
            (
                "application/json; charset=utf-8",
                "upstream-openapi.json",
                json,
            )
        }
        "admin-openapi.json" => (
            "application/json; charset=utf-8",
            "admin-openapi.json",
            ADMIN_OPENAPI_RAW.as_bytes().to_vec(),
        ),
        other => {
            return Err(ApiError(AppError::NotFound(format!(
                "未知契约名称: {other}"
            ))));
        }
    };

    let mut resp = (StatusCode::OK, bytes).into_response();
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if q.download.as_deref() == Some("1")
        && let Ok(hv) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
    {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, hv);
    }
    Ok(resp)
}

/// `GET /api/v1/logs/recent` handler.
pub async fn get_recent_logs(
    State(state): State<AdminState>,
    Query(q): Query<RecentLogsQuery>,
) -> Json<serde_json::Value> {
    let items = state.supervisor().logger().query_recent(&LogQuery {
        front: q.front,
        status: q.status,
        request_id: q.request_id,
        limit: q.limit.unwrap_or(100),
    });
    Json(serde_json::json!({ "items": items }))
}

/// `GET /api/v1/jobs` handler.
pub async fn list_jobs(State(state): State<AdminState>) -> Json<serde_json::Value> {
    let snap = state.supervisor().runtime().load();
    let items = state
        .supervisor()
        .jobs()
        .list_recent(snap.settings.job_expire_seconds);
    Json(serde_json::json!({ "items": items }))
}
