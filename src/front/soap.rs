use std::sync::Arc;

use axum::Router;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::auth::{AuthInput, AuthPolicy, authenticate, policy_from};
use crate::config::{AppConfig, apply_latency};
use crate::core::{Call, Core, CoreErrorKind, dispatch_logged};
use crate::front::access::{AccessMeta, access_log};
use crate::front::util::auth_to_rest;
use crate::logger::context::LogContext;
use crate::types::gateway::{METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH};

const WSDL: &str = include_str!("../../spec/upupt.wsdl");

#[derive(Clone)]
struct SoapState {
    config: Arc<AppConfig>,
    core: Arc<Core>,
    policy: AuthPolicy,
}

pub fn router(config: Arc<AppConfig>, core: Arc<Core>, meta: AccessMeta) -> Router {
    let policy = policy_from(
        config.fronts.soap.common.auth.clone(),
        None,
        Arc::clone(&config.apps),
        config.fronts.soap.common.port,
    );
    let state = SoapState {
        config,
        core,
        policy,
    };
    Router::new()
        .route("/soap", get(handle_wsdl).post(handle_soap))
        .fallback(handle_not_found)
        .layer(middleware::from_fn(access_log))
        .layer(axum::Extension(meta))
        .with_state(state)
}

async fn handle_not_found(Extension(ctx): Extension<LogContext>) -> Response {
    ctx.set_fail("not_found", "not found", serde_json::json!("not_found"));
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn handle_wsdl() -> Response {
    ([(header::CONTENT_TYPE, "text/xml; charset=utf-8")], WSDL).into_response()
}

async fn handle_soap(
    Extension(ctx): Extension<LogContext>,
    State(state): State<SoapState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> Response {
    let input = AuthInput {
        headers: &headers,
        query: uri.query(),
        method: "POST",
        path: "/soap",
        body: &body,
        query_overrides_header: false,
    };
    let start_auth = std::time::Instant::now();
    let app = match authenticate(&state.policy, &input).await {
        Ok(ok) => {
            ctx.set_client_id(ok.app.client_id.clone());
            ctx.set_auth(ok.method.as_str());
            ctx.add_duration_auth(crate::logger::elapsed_ms(start_auth));
            ok.app
        }
        Err(err) => {
            ctx.add_duration_auth(crate::logger::elapsed_ms(start_auth));
            let (status, _, message) = auth_to_rest(&err);
            ctx.set_fail(
                crate::front::util::auth_kind(&err),
                message,
                serde_json::json!(message),
            );
            return (status, message).into_response();
        }
    };
    ctx.add_duration_latency(apply_latency(&state.config.latency).await);
    let xml = String::from_utf8_lossy(&body);
    let Some((op, fields)) = parse_body(&xml) else {
        return soap_fault(&ctx, "Client", "invalid SOAP body");
    };
    let method = match op.as_str() {
        "SearchPosts" => METHOD_POSTS_SEARCH,
        "RenderPhoto" => METHOD_PHOTOS_RENDER,
        "CompareImage" => METHOD_IMGDEF_COMPARE,
        _ => {
            ctx.set_fail(
                "unknown_method",
                "unknown method",
                serde_json::json!("Client"),
            );
            return soap_fault(&ctx, "Client", "unknown method");
        }
    };
    ctx.set_rpc(method);
    let biz = match soap_biz(&op, &fields) {
        Ok(v) => v,
        Err(msg) => return soap_fault(&ctx, "Client", &msg),
    };
    let call = Call::from_app(&app, method.to_string(), biz);
    match dispatch_logged(&state.core, &ctx, call) {
        Ok(value) => soap_ok(&op, &value),
        Err(err) => {
            let code = match err.kind {
                CoreErrorKind::Internal(_) => "Server",
                _ => "Client",
            };
            soap_fault(&ctx, code, err.message())
        }
    }
}

fn soap_biz(
    op: &str,
    fields: &std::collections::HashMap<String, String>,
) -> Result<serde_json::Value, String> {
    match op {
        "SearchPosts" => Ok(serde_json::json!({
            "keyword": require_field(fields, "keyword")?,
            "user_id": parse_u64(fields, "user_id")?,
            "page": parse_u32(fields, "page")?,
            "page_size": parse_u32(fields, "page_size")?,
            "sort": require_field(fields, "sort")?,
        })),
        "RenderPhoto" => Ok(serde_json::json!({
            "scene": require_field(fields, "scene")?,
            "width": parse_u32(fields, "width")?,
            "height": parse_u32(fields, "height")?,
            "format": require_field(fields, "format")?,
        })),
        _ => Ok(serde_json::json!({
            "filename": require_field(fields, "filename")?,
            "image": require_field(fields, "image")?,
            "threshold": parse_u32(fields, "threshold")?,
        })),
    }
}

fn require_field<'a>(
    fields: &'a std::collections::HashMap<String, String>,
    key: &str,
) -> Result<&'a str, String> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

fn parse_u64(fields: &std::collections::HashMap<String, String>, key: &str) -> Result<u64, String> {
    require_field(fields, key)?
        .parse()
        .map_err(|_| format!("invalid {key}"))
}

fn parse_u32(fields: &std::collections::HashMap<String, String>, key: &str) -> Result<u32, String> {
    require_field(fields, key)?
        .parse()
        .map_err(|_| format!("invalid {key}"))
}

fn parse_body(xml: &str) -> Option<(String, std::collections::HashMap<String, String>)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut in_body = false;
    let mut op = None;
    let mut current = None;
    let mut fields = std::collections::HashMap::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if local.eq_ignore_ascii_case("Body") {
                    in_body = true;
                } else if in_body && op.is_none() {
                    op = Some(local);
                } else if in_body {
                    current = Some(local);
                }
            }
            Ok(Event::Empty(e)) => {
                if in_body {
                    let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                    fields.insert(local, String::new());
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(name) = current.take() {
                    let text = t.unescape().ok()?.into_owned();
                    fields.insert(name, text);
                }
            }
            Ok(Event::End(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if local.eq_ignore_ascii_case("Body") {
                    in_body = false;
                } else if in_body && let Some(name) = current.take() {
                    fields.entry(name).or_insert_with(String::new);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
    Some((op?, fields))
}

fn soap_ok(op: &str, value: &serde_json::Value) -> Response {
    let inner = json_to_xml(value);
    let xml = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><{op}Response>{inner}</{op}Response></s:Body></s:Envelope>"#
    );
    ([(header::CONTENT_TYPE, "text/xml; charset=utf-8")], xml).into_response()
}

fn json_to_xml(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("<{k}>{}</{k}>", json_to_xml(v)))
            .collect(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(|v| format!("<item>{}</item>", json_to_xml(v)))
            .collect(),
        serde_json::Value::String(s) => xml_escape(s),
        other => xml_escape(&other.to_string()),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn soap_fault(ctx: &LogContext, code: &str, message: &str) -> Response {
    ctx.set_error(message);
    ctx.set_code(serde_json::json!(code));
    let xml = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><s:Fault><faultcode>{code}</faultcode><faultstring>{}</faultstring><detail><request_id>{}</request_id></detail></s:Fault></s:Body></s:Envelope>"#,
        xml_escape(message),
        ctx.request_id()
    );
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
        xml,
    )
        .into_response()
}
