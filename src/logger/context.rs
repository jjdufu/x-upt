use std::sync::{Arc, Mutex};

use serde_json::Value;

#[derive(Clone)]
pub struct LogContext {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    request_id: String,
    client_id: Option<String>,
    input: Value,
    output: Value,
    error: Option<String>,
    error_kind: Option<String>,
    code: Option<Value>,
    rpc: Option<String>,
    job_id: Option<String>,
    front: String,
    port: u16,
    extras: Vec<ExtraCall>,
    ip: Option<String>,
    query: Option<String>,
    user_agent: Option<String>,
    referer: Option<String>,
    content_type: Option<String>,
    auth: Option<String>,
    auth_hint: Option<String>,
    duration_auth: Option<u64>,
    duration_core: Option<u64>,
    duration_latency: Option<u64>,
}

#[derive(Clone)]
pub struct ExtraCall {
    pub rpc: Option<String>,
    pub input: Value,
    pub output: Value,
    pub error: Option<String>,
    pub error_kind: Option<String>,
    pub code: Option<Value>,
    pub job_id: Option<String>,
    pub duration_core: Option<u64>,
}

impl LogContext {
    #[must_use]
    pub fn new(request_id: String, front: String, port: u16) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                request_id,
                client_id: None,
                input: Value::Object(serde_json::Map::new()),
                output: Value::Null,
                error: None,
                error_kind: None,
                code: None,
                rpc: None,
                job_id: None,
                front,
                port,
                extras: Vec::new(),
                ip: None,
                query: None,
                user_agent: None,
                referer: None,
                content_type: None,
                auth: None,
                auth_hint: None,
                duration_auth: None,
                duration_core: None,
                duration_latency: None,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn request_id(&self) -> String {
        self.lock().request_id.clone()
    }

    pub fn set_client_id(&self, client_id: String) {
        self.lock().client_id = Some(client_id);
    }

    pub fn set_in(&self, input: Value) {
        self.lock().input = input;
    }

    pub fn set_out(&self, output: Value) {
        self.lock().output = output;
    }

    pub fn set_error(&self, error: impl Into<String>) {
        self.lock().error = Some(error.into());
    }

    pub fn set_error_kind(&self, kind: impl Into<String>) {
        self.lock().error_kind = Some(kind.into());
    }

    pub fn set_code(&self, code: Value) {
        self.lock().code = Some(code);
    }

    pub fn set_fail(&self, kind: &str, message: impl Into<String>, code: Value) {
        let mut guard = self.lock();
        guard.error_kind = Some(kind.to_string());
        guard.error = Some(message.into());
        guard.code = Some(code);
    }

    pub fn set_rpc(&self, rpc: impl Into<String>) {
        let value = rpc.into();
        if value.is_empty() {
            return;
        }
        self.lock().rpc = Some(value);
    }

    pub fn clear_rpc(&self) {
        self.lock().rpc = None;
    }

    pub fn set_job_id(&self, job_id: impl Into<String>) {
        self.lock().job_id = Some(job_id.into());
    }

    pub fn set_ip(&self, ip: Option<String>) {
        self.lock().ip = ip;
    }

    pub fn set_query(&self, query: Option<String>) {
        self.lock().query = query;
    }

    pub fn set_user_agent(&self, value: Option<String>) {
        self.lock().user_agent = value;
    }

    pub fn set_referer(&self, value: Option<String>) {
        self.lock().referer = value;
    }

    pub fn set_content_type(&self, value: Option<String>) {
        self.lock().content_type = value;
    }

    pub fn set_auth(&self, auth: impl Into<String>) {
        self.lock().auth = Some(auth.into());
    }

    pub fn set_auth_hint(&self, hint: Option<String>) {
        self.lock().auth_hint = hint;
    }

    pub fn add_duration_auth(&self, ms: u64) {
        let mut guard = self.lock();
        guard.duration_auth = Some(guard.duration_auth.unwrap_or(0).saturating_add(ms));
    }

    pub fn add_duration_core(&self, ms: u64) {
        let mut guard = self.lock();
        guard.duration_core = Some(guard.duration_core.unwrap_or(0).saturating_add(ms));
    }

    pub fn add_duration_latency(&self, ms: u64) {
        let mut guard = self.lock();
        guard.duration_latency = Some(guard.duration_latency.unwrap_or(0).saturating_add(ms));
    }

    pub fn push_extra(&self, extra: ExtraCall) {
        self.lock().extras.push(extra);
    }

    pub fn set_last_extra_code(&self, code: Value) {
        if let Some(last) = self.lock().extras.last_mut() {
            last.code = Some(code);
        }
    }

    pub fn collapse_single_extra(&self) {
        let mut guard = self.lock();
        if guard.extras.len() == 1 && guard.rpc.is_none() {
            let extra = guard.extras.remove(0);
            let gql = guard.input.get("query").is_some();
            guard.rpc = extra.rpc;
            if gql {
                let mut map = match &guard.input {
                    Value::Object(m) => m.clone(),
                    _ => serde_json::Map::new(),
                };
                map.insert("biz".to_string(), extra.input);
                guard.input = Value::Object(map);
            } else {
                guard.input = extra.input;
            }
            guard.output = extra.output;
            if extra.error.is_some() {
                guard.error = extra.error;
            }
            if extra.error_kind.is_some() {
                guard.error_kind = extra.error_kind;
            }
            if extra.code.is_some() {
                guard.code = extra.code;
            }
            guard.job_id = extra.job_id;
            if let Some(ms) = extra.duration_core {
                guard.duration_core = Some(guard.duration_core.unwrap_or(0).saturating_add(ms));
            }
        } else if guard.extras.len() > 1 && guard.rpc.is_none() {
            let n = guard.extras.len();
            match &mut guard.input {
                Value::Object(map) => {
                    map.insert("fields".to_string(), serde_json::json!(n));
                }
                other => {
                    *other = serde_json::json!({ "fields": n });
                }
            }
            guard.output = Value::Null;
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let guard = self.lock();
        Snapshot {
            request_id: guard.request_id.clone(),
            client_id: guard.client_id.clone(),
            input: guard.input.clone(),
            output: guard.output.clone(),
            error: guard.error.clone(),
            error_kind: guard.error_kind.clone(),
            code: guard.code.clone(),
            rpc: guard.rpc.clone(),
            job_id: guard.job_id.clone(),
            front: guard.front.clone(),
            port: guard.port,
            extras: guard.extras.clone(),
            ip: guard.ip.clone(),
            query: guard.query.clone(),
            user_agent: guard.user_agent.clone(),
            referer: guard.referer.clone(),
            content_type: guard.content_type.clone(),
            auth: guard.auth.clone().or_else(|| guard.auth_hint.clone()),
            duration_auth: guard.duration_auth,
            duration_core: guard.duration_core,
            duration_latency: guard.duration_latency,
        }
    }
}

pub struct Snapshot {
    pub request_id: String,
    pub client_id: Option<String>,
    pub input: Value,
    pub output: Value,
    pub error: Option<String>,
    pub error_kind: Option<String>,
    pub code: Option<Value>,
    pub rpc: Option<String>,
    pub job_id: Option<String>,
    pub front: String,
    pub port: u16,
    pub extras: Vec<ExtraCall>,
    pub ip: Option<String>,
    pub query: Option<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub content_type: Option<String>,
    pub auth: Option<String>,
    pub duration_auth: Option<u64>,
    pub duration_core: Option<u64>,
    pub duration_latency: Option<u64>,
}
