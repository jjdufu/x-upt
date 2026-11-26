//! Same-source structured API documentation and OpenAPI 3.0 generator for all 15 upstream fronts.

use serde::Serialize;

use crate::domain::{EnvelopeProfileSpec, FrontName, FrontRecord};

/// Single endpoint documentation entry with dynamically populated host/port examples.
#[derive(Debug, Clone, Serialize)]
pub struct EndpointDoc {
    /// HTTP/protocol method (`POST`, `GET`, `gRPC`, `WS`).
    pub method: String,
    /// Endpoint path or RPC path.
    pub path: String,
    /// Concise endpoint description.
    pub description: String,
    /// Formatted sample request payload.
    pub request_example: String,
    /// Formatted sample response payload.
    pub response_example: String,
    /// Runnable CLI example (`curl` / `grpcurl` / `websocat`) using live configured port.
    pub curl_example: String,
}

/// Complete documentation view for one simulated upstream front.
#[derive(Debug, Clone, Serialize)]
pub struct FrontDocView {
    /// Front snake_case identifier.
    pub name: String,
    /// `"core"` or `"envelope"`.
    pub category: &'static str,
    /// Protocol label.
    pub protocol: &'static str,
    /// Configured bind host.
    pub host: String,
    /// Configured bind port.
    pub port: u16,
    /// Current enabled flag.
    pub enabled: bool,
    /// Functional summary.
    pub summary: String,
    /// Authentication or envelope security summary.
    pub auth_notes: String,
    /// Endpoint list.
    pub endpoints: Vec<EndpointDoc>,
}

/// Builds live port-aware documentation for all 15 fronts.
#[must_use]
pub fn build_front_docs(
    fronts: &[FrontRecord],
    profiles_by_front: &[(FrontName, Vec<EnvelopeProfileSpec>)],
) -> Vec<FrontDocView> {
    fronts
        .iter()
        .map(|f| {
            let profiles = profiles_by_front
                .iter()
                .find(|(n, _)| *n == f.name)
                .map_or(&[][..], |(_, p)| p.as_slice());
            build_one(f, profiles)
        })
        .collect()
}

/// Generates an OpenAPI 3.0 JSON document reflecting all HTTP-based upstream fronts and configured ports.
#[must_use]
pub fn build_upstream_openapi(
    fronts: &[FrontRecord],
    profiles_by_front: &[(FrontName, Vec<EnvelopeProfileSpec>)],
) -> serde_json::Value {
    let mut servers = Vec::new();
    let mut paths = serde_json::Map::new();

    for f in fronts {
        if matches!(f.name, FrontName::Grpc | FrontName::Ws) {
            continue;
        }
        let scheme = if f.tls.enabled { "https" } else { "http" };
        let host = display_host(&f.host);
        servers.push(serde_json::json!({
            "url": format!("{scheme}://{host}:{}", f.port),
            "description": format!("{} ({})", f.name.as_str(), f.name.protocol_label())
        }));
        let profiles = profiles_by_front
            .iter()
            .find(|(n, _)| *n == f.name)
            .map_or(&[][..], |(_, p)| p.as_slice());
        let doc = build_one(f, profiles);
        for ep in doc.endpoints {
            let key = format!("/_front/{}{}", f.name.as_str(), ep.path);
            let m = ep.method.to_ascii_lowercase();
            let entry = paths
                .entry(key)
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    m,
                    serde_json::json!({
                        "summary": ep.description,
                        "tags": [f.name.as_str()],
                        "responses": { "200": { "description": "OK" } }
                    }),
                );
            }
        }
    }

    serde_json::json!({
        "openapi": "3.0.3",
        "info": {
            "title": "x-upt Simulated Upstream Protocols",
            "version": "0.1.0",
            "description": "Same-source specification of HTTP-accessible simulated upstream fronts."
        },
        "servers": servers,
        "paths": paths
    })
}

fn display_host(host: &str) -> &str {
    let h = host.trim();
    if h.is_empty() || h == "0.0.0.0" || h == "::" {
        "127.0.0.1"
    } else {
        h
    }
}

fn build_one(f: &FrontRecord, profiles: &[EnvelopeProfileSpec]) -> FrontDocView {
    let scheme = if f.tls.enabled { "https" } else { "http" };
    let host = display_host(&f.host);
    let base = format!("{scheme}://{host}:{}", f.port);
    if f.name.is_envelope() {
        return build_envelope_doc(f, &base, profiles);
    }
    let auth_list = if f.name == FrontName::SignGateway {
        "HMAC-SHA256 表单签名 (app_id + timestamp + nonce + sign)".to_string()
    } else {
        f.auth
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (summary, endpoints) = core_endpoints(f, host, &base);
    FrontDocView {
        name: f.name.as_str().to_string(),
        category: f.name.category(),
        protocol: f.name.protocol_label(),
        host: f.host.clone(),
        port: f.port,
        enabled: f.enabled,
        summary: summary.to_string(),
        auth_notes: auth_list,
        endpoints,
    }
}

fn core_endpoints(f: &FrontRecord, host: &str, base: &str) -> (&'static str, Vec<EndpointDoc>) {
    let sample_biz = r#"{"keyword":"gRPC","user_id":6,"page":1,"page_size":10,"sort":"id_desc"}"#;
    let sample_posts_out = r#"{"items":[{"userId":6,"id":18,"title":"...","body":"..."}],"total":1,"page":1,"page_size":10}"#;
    match f.name {
        FrontName::OauthGateway => (
            "OAuth 2.0 client_credentials 换票 + JSON 信封网关",
            vec![
                EndpointDoc {
                    method: "POST".into(),
                    path: "/v3/oauth/token".into(),
                    description: "签发本面专用 Bearer 访问令牌".into(),
                    request_example: "grant_type=client_credentials&client_id=demo_client&client_secret=<SECRET>".into(),
                    response_example: r#"{"access_token":"...","token_type":"Bearer","expires_in":1800,"scope":"x_upt.posts.search"}"#.into(),
                    curl_example: format!("curl -X POST '{base}/v3/oauth/token' -d 'grant_type=client_credentials&client_id=demo_client&client_secret=<SECRET>'"),
                },
                EndpointDoc {
                    method: "POST".into(),
                    path: "/v3/gateway".into(),
                    description: "执行已开启的模拟业务子接口并返回标准 code/msg/request_id/data 信封".into(),
                    request_example: format!(r#"{{"method":"x_upt.posts.search","biz_content":{sample_biz}}}"#),
                    response_example: format!(r#"{{"code":0,"msg":"ok","request_id":"...","data":{sample_posts_out}}}"#),
                    curl_example: format!(r#"curl -X POST '{base}/v3/gateway' -H 'Authorization: Bearer <TOKEN>' -H 'Content-Type: application/json' -d '{{"method":"x_upt.posts.search","biz_content":{sample_biz}}}'"#),
                },
            ],
        ),
        FrontName::SignGateway => (
            "表单字段 ASCII 升序拼接 + HMAC-SHA256 Base64 双向签名网关",
            vec![EndpointDoc {
                method: "POST".into(),
                path: "/gateway.do".into(),
                description: "校验 app_id/charset/version/timestamp/nonce/sign_type/sign 后调用业务核".into(),
                request_example: format!("app_id=demo_client&method=x_upt.posts.search&charset=UTF-8&version=1.0&timestamp=1700000000&nonce=nonce12345&sign_type=HMAC-SHA256&sign=<BASE64_HMAC>&biz_content={sample_biz}"),
                response_example: r#"{"x_upt_posts_search_response":{"code":"10000","msg":"Success","request_id":"...","items":[],"total":0,"page":1,"page_size":10},"sign":"<BASE64_HMAC>"}"#.into(),
                curl_example: format!("curl -X POST '{base}/gateway.do' -H 'Content-Type: application/x-www-form-urlencoded' --data-urlencode 'app_id=demo_client' --data-urlencode 'method=x_upt.posts.search'"),
            }],
        ),
        FrontName::Rest => rest_endpoints(base, sample_biz, sample_posts_out),
        FrontName::Jsonrpc => (
            "JSON-RPC 2.0 单条、通知与批量调用接口",
            vec![EndpointDoc {
                method: "POST".into(),
                path: "/rpc".into(),
                description: "JSON-RPC 2.0 入口（支持 batch 数组与无 id 通知）".into(),
                request_example: format!(r#"{{"jsonrpc":"2.0","method":"x_upt.posts.search","params":{sample_biz},"id":1}}"#),
                response_example: format!(r#"{{"jsonrpc":"2.0","result":{sample_posts_out},"id":1}}"#),
                curl_example: format!(r#"curl -X POST '{base}/rpc' -H 'X-API-Key: <SECRET>' -H 'Content-Type: application/json' -d '{{"jsonrpc":"2.0","method":"x_upt.posts.search","params":{sample_biz},"id":1}}'"#),
            }],
        ),
        FrontName::Graphql => (
            "GraphQL 查询/变更服务与实时 SDL 导出",
            vec![
                EndpointDoc {
                    method: "POST".into(),
                    path: "/graphql".into(),
                    description: "执行 postsSearch 查询及 photosRender / imgdefCompare 变更".into(),
                    request_example: r#"{"query":"query($k:String!,$u:Int!){postsSearch(keyword:$k,userId:$u,page:1,pageSize:10,sort:ID_DESC){total items{id title}}}","variables":{"k":"gRPC","u":6}}"#.into(),
                    response_example: r#"{"data":{"postsSearch":{"total":1,"items":[{"id":18,"title":"..."}]}},"extensions":{"request_id":"..."}}"#.into(),
                    curl_example: format!(r#"curl -X POST '{base}/graphql' -H 'X-API-Key: <SECRET>' -H 'Content-Type: application/json' -d '{{"query":"{{ postsSearch(keyword:\"\",userId:1,page:1,pageSize:5,sort:ID_ASC){{ total }} }}"}}'"#),
                },
                EndpointDoc {
                    method: "GET".into(),
                    path: "/sdl".into(),
                    description: "免鉴权获取当前运行期 GraphQL Schema SDL".into(),
                    request_example: String::new(),
                    response_example: "type Query { postsSearch(...): PostsResult! }".into(),
                    curl_example: format!("curl '{base}/sdl'"),
                },
            ],
        ),
        FrontName::Grpc => (
            "HTTP/2 (h2c / TLS) Unary gRPC 服务 x_upt.v1.XUpt",
            vec![EndpointDoc {
                method: "gRPC".into(),
                path: "/x_upt.v1.XUpt/SearchPosts".into(),
                description: "Unary RPC：SearchPosts / RenderPhoto / CompareImage".into(),
                request_example: sample_biz.into(),
                response_example: sample_posts_out.into(),
                curl_example: format!(r#"grpcurl -plaintext -H 'x-api-key: <SECRET>' -d '{sample_biz}' {host}:{} x_upt.v1.XUpt/SearchPosts"#, f.port),
            }],
        ),
        FrontName::Soap => (
            "SOAP 1.1 XML 协议面与动态地址 WSDL",
            vec![
                EndpointDoc {
                    method: "POST".into(),
                    path: "/soap".into(),
                    description: "SOAP 1.1 Envelope 调用 SearchPosts / RenderPhoto / CompareImage".into(),
                    request_example: "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><SearchPosts><keyword>gRPC</keyword><user_id>6</user_id><page>1</page><page_size>10</page_size><sort>id_desc</sort></SearchPosts></s:Body></s:Envelope>".into(),
                    response_example: "<s:Envelope ...><s:Body><SearchPostsResponse>...</SearchPostsResponse></s:Body></s:Envelope>".into(),
                    curl_example: format!("curl -u 'demo_client:<SECRET>' -X POST '{base}/soap' -H 'Content-Type: text/xml; charset=utf-8' -d '<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><SearchPosts><keyword></keyword><user_id>1</user_id><page>1</page><page_size>5</page_size><sort>id_asc</sort></SearchPosts></s:Body></s:Envelope>'"),
                },
                EndpointDoc {
                    method: "GET".into(),
                    path: "/soap".into(),
                    description: "免鉴权下载绑定当前端口地址的 WSDL 定义".into(),
                    request_example: String::new(),
                    response_example: "<definitions targetNamespace=\"urn:x-upt\">...</definitions>".into(),
                    curl_example: format!("curl '{base}/soap'"),
                },
            ],
        ),
        FrontName::Ws => {
            let ws_scheme = if f.tls.enabled { "wss" } else { "ws" };
            (
                "WebSocket 长连接文本帧调用（支持握手头、Query 或首帧鉴权）",
                vec![EndpointDoc {
                    method: "WS".into(),
                    path: "/ws".into(),
                    description: "发送 {\"id\",\"method\",\"biz_content\"} 帧并接收带 request_id 的响应帧".into(),
                    request_example: format!(r#"{{"id":"req-1","method":"x_upt.posts.search","biz_content":{sample_biz}}}"#),
                    response_example: format!(r#"{{"id":"req-1","request_id":"req-1","code":0,"msg":"ok","data":{sample_posts_out}}}"#),
                    curl_example: format!("websocat '{ws_scheme}://{host}:{}/ws?api_key=<SECRET>'", f.port),
                }],
            )
        }
        _ => (
            "异步任务提交（202 Accepted）与状态轮询接口",
            vec![
                EndpointDoc {
                    method: "POST".into(),
                    path: "/jobs".into(),
                    description: "提交异步业务调用任务".into(),
                    request_example: format!(r#"{{"method":"x_upt.posts.search","biz_content":{sample_biz}}}"#),
                    response_example: r#"{"job_id":"...","status":"pending"}"#.into(),
                    curl_example: format!(r#"curl -X POST '{base}/jobs' -H 'X-API-Key: <SECRET>' -H 'Content-Type: application/json' -d '{{"method":"x_upt.posts.search","biz_content":{sample_biz}}}'"#),
                },
                EndpointDoc {
                    method: "GET".into(),
                    path: "/jobs/{id}".into(),
                    description: "轮询本客户端提交的任务执行状态与结果".into(),
                    request_example: String::new(),
                    response_example: r#"{"job_id":"...","status":"succeeded","result":{...},"error":null,"message":null}"#.into(),
                    curl_example: format!("curl '{base}/jobs/<JOB_ID>' -H 'X-API-Key: <SECRET>'"),
                },
            ],
        ),
    }
}

fn rest_endpoints(
    base: &str,
    sample_biz: &str,
    sample_posts_out: &str,
) -> (&'static str, Vec<EndpointDoc>) {
    (
        "标准 REST 多方法（GET / POST JSON / POST Form / Multipart / HEAD）业务接口",
        vec![
            EndpointDoc {
                method: "POST".into(),
                path: "/posts/search".into(),
                description: "检索已导入帖子（同路径支持 GET query、POST JSON、POST Form、HEAD）".into(),
                request_example: sample_biz.into(),
                response_example: sample_posts_out.into(),
                curl_example: format!("curl -X POST '{base}/posts/search' -H 'X-API-Key: <SECRET>' -H 'Content-Type: application/json' -d '{sample_biz}'"),
            },
            EndpointDoc {
                method: "POST".into(),
                path: "/photos/render".into(),
                description: "按场景哈希稳定选取已导入图片并缩放转码".into(),
                request_example: r#"{"scene":"article_cover","width":64,"height":64,"format":"png"}"#.into(),
                response_example: r#"{"scene":"article_cover","width":64,"height":64,"format":"png","image":{"filename":"...","mime":"image/png","size":512,"base64":"..."}}"#.into(),
                curl_example: format!(r#"curl -X POST '{base}/photos/render' -H 'X-API-Key: <SECRET>' -H 'Content-Type: application/json' -d '{{"scene":"article_cover","width":64,"height":64,"format":"png"}}'"#),
            },
            EndpointDoc {
                method: "POST".into(),
                path: "/imgdef/compare".into(),
                description: "计算上传图与指定已导入图片的 64 位感知哈希汉明距离（支持 JSON 与 multipart/form-data）".into(),
                request_example: r#"{"filename":"sample.png","image":"<BASE64>","threshold":5}"#.into(),
                response_example: r#"{"matched":true,"filename":"sample.png","distance":0,"threshold":5}"#.into(),
                curl_example: format!(r#"curl -X POST '{base}/imgdef/compare' -H 'X-API-Key: <SECRET>' -F 'filename=sample.png' -F 'threshold=5' -F 'image=@sample.png'"#),
            },
        ],
    )
}

fn build_envelope_doc(
    f: &FrontRecord,
    base: &str,
    profiles: &[EnvelopeProfileSpec],
) -> FrontDocView {
    let mut endpoints = Vec::new();
    for p in profiles {
        let method = p.methods.first().cloned().unwrap_or_else(|| "POST".into());
        let resp = if p.kind == "issue" {
            let tf = p.token_response_field.as_deref().unwrap_or("access_token");
            format!(r#"{{"{tf}":"...","expires_in":1800}}"#)
        } else {
            r#"{"ok":true,"reason":"","hit":true,"fields":{"city":"hz"},"digest":"..."}"#.into()
        };
        endpoints.push(EndpointDoc {
            method: method.clone(),
            path: p.path.clone(),
            description: format!(
                "画像 {} (kind={}, body={}, sign={:?}, crypt={:?}, fail_in={})",
                p.name, p.kind, p.body_kind, p.sign_alg, p.crypt_alg, p.fail_in
            ),
            request_example: r#"{"city":"hz"}"#.into(),
            response_example: resp,
            curl_example: format!("curl -X {method} '{base}{}'", p.path),
        });
    }
    FrontDocView {
        name: f.name.as_str().to_string(),
        category: "envelope",
        protocol: f.name.protocol_label(),
        host: f.host.clone(),
        port: f.port,
        enabled: f.enabled,
        summary: "按槽位提取参数、验签/换票/验凭据并按画像执行加解密，返回确定性 digest 结构"
            .into(),
        auth_notes: format!("已配置 {} 条信封画像", profiles.len()),
        endpoints,
    }
}
