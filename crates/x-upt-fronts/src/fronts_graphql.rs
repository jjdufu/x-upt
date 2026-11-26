//! Dynamic `async-graphql` schema, SDL exporter, and router for the `graphql` front.

use std::sync::Arc;
use std::time::Instant;

use async_graphql::dynamic::{
    Enum, EnumItem, Field, FieldFuture, FieldValue, Object, Schema, TypeRef,
};
use async_graphql::{ErrorExtensions, Value as GqlValue};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, OriginalUri, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use http::{HeaderMap, StatusCode, header};
use x_upt_core::domain::{
    AuthedApp, METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH,
};
use x_upt_core::services::Call;
use x_upt_store::dto::RuntimeSnapshotData;

use crate::fronts_gateway::oauth_token_ep;
use crate::fronts_rest_rpc::{rest_auth_fail, rest_error, rest_not_found};
use crate::http_guard::{FrontCtx, attach_std_headers, verify_http_request};
use crate::logger::{LogOutcome, ReqMeta, build_log_entry};
use crate::snapshot::simulate_latency;

#[derive(Clone)]
struct GqlExecCtx {
    snap: Arc<RuntimeSnapshotData>,
    app: AuthedApp,
}

/// Builds the `graphql` front `axum::Router`.
pub fn build_graphql_router(ctx: FrontCtx) -> Router {
    Router::new()
        .route("/oauth/token", post(oauth_token_ep))
        .route("/sdl", get(graphql_sdl_ep))
        .route("/graphql", post(graphql_post_ep).get(graphql_get_405))
        .fallback(rest_not_found)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(ctx)
}

/// Exports the exact same-source GraphQL SDL string used by the live `/graphql` executor.
///
/// # Errors
/// Returns an error string if dynamic schema construction fails.
pub fn export_graphql_sdl() -> Result<String, String> {
    build_dynamic_schema().map(|s| s.sdl())
}

fn build_dynamic_schema() -> Result<Schema, String> {
    let sort_enum = Enum::new("PostSort").items([
        EnumItem::new("ID_ASC"),
        EnumItem::new("ID_DESC"),
        EnumItem::new("TITLE_ASC"),
        EnumItem::new("TITLE_DESC"),
    ]);
    let scene_enum = Enum::new("PhotoScene").items([
        EnumItem::new("ARTICLE_COVER"),
        EnumItem::new("AVATAR"),
        EnumItem::new("BANNER"),
    ]);
    let fmt_enum = Enum::new("PhotoFormat").items([
        EnumItem::new("PNG"),
        EnumItem::new("JPEG"),
        EnumItem::new("GIF"),
    ]);

    let post_obj = build_post_objects();
    let photo_objs = build_photo_objects();
    let imgdef_obj = build_imgdef_object();

    let query = Object::new("Query").field(build_posts_field());
    let mutation = Object::new("Mutation")
        .field(build_photos_field())
        .field(build_imgdef_field());

    let mut builder = Schema::build("Query", Some("Mutation"), None)
        .register(sort_enum)
        .register(scene_enum)
        .register(fmt_enum)
        .register(imgdef_obj)
        .register(query)
        .register(mutation);
    for o in post_obj {
        builder = builder.register(o);
    }
    for o in photo_objs {
        builder = builder.register(o);
    }
    builder
        .finish()
        .map_err(|e| format!("graphql schema build error: {e}"))
}

fn build_post_objects() -> [Object; 2] {
    let post = Object::new("Post")
        .field(json_int_field("userId", "userId"))
        .field(json_int_field("id", "id"))
        .field(json_str_field("title", "title"))
        .field(json_str_field("body", "body"));
    let res = Object::new("PostsResult")
        .field(Field::new(
            "items",
            TypeRef::named_nn_list_nn("Post"),
            |ctx| {
                FieldFuture::new(async move {
                    let val = ctx.parent_value.try_downcast_ref::<serde_json::Value>()?;
                    let arr = val
                        .get("items")
                        .and_then(serde_json::Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let list: Vec<FieldValue<'_>> =
                        arr.into_iter().map(FieldValue::owned_any).collect();
                    Ok(Some(FieldValue::list(list)))
                })
            },
        ))
        .field(json_int_field("total", "total"))
        .field(json_int_field("page", "page"))
        .field(json_int_field("pageSize", "page_size"));
    [post, res]
}

fn build_photo_objects() -> [Object; 2] {
    let info = Object::new("ImageInfo")
        .field(json_str_field("filename", "filename"))
        .field(json_str_field("mime", "mime"))
        .field(json_int_field("size", "size"))
        .field(json_str_field("base64", "base64"));
    let res = Object::new("PhotosResult")
        .field(json_str_field("scene", "scene"))
        .field(json_int_field("width", "width"))
        .field(json_int_field("height", "height"))
        .field(json_str_field("format", "format"))
        .field(Field::new("image", TypeRef::named_nn("ImageInfo"), |ctx| {
            FieldFuture::new(async move {
                let val = ctx.parent_value.try_downcast_ref::<serde_json::Value>()?;
                let img = val.get("image").cloned().unwrap_or(serde_json::Value::Null);
                Ok(Some(FieldValue::owned_any(img)))
            })
        }));
    [info, res]
}

fn build_imgdef_object() -> Object {
    Object::new("ImgdefResult")
        .field(Field::new(
            "matched",
            TypeRef::named_nn(TypeRef::BOOLEAN),
            |ctx| {
                FieldFuture::new(async move {
                    let val = ctx.parent_value.try_downcast_ref::<serde_json::Value>()?;
                    let b = val
                        .get("matched")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    Ok(Some(GqlValue::Boolean(b)))
                })
            },
        ))
        .field(json_str_field("filename", "filename"))
        .field(json_int_field("distance", "distance"))
        .field(json_int_field("threshold", "threshold"))
}

fn json_str_field(gql_name: &str, json_key: &'static str) -> Field {
    Field::new(gql_name, TypeRef::named_nn(TypeRef::STRING), move |ctx| {
        FieldFuture::new(async move {
            let val = ctx.parent_value.try_downcast_ref::<serde_json::Value>()?;
            let s = val
                .get(json_key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            Ok(Some(GqlValue::String(s)))
        })
    })
}

fn json_int_field(gql_name: &str, json_key: &'static str) -> Field {
    Field::new(gql_name, TypeRef::named_nn(TypeRef::INT), move |ctx| {
        FieldFuture::new(async move {
            let val = ctx.parent_value.try_downcast_ref::<serde_json::Value>()?;
            let n = val
                .get(json_key)
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            Ok(Some(GqlValue::from(n)))
        })
    })
}

fn build_posts_field() -> Field {
    use async_graphql::dynamic::InputValue;
    Field::new("postsSearch", TypeRef::named_nn("PostsResult"), |ctx| {
        FieldFuture::new(async move {
            let exec = ctx.data::<GqlExecCtx>()?;
            let keyword = ctx.args.try_get("keyword")?.string()?.to_string();
            let user_id = ctx.args.try_get("userId")?.i64()?;
            let page = ctx.args.try_get("page")?.i64()?;
            let page_size = ctx.args.try_get("pageSize")?.i64()?;
            let sort = ctx.args.try_get("sort")?.enum_name()?.to_ascii_lowercase();
            let biz = serde_json::json!({
                "keyword": keyword,
                "user_id": user_id,
                "page": page,
                "page_size": page_size,
                "sort": sort,
            });
            run_gql_call(exec, METHOD_POSTS_SEARCH, biz)
        })
    })
    .argument(InputValue::new(
        "keyword",
        TypeRef::named_nn(TypeRef::STRING),
    ))
    .argument(InputValue::new("userId", TypeRef::named_nn(TypeRef::INT)))
    .argument(InputValue::new("page", TypeRef::named_nn(TypeRef::INT)))
    .argument(InputValue::new("pageSize", TypeRef::named_nn(TypeRef::INT)))
    .argument(InputValue::new("sort", TypeRef::named_nn("PostSort")))
}

fn build_photos_field() -> Field {
    use async_graphql::dynamic::InputValue;
    Field::new("photosRender", TypeRef::named_nn("PhotosResult"), |ctx| {
        FieldFuture::new(async move {
            let exec = ctx.data::<GqlExecCtx>()?;
            let scene = ctx.args.try_get("scene")?.enum_name()?.to_ascii_lowercase();
            let width = ctx.args.try_get("width")?.i64()?;
            let height = ctx.args.try_get("height")?.i64()?;
            let format = ctx
                .args
                .try_get("format")?
                .enum_name()?
                .to_ascii_lowercase();
            let biz = serde_json::json!({
                "scene": scene,
                "width": width,
                "height": height,
                "format": format,
            });
            run_gql_call(exec, METHOD_PHOTOS_RENDER, biz)
        })
    })
    .argument(InputValue::new("scene", TypeRef::named_nn("PhotoScene")))
    .argument(InputValue::new("width", TypeRef::named_nn(TypeRef::INT)))
    .argument(InputValue::new("height", TypeRef::named_nn(TypeRef::INT)))
    .argument(InputValue::new("format", TypeRef::named_nn("PhotoFormat")))
}

fn build_imgdef_field() -> Field {
    use async_graphql::dynamic::InputValue;
    Field::new("imgdefCompare", TypeRef::named_nn("ImgdefResult"), |ctx| {
        FieldFuture::new(async move {
            let exec = ctx.data::<GqlExecCtx>()?;
            let filename = ctx.args.try_get("filename")?.string()?.to_string();
            let image = ctx.args.try_get("image")?.string()?.to_string();
            let threshold = ctx.args.try_get("threshold")?.i64()?;
            let biz = serde_json::json!({
                "filename": filename,
                "image": image,
                "threshold": threshold,
            });
            run_gql_call(exec, METHOD_IMGDEF_COMPARE, biz)
        })
    })
    .argument(InputValue::new(
        "filename",
        TypeRef::named_nn(TypeRef::STRING),
    ))
    .argument(InputValue::new("image", TypeRef::named_nn(TypeRef::STRING)))
    .argument(InputValue::new(
        "threshold",
        TypeRef::named_nn(TypeRef::INT),
    ))
}

fn run_gql_call<'a>(
    exec: &GqlExecCtx,
    method: &str,
    biz: serde_json::Value,
) -> Result<Option<FieldValue<'a>>, async_graphql::Error> {
    match exec
        .snap
        .dataset
        .dispatch(Call::from_app(&exec.app, method.to_string(), biz))
    {
        Ok(val) => Ok(Some(FieldValue::owned_any(val))),
        Err(err) => {
            let code = err.kind.as_str().to_string();
            Err(async_graphql::Error::new(err.message())
                .extend_with(|_, ext| ext.set("code", code)))
        }
    }
}

async fn graphql_sdl_ep(
    State(ctx): State<FrontCtx>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let meta = ReqMeta::from_http("GET", uri.path(), uri.query(), &headers, None, Some(0));
    let req_id = meta.request_id.clone();
    let sdl = export_graphql_sdl().unwrap_or_default();
    let mut resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        sdl,
    )
        .into_response();
    attach_std_headers(&mut resp, &req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 200,
            ..LogOutcome::default()
        },
    ));
    resp
}

async fn graphql_get_405(
    State(ctx): State<FrontCtx>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let meta = ReqMeta::from_http("GET", uri.path(), uri.query(), &headers, None, Some(0));
    let req_id = meta.request_id.clone();
    rest_error(
        &ctx,
        meta,
        &req_id,
        StatusCode::METHOD_NOT_ALLOWED,
        "bad_request",
        "GET /graphql not allowed",
        None,
    )
}

async fn graphql_post_ep(
    State(ctx): State<FrontCtx>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let meta = ReqMeta::from_http(
        "POST",
        uri.path(),
        uri.query(),
        &headers,
        None,
        Some(body.len()),
    );
    let req_id = meta.request_id.clone();
    let snap = ctx.runtime.load();
    let caller = match verify_http_request(
        &ctx,
        &snap,
        "POST",
        uri.path(),
        uri.query(),
        &headers,
        &body,
    ) {
        Ok(c) => c,
        Err(f) => return rest_auth_fail(&ctx, meta, &req_id, f.error, f.duration_ms),
    };
    let gql_req: async_graphql::Request = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return rest_error(
                &ctx,
                meta,
                &req_id,
                StatusCode::BAD_REQUEST,
                "bad_request",
                &format!("invalid GraphQL JSON: {e}"),
                Some(&caller.app.client_id),
            );
        }
    };
    let Ok(schema) = build_dynamic_schema() else {
        return rest_error(
            &ctx,
            meta,
            &req_id,
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "schema error",
            Some(&caller.app.client_id),
        );
    };

    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let exec_ctx = GqlExecCtx {
        snap: Arc::clone(&snap),
        app: caller.app.clone(),
    };
    let mut gql_resp = schema.execute(gql_req.data(exec_ctx)).await;
    gql_resp
        .extensions
        .insert("request_id".to_string(), GqlValue::String(req_id.clone()));
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);
    let out_json = serde_json::to_value(&gql_resp).unwrap_or(serde_json::Value::Null);
    let in_json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);

    let mut resp = (StatusCode::OK, axum::Json(out_json.clone())).into_response();
    attach_std_headers(&mut resp, &req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 200,
            duration_auth: Some(caller.duration_ms),
            duration_core: Some(core_ms),
            duration_latency: Some(lat_ms),
            client_id: Some(caller.app.client_id),
            auth: Some(caller.method.as_str().to_string()),
            rpc: Some("graphql".to_string()),
            input: in_json,
            out: out_json,
            ..LogOutcome::default()
        },
    ));
    resp
}
