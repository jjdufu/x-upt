use std::sync::Arc;

use async_graphql::ErrorExtensions;
use async_graphql::dynamic::{
    Enum, EnumItem, Field, FieldFuture, FieldValue, InputValue, Object, Schema, TypeRef,
};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::Router;
use axum::extract::{Extension, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::{get, post};

use crate::auth::oauth::{issue_token, token_success};
use crate::auth::token::TokenManager;
use crate::auth::{AuthPolicy, authenticate_request, policy_from};
use crate::config::{AppConfig, AuthMethod, apply_latency};
use crate::core::{Call, Core, dispatch_extra};
use crate::front::access::{AccessMeta, access_log};
use crate::front::util::{rest_auth_error, rest_error};
use crate::logger::context::LogContext;
use crate::types::gateway::{METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH};
use crate::types::token::{AuthedApp, TokenForm};

#[derive(Clone)]
struct GqlState {
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Option<Arc<TokenManager>>,
    policy: AuthPolicy,
    schema: Schema,
}

pub fn router(
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Option<Arc<TokenManager>>,
    meta: AccessMeta,
) -> Router {
    let schema = build_schema(Arc::clone(&core));
    let policy = policy_from(
        config.fronts.graphql.common.auth.clone(),
        tokens.clone(),
        Arc::clone(&config.apps),
        config.fronts.graphql.common.port,
    );
    let state = GqlState {
        config: Arc::clone(&config),
        core,
        tokens,
        policy,
        schema,
    };
    let mut open = Router::new()
        .route("/graphql", get(method_not_allowed))
        .route("/sdl", get(handle_sdl));
    if state.policy.allows(AuthMethod::OauthCc) {
        open = open.route("/oauth/token", post(handle_token));
    }
    let authed = Router::new()
        .route("/graphql", post(handle_gql))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_mw));
    open.merge(authed)
        .fallback(handle_not_found)
        .layer(middleware::from_fn(access_log))
        .layer(axum::Extension(meta))
        .with_state(state)
}

async fn method_not_allowed() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

async fn handle_not_found(Extension(ctx): Extension<LogContext>) -> Response {
    ctx.set_fail("not_found", "not found", serde_json::json!("not_found"));
    rest_error(
        StatusCode::NOT_FOUND,
        "not_found",
        "not found",
        &ctx.request_id(),
    )
}

async fn auth_mw(
    State(state): State<GqlState>,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let ctx = req.extensions().get::<LogContext>().cloned();
    let start = std::time::Instant::now();
    match authenticate_request(&state.policy, req, false).await {
        Ok((ok, mut req)) => {
            if let Some(ref ctx) = ctx {
                ctx.set_client_id(ok.app.client_id.clone());
                ctx.set_auth(ok.method.as_str());
                ctx.add_duration_auth(crate::logger::elapsed_ms(start));
            }
            req.extensions_mut().insert(ok.app);
            Ok(next.run(req).await)
        }
        Err(err) => {
            if let Some(ref ctx) = ctx {
                ctx.add_duration_auth(crate::logger::elapsed_ms(start));
            }
            Err(rest_auth_error(&ctx, &err))
        }
    }
}

async fn handle_token(
    Extension(ctx): Extension<LogContext>,
    State(state): State<GqlState>,
    headers: HeaderMap,
    axum::Form(form): axum::Form<TokenForm>,
) -> Response {
    ctx.set_auth("oauth_cc");
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let Some(tokens) = &state.tokens else {
        ctx.set_fail(
            "invalid_client",
            "oauth is not enabled",
            serde_json::json!("invalid_client"),
        );
        return crate::auth::oauth::oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "oauth is not enabled",
        );
    };
    match issue_token(
        &ctx,
        &state.config.apps,
        tokens,
        state.config.methods,
        &headers,
        &form,
    )
    .await
    {
        Ok(resp) => token_success(resp),
        Err(resp) => resp,
    }
}

async fn handle_sdl(State(state): State<GqlState>) -> impl IntoResponse {
    state.schema.sdl()
}

async fn handle_gql(
    Extension(ctx): Extension<LogContext>,
    Extension(app): Extension<AuthedApp>,
    State(state): State<GqlState>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let rid = ctx.request_id();
    let inner = req.into_inner();
    ctx.set_in(serde_json::json!({
        "query": inner.query,
        "operationName": inner.operation_name,
        "variables": serde_json::to_value(&inner.variables).unwrap_or(serde_json::Value::Null),
    }));
    let req = inner
        .data(Arc::clone(&state.core))
        .data(app)
        .data(ctx.clone());
    let mut response = state.schema.execute(req).await;
    ctx.collapse_single_extra();
    attach_request_id(&mut response, &rid);
    response.into()
}

fn attach_request_id(response: &mut async_graphql::Response, request_id: &str) {
    response.extensions.insert(
        "request_id".to_string(),
        async_graphql::Value::String(request_id.into()),
    );
}

fn build_schema(core: Arc<Core>) -> Schema {
    let post = Object::new("Post")
        .field(json_field(
            "userId",
            TypeRef::named_nn(TypeRef::INT),
            "userId",
        ))
        .field(json_field("id", TypeRef::named_nn(TypeRef::INT), "id"))
        .field(json_field(
            "title",
            TypeRef::named_nn(TypeRef::STRING),
            "title",
        ))
        .field(json_field(
            "body",
            TypeRef::named_nn(TypeRef::STRING),
            "body",
        ));
    let posts_result = Object::new("PostsResult")
        .field(json_field_list("items", "Post"))
        .field(json_field(
            "total",
            TypeRef::named_nn(TypeRef::INT),
            "total",
        ))
        .field(json_field("page", TypeRef::named_nn(TypeRef::INT), "page"))
        .field(json_field(
            "pageSize",
            TypeRef::named_nn(TypeRef::INT),
            "page_size",
        ));
    let image = Object::new("ImageInfo")
        .field(json_field(
            "filename",
            TypeRef::named_nn(TypeRef::STRING),
            "filename",
        ))
        .field(json_field(
            "mime",
            TypeRef::named_nn(TypeRef::STRING),
            "mime",
        ))
        .field(json_field("size", TypeRef::named_nn(TypeRef::INT), "size"))
        .field(json_field(
            "base64",
            TypeRef::named_nn(TypeRef::STRING),
            "base64",
        ));
    let photos_result = Object::new("PhotosResult")
        .field(json_field(
            "scene",
            TypeRef::named_nn(TypeRef::STRING),
            "scene",
        ))
        .field(json_field(
            "width",
            TypeRef::named_nn(TypeRef::INT),
            "width",
        ))
        .field(json_field(
            "height",
            TypeRef::named_nn(TypeRef::INT),
            "height",
        ))
        .field(json_field(
            "format",
            TypeRef::named_nn(TypeRef::STRING),
            "format",
        ))
        .field(Field::new("image", TypeRef::named_nn("ImageInfo"), |ctx| {
            FieldFuture::new(async move {
                let v = ctx.parent_value.try_downcast_ref::<serde_json::Value>()?;
                Ok(Some(FieldValue::owned_any(v["image"].clone())))
            })
        }));
    let imgdef_result = Object::new("ImgdefResult")
        .field(json_field(
            "matched",
            TypeRef::named_nn(TypeRef::BOOLEAN),
            "matched",
        ))
        .field(json_field(
            "filename",
            TypeRef::named_nn(TypeRef::STRING),
            "filename",
        ))
        .field(json_field(
            "distance",
            TypeRef::named_nn(TypeRef::INT),
            "distance",
        ))
        .field(json_field(
            "threshold",
            TypeRef::named_nn(TypeRef::INT),
            "threshold",
        ));

    let sort = Enum::new("PostSort")
        .item(EnumItem::new("ID_ASC"))
        .item(EnumItem::new("ID_DESC"))
        .item(EnumItem::new("TITLE_ASC"))
        .item(EnumItem::new("TITLE_DESC"));
    let scene = Enum::new("PhotoScene")
        .item(EnumItem::new("ARTICLE_COVER"))
        .item(EnumItem::new("AVATAR"))
        .item(EnumItem::new("BANNER"));
    let format = Enum::new("PhotoFormat")
        .item(EnumItem::new("PNG"))
        .item(EnumItem::new("JPEG"))
        .item(EnumItem::new("GIF"));

    let core_q = Arc::clone(&core);
    let query = Object::new("Query").field(
        Field::new(
            "postsSearch",
            TypeRef::named_nn("PostsResult"),
            move |ctx| {
                let core = Arc::clone(&core_q);
                FieldFuture::new(async move {
                    let app = ctx.data::<AuthedApp>()?.clone();
                    let log = ctx.data::<LogContext>()?.clone();
                    let keyword = ctx.args.try_get("keyword")?.string()?.to_string();
                    let user_id = ctx.args.try_get("userId")?.u64()?;
                    let page = u32::try_from(ctx.args.try_get("page")?.u64()?).unwrap_or(0);
                    let page_size =
                        u32::try_from(ctx.args.try_get("pageSize")?.u64()?).unwrap_or(0);
                    let sort = gql_sort(ctx.args.try_get("sort")?.enum_name()?);
                    let biz = serde_json::json!({
                        "keyword": keyword, "user_id": user_id, "page": page,
                        "page_size": page_size, "sort": sort,
                    });
                    dispatch_gql(&core, &log, &app, METHOD_POSTS_SEARCH, biz)
                })
            },
        )
        .argument(InputValue::new(
            "keyword",
            TypeRef::named_nn(TypeRef::STRING),
        ))
        .argument(InputValue::new("userId", TypeRef::named_nn(TypeRef::INT)))
        .argument(InputValue::new("page", TypeRef::named_nn(TypeRef::INT)))
        .argument(InputValue::new("pageSize", TypeRef::named_nn(TypeRef::INT)))
        .argument(InputValue::new("sort", TypeRef::named_nn("PostSort"))),
    );

    let core_p = Arc::clone(&core);
    let core_i = Arc::clone(&core);
    let mutation = Object::new("Mutation")
        .field(
            Field::new(
                "photosRender",
                TypeRef::named_nn("PhotosResult"),
                move |ctx| {
                    let core = Arc::clone(&core_p);
                    FieldFuture::new(async move {
                        let app = ctx.data::<AuthedApp>()?.clone();
                        let log = ctx.data::<LogContext>()?.clone();
                        let scene = gql_scene(ctx.args.try_get("scene")?.enum_name()?);
                        let width = u32::try_from(ctx.args.try_get("width")?.u64()?).unwrap_or(0);
                        let height = u32::try_from(ctx.args.try_get("height")?.u64()?).unwrap_or(0);
                        let format = gql_format(ctx.args.try_get("format")?.enum_name()?);
                        let biz = serde_json::json!({
                            "scene": scene, "width": width, "height": height, "format": format,
                        });
                        dispatch_gql(&core, &log, &app, METHOD_PHOTOS_RENDER, biz)
                    })
                },
            )
            .argument(InputValue::new("scene", TypeRef::named_nn("PhotoScene")))
            .argument(InputValue::new("width", TypeRef::named_nn(TypeRef::INT)))
            .argument(InputValue::new("height", TypeRef::named_nn(TypeRef::INT)))
            .argument(InputValue::new("format", TypeRef::named_nn("PhotoFormat"))),
        )
        .field(
            Field::new(
                "imgdefCompare",
                TypeRef::named_nn("ImgdefResult"),
                move |ctx| {
                    let core = Arc::clone(&core_i);
                    FieldFuture::new(async move {
                        let app = ctx.data::<AuthedApp>()?.clone();
                        let log = ctx.data::<LogContext>()?.clone();
                        let filename = ctx.args.try_get("filename")?.string()?.to_string();
                        let image = ctx.args.try_get("image")?.string()?.to_string();
                        let threshold =
                            u32::try_from(ctx.args.try_get("threshold")?.u64()?).unwrap_or(0);
                        let biz = serde_json::json!({
                            "filename": filename, "image": image, "threshold": threshold,
                        });
                        dispatch_gql(&core, &log, &app, METHOD_IMGDEF_COMPARE, biz)
                    })
                },
            )
            .argument(InputValue::new(
                "filename",
                TypeRef::named_nn(TypeRef::STRING),
            ))
            .argument(InputValue::new("image", TypeRef::named_nn(TypeRef::STRING)))
            .argument(InputValue::new(
                "threshold",
                TypeRef::named_nn(TypeRef::INT),
            )),
        );

    Schema::build(query.type_name(), Some(mutation.type_name()), None)
        .register(post)
        .register(posts_result)
        .register(image)
        .register(photos_result)
        .register(imgdef_result)
        .register(sort)
        .register(scene)
        .register(format)
        .register(query)
        .register(mutation)
        .finish()
        .expect("graphql schema")
}

fn json_field(name: &'static str, ty: TypeRef, json_key: &'static str) -> Field {
    Field::new(name, ty, move |ctx| {
        FieldFuture::new(async move {
            let v = ctx.parent_value.try_downcast_ref::<serde_json::Value>()?;
            Ok(Some(
                async_graphql::Value::from_json(
                    v.get(json_key).cloned().unwrap_or(serde_json::Value::Null),
                )
                .unwrap_or(async_graphql::Value::Null),
            ))
        })
    })
}

fn json_field_list(name: &'static str, ty: &'static str) -> Field {
    Field::new(name, TypeRef::named_nn_list_nn(ty), |ctx| {
        FieldFuture::new(async move {
            let v = ctx.parent_value.try_downcast_ref::<serde_json::Value>()?;
            let items = v
                .get("items")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let values = items
                .into_iter()
                .map(FieldValue::owned_any)
                .collect::<Vec<_>>();
            Ok(Some(FieldValue::list(values)))
        })
    })
}

fn dispatch_gql(
    core: &Core,
    ctx: &LogContext,
    app: &AuthedApp,
    method: &str,
    biz: serde_json::Value,
) -> Result<Option<FieldValue<'static>>, async_graphql::Error> {
    let call = Call::from_app(app, method.to_string(), biz);
    match dispatch_extra(core, ctx, call) {
        Ok(value) => Ok(Some(FieldValue::owned_any(value))),
        Err(err) => {
            let (_, code) = crate::front::util::rest_short(&err);
            ctx.set_last_extra_code(serde_json::json!(code));
            Err(
                async_graphql::Error::new(err.message().to_string()).extend_with(|_, e| {
                    e.set("code", code);
                }),
            )
        }
    }
}

fn gql_sort(name: &str) -> &'static str {
    match name {
        "ID_ASC" => "id_asc",
        "ID_DESC" => "id_desc",
        "TITLE_ASC" => "title_asc",
        _ => "title_desc",
    }
}

fn gql_scene(name: &str) -> &'static str {
    match name {
        "ARTICLE_COVER" => "article_cover",
        "BANNER" => "banner",
        _ => "avatar",
    }
}

fn gql_format(name: &str) -> &'static str {
    match name {
        "JPEG" => "jpeg",
        "GIF" => "gif",
        _ => "png",
    }
}
