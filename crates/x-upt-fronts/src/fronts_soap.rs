//! SOAP 1.1 XML + dynamic WSDL router (`GET /soap`, `POST /soap`).

use std::collections::HashMap;
use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, OriginalUri, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use http::{HeaderMap, StatusCode, header};
use quick_xml::Reader;
use quick_xml::events::Event;
use x_upt_core::domain::{METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH};
use x_upt_core::error::CoreErrorKind;
use x_upt_core::services::Call;

use crate::contracts::soap_wsdl;
use crate::fronts_rest_rpc::{rest_auth_fail, rest_not_found};
use crate::http_guard::{FrontCtx, attach_std_headers, verify_http_request};
use crate::logger::{LogOutcome, ReqMeta, build_log_entry};
use crate::snapshot::simulate_latency;

/// Builds the `soap` front `axum::Router`.
pub fn build_soap_router(ctx: FrontCtx) -> Router {
    Router::new()
        .route("/soap", get(soap_wsdl_ep).post(soap_post_ep))
        .fallback(rest_not_found)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(ctx)
}

async fn soap_wsdl_ep(
    State(ctx): State<FrontCtx>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let meta = ReqMeta::from_http("GET", uri.path(), uri.query(), &headers, None, Some(0));
    let req_id = meta.request_id.clone();
    let snap = ctx.runtime.load();
    let front_rec = snap
        .fronts
        .get(&ctx.front)
        .cloned()
        .unwrap_or_else(|| x_upt_core::domain::FrontRecord::default_for(ctx.front));
    let xml = soap_wsdl(&front_rec);
    let mut resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
        xml,
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

async fn soap_post_ep(
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

    let (op_name, fields) = match parse_soap_body(&body) {
        Ok(v) => v,
        Err(msg) => {
            return soap_fault(
                &ctx,
                meta,
                &req_id,
                "Client",
                &msg,
                Some(&caller.app.client_id),
            );
        }
    };

    let (biz_method, biz_val) = match soap_fields_to_biz(&op_name, &fields) {
        Ok(v) => v,
        Err(msg) => {
            return soap_fault(
                &ctx,
                meta,
                &req_id,
                "Client",
                &msg,
                Some(&caller.app.client_id),
            );
        }
    };

    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let res = snap.dataset.dispatch(Call::from_app(
        &caller.app,
        biz_method.to_string(),
        biz_val.clone(),
    ));
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);

    match res {
        Ok(data) => {
            let xml = format_soap_response(&op_name, &data);
            let mut resp = (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
                xml,
            )
                .into_response();
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
                    rpc: Some(biz_method.to_string()),
                    input: biz_val,
                    out: data,
                    ..LogOutcome::default()
                },
            ));
            resp
        }
        Err(err) => {
            let fault_code = match err.kind {
                CoreErrorKind::Internal(_) => "Server",
                _ => "Client",
            };
            soap_fault(
                &ctx,
                meta,
                &req_id,
                fault_code,
                err.message(),
                Some(&caller.app.client_id),
            )
        }
    }
}

fn parse_soap_body(xml: &[u8]) -> Result<(String, HashMap<String, String>), String> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut in_body = false;
    let mut op_name: Option<String> = None;
    let mut current_field: Option<String> = None;
    let mut fields = HashMap::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_tag(e.name().as_ref());
                if !in_body {
                    if local == "Body" {
                        in_body = true;
                    }
                } else if op_name.is_none() {
                    op_name = Some(local);
                } else if current_field.is_none() {
                    fields.entry(local.clone()).or_default();
                    current_field = Some(local);
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_tag(e.name().as_ref());
                if in_body && op_name.is_some() && current_field.is_none() {
                    fields.insert(local, String::new());
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(f) = &current_field {
                    let text = t.unescape().map_err(|e| format!("xml text error: {e}"))?;
                    fields.entry(f.clone()).or_default().push_str(&text);
                }
            }
            Ok(Event::End(e)) => {
                let local = local_tag(e.name().as_ref());
                if current_field.as_deref() == Some(local.as_str()) {
                    current_field = None;
                } else if op_name.as_deref() == Some(local.as_str()) {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("malformed SOAP XML: {e}")),
            _ => {}
        }
        buf.clear();
    }

    let op = op_name.ok_or_else(|| "missing SOAP Body operation element".to_string())?;
    Ok((op, fields))
}

fn local_tag(raw: &[u8]) -> String {
    let s = std::str::from_utf8(raw).unwrap_or("");
    s.rsplit(':').next().unwrap_or(s).to_string()
}

fn soap_fields_to_biz(
    op: &str,
    fields: &HashMap<String, String>,
) -> Result<(&'static str, serde_json::Value), String> {
    let mut obj = serde_json::Map::new();
    for (k, v) in fields {
        let val = if matches!(
            k.as_str(),
            "user_id" | "page" | "page_size" | "width" | "height" | "threshold"
        ) {
            let n = v
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("field {k} must be an unsigned integer"))?;
            serde_json::json!(n)
        } else {
            serde_json::Value::String(v.clone())
        };
        obj.insert(k.clone(), val);
    }
    let method = match op {
        "SearchPosts" => METHOD_POSTS_SEARCH,
        "RenderPhoto" => METHOD_PHOTOS_RENDER,
        "CompareImage" => METHOD_IMGDEF_COMPARE,
        other => return Err(format!("unknown SOAP operation: {other}")),
    };
    Ok((method, serde_json::Value::Object(obj)))
}

fn format_soap_response(op: &str, data: &serde_json::Value) -> String {
    let inner = match op {
        "SearchPosts" => {
            let mut items_xml = String::new();
            if let Some(arr) = data.get("items").and_then(serde_json::Value::as_array) {
                for it in arr {
                    let uid = it
                        .get("userId")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let id = it
                        .get("id")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let title = xml_esc(
                        it.get("title")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(""),
                    );
                    let body = xml_esc(
                        it.get("body")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(""),
                    );
                    items_xml.push_str(&format!(
                        "<item><userId>{uid}</userId><id>{id}</id><title>{title}</title><body>{body}</body></item>"
                    ));
                }
            }
            let total = data
                .get("total")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let page = data
                .get("page")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(1);
            let ps = data
                .get("page_size")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(10);
            format!(
                "<SearchPostsResponse xmlns=\"urn:x-upt\"><items>{items_xml}</items><total>{total}</total><page>{page}</page><page_size>{ps}</page_size></SearchPostsResponse>"
            )
        }
        "RenderPhoto" => {
            let scene = xml_esc(
                data.get("scene")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let w = data
                .get("width")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let h = data
                .get("height")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let fmt = xml_esc(
                data.get("format")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let img = data.get("image").unwrap_or(&serde_json::Value::Null);
            let fname = xml_esc(
                img.get("filename")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let mime = xml_esc(
                img.get("mime")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let sz = img
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let b64 = xml_esc(
                img.get("base64")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            format!(
                "<RenderPhotoResponse xmlns=\"urn:x-upt\"><scene>{scene}</scene><width>{w}</width><height>{h}</height><format>{fmt}</format><image><filename>{fname}</filename><mime>{mime}</mime><size>{sz}</size><base64>{b64}</base64></image></RenderPhotoResponse>"
            )
        }
        _ => {
            let m = data
                .get("matched")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let fname = xml_esc(
                data.get("filename")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let dist = data
                .get("distance")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let thr = data
                .get("threshold")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            format!(
                "<CompareImageResponse xmlns=\"urn:x-upt\"><matched>{m}</matched><filename>{fname}</filename><distance>{dist}</distance><threshold>{thr}</threshold></CompareImageResponse>"
            )
        }
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>{inner}</s:Body></s:Envelope>"
    )
}

fn soap_fault(
    ctx: &FrontCtx,
    meta: ReqMeta,
    req_id: &str,
    fault_code: &str,
    fault_string: &str,
    client_id: Option<&str>,
) -> Response {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>{}</faultcode><faultstring>{}</faultstring><detail><request_id>{}</request_id></detail></s:Fault></s:Body></s:Envelope>",
        xml_esc(fault_code),
        xml_esc(fault_string),
        xml_esc(req_id),
    );
    let mut resp = (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
        xml,
    )
        .into_response();
    attach_std_headers(&mut resp, req_id, None);
    ctx.logger.record(build_log_entry(
        ctx.front,
        ctx.port,
        meta,
        LogOutcome {
            status: 500,
            client_id: client_id.map(ToString::to_string),
            error: Some(fault_string.to_string()),
            error_kind: Some("soap_fault".to_string()),
            ..LogOutcome::default()
        },
    ));
    resp
}

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
