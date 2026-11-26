use serde::Serialize;

/// 日志行类型：`access` 为一次真实入站请求；`job_done` / `job_expired` 为异步任务状态变化，
/// 沿用提交时的 request_id / method / path，但不是新的入站请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEvent {
    Access,
    JobDone,
    JobExpired,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub request_id: String,
    pub ts: String,
    pub level: String,
    pub event: LogEvent,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub status: u16,
    pub code: Option<serde_json::Value>,
    pub duration: u64,
    pub duration_auth: Option<u64>,
    pub duration_core: Option<u64>,
    pub duration_latency: Option<u64>,
    pub ip: Option<String>,
    pub client_id: Option<String>,
    pub bytes_in: Option<u64>,
    pub bytes_out: Option<u64>,
    pub error: Option<String>,
    pub error_kind: Option<String>,
    #[serde(rename = "in")]
    pub input: serde_json::Value,
    pub out: serde_json::Value,
    pub front: String,
    pub port: u16,
    pub rpc: Option<String>,
    pub job_id: Option<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub content_type: Option<String>,
    pub auth: Option<String>,
    pub pid: u32,
    pub host: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::LogEvent;

    #[test]
    fn log_event_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&LogEvent::Access).unwrap(),
            "\"access\""
        );
        assert_eq!(
            serde_json::to_string(&LogEvent::JobDone).unwrap(),
            "\"job_done\""
        );
        assert_eq!(
            serde_json::to_string(&LogEvent::JobExpired).unwrap(),
            "\"job_expired\""
        );
    }
}
