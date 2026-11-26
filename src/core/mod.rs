use serde::de::DeserializeOwned;

use crate::config::MethodsConfig;
use crate::logger::context::{ExtraCall, LogContext};
use crate::service::{ServiceError, imgdef, photos, posts};
use crate::types::gateway::{
    METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH, is_known_method,
};
use crate::types::imgdef::ImgdefRequest;
use crate::types::photos::PhotosRequest;
use crate::types::posts::PostsRequest;
use crate::types::token::AuthedApp;

#[derive(Clone)]
pub struct Core {
    pub imgs_dir: String,
    pub methods: MethodsConfig,
}

pub struct Call {
    pub client_id: String,
    pub allowed_methods: std::collections::HashSet<String>,
    pub method: String,
    pub biz_content: serde_json::Value,
}

impl Call {
    #[must_use]
    pub fn from_app(app: &AuthedApp, method: String, biz_content: serde_json::Value) -> Self {
        Self {
            client_id: app.client_id.clone(),
            allowed_methods: app.methods.clone(),
            method,
            biz_content,
        }
    }
}

pub struct DispatchOk {
    pub value: serde_json::Value,
}

#[derive(Clone)]
pub struct CoreError {
    pub kind: CoreErrorKind,
}

#[derive(Clone)]
pub enum CoreErrorKind {
    BadRequest(String),
    UnknownMethod,
    Forbidden,
    Unprocessable(String),
    NotFound(String),
    Internal(String),
}

impl CoreErrorKind {
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

impl CoreError {
    fn new(kind: CoreErrorKind) -> Self {
        Self { kind }
    }

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

impl Core {
    pub fn dispatch(&self, call: Call) -> Result<DispatchOk, CoreError> {
        let method = call.method.trim().to_string();

        if method.is_empty() {
            return Err(CoreError::new(CoreErrorKind::BadRequest(
                "method must be non-empty".to_string(),
            )));
        }
        if !is_known_method(&method) || !self.methods.is_enabled(&method) {
            return Err(CoreError::new(CoreErrorKind::UnknownMethod));
        }
        if !call.allowed_methods.contains(&method) {
            return Err(CoreError::new(CoreErrorKind::Forbidden));
        }
        if !call.biz_content.is_object() {
            return Err(CoreError::new(CoreErrorKind::BadRequest(
                "biz_content must be an object".to_string(),
            )));
        }

        match method.as_str() {
            METHOD_POSTS_SEARCH => self.invoke_posts(&call.biz_content),
            METHOD_PHOTOS_RENDER => self.invoke_photos(&call.biz_content),
            METHOD_IMGDEF_COMPARE => self.invoke_imgdef(&call.biz_content),
            _ => Err(CoreError::new(CoreErrorKind::UnknownMethod)),
        }
    }

    fn invoke_posts(&self, biz: &serde_json::Value) -> Result<DispatchOk, CoreError> {
        let req: PostsRequest = parse_biz(biz)?;
        match posts::search_posts(&req) {
            Ok(resp) => to_value(resp),
            Err(err) => Err(from_service(err)),
        }
    }

    fn invoke_photos(&self, biz: &serde_json::Value) -> Result<DispatchOk, CoreError> {
        let req: PhotosRequest = parse_biz(biz)?;
        match photos::render_photo(&req, &self.imgs_dir) {
            Ok(resp) => to_value(resp),
            Err(err) => Err(from_service(err)),
        }
    }

    fn invoke_imgdef(&self, biz: &serde_json::Value) -> Result<DispatchOk, CoreError> {
        let req: ImgdefRequest = parse_biz(biz)?;
        match imgdef::compare(&req, &self.imgs_dir) {
            Ok(resp) => to_value(resp),
            Err(err) => Err(from_service(err)),
        }
    }
}

fn parse_biz<T: DeserializeOwned>(value: &serde_json::Value) -> Result<T, CoreError> {
    match serde_json::from_value(value.clone()) {
        Ok(v) => Ok(v),
        Err(err) => Err(CoreError::new(CoreErrorKind::Unprocessable(
            err.to_string(),
        ))),
    }
}

fn to_value<T: serde::Serialize>(data: T) -> Result<DispatchOk, CoreError> {
    match serde_json::to_value(data) {
        Ok(value) => Ok(DispatchOk { value }),
        Err(_) => Err(CoreError::new(CoreErrorKind::Internal(
            "failed to encode response".to_string(),
        ))),
    }
}

fn from_service(err: ServiceError) -> CoreError {
    let kind = match err {
        ServiceError::BadRequest(m) => CoreErrorKind::BadRequest(m),
        ServiceError::NotFound(m) => CoreErrorKind::NotFound(m),
        ServiceError::Internal(m) => CoreErrorKind::Internal(m),
    };
    CoreError::new(kind)
}

fn apply_ok(ctx: &LogContext, method: &str, input: serde_json::Value, value: serde_json::Value) {
    ctx.set_rpc(method);
    ctx.set_in(input);
    ctx.set_out(value);
}

fn apply_err(ctx: &LogContext, method: &str, input: serde_json::Value, err: &CoreError) {
    ctx.set_rpc(method);
    ctx.set_in(input);
    ctx.set_out(serde_json::Value::Null);
    ctx.set_error_kind(err.kind.as_str());
    ctx.set_error(err.message());
}

pub fn dispatch_logged(
    core: &Core,
    ctx: &LogContext,
    call: Call,
) -> Result<serde_json::Value, CoreError> {
    let method = call.method.clone();
    let input = call.biz_content.clone();
    ctx.set_client_id(call.client_id.clone());
    let start = std::time::Instant::now();
    let result = core.dispatch(call);
    ctx.add_duration_core(crate::logger::elapsed_ms(start));
    match result {
        Ok(ok) => {
            apply_ok(ctx, &method, input, ok.value.clone());
            Ok(ok.value)
        }
        Err(err) => {
            apply_err(ctx, &method, input, &err);
            Err(err)
        }
    }
}

pub fn dispatch_extra(
    core: &Core,
    ctx: &LogContext,
    call: Call,
) -> Result<serde_json::Value, CoreError> {
    let method = call.method.clone();
    let input = call.biz_content.clone();
    ctx.set_client_id(call.client_id.clone());
    let start = std::time::Instant::now();
    let result = core.dispatch(call);
    let duration_core = Some(crate::logger::elapsed_ms(start));
    match result {
        Ok(ok) => {
            ctx.push_extra(ExtraCall {
                rpc: Some(method),
                input,
                output: ok.value.clone(),
                error: None,
                error_kind: None,
                code: None,
                job_id: None,
                duration_core,
            });
            Ok(ok.value)
        }
        Err(err) => {
            ctx.push_extra(ExtraCall {
                rpc: Some(method),
                input,
                output: serde_json::Value::Null,
                error: Some(err.message().to_string()),
                error_kind: Some(err.kind.as_str().to_string()),
                code: None,
                job_id: None,
                duration_core,
            });
            Err(err)
        }
    }
}
