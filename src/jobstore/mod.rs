use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::{AppConfig, FrontName, apply_latency};
use crate::core::{Call, Core};
use crate::logger;
use crate::types::logger::{LogEntry, LogEvent};
use crate::types::token::AuthedApp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Expired,
}

impl JobStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

pub struct JobLogMeta {
    pub ip: Option<String>,
    pub query: Option<String>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    pub content_type: Option<String>,
    pub auth: Option<String>,
}

pub struct Job {
    pub id: Uuid,
    pub method: String,
    pub biz_content: serde_json::Value,
    pub status: JobStatus,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created: Instant,
    pub app: AuthedApp,
    pub request_id: String,
    pub log: JobLogMeta,
}

pub struct JobView {
    pub job_id: String,
    pub status: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub message: Option<String>,
}

pub struct JobStore {
    front: FrontName,
    port: u16,
    expire: Duration,
    jobs: RwLock<HashMap<Uuid, Job>>,
    tx: mpsc::Sender<Uuid>,
}

impl JobStore {
    #[must_use]
    pub fn new(
        front: FrontName,
        expire_seconds: u64,
        core: Arc<Core>,
        config: Arc<AppConfig>,
        workers: usize,
        shutdown: CancellationToken,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(256);
        let port = config.fronts.common(front).port;
        let store = Arc::new(Self {
            front,
            port,
            expire: Duration::from_secs(expire_seconds.max(1)),
            jobs: RwLock::new(HashMap::new()),
            tx,
        });
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        for _ in 0..workers.max(1) {
            let store = Arc::clone(&store);
            let core = Arc::clone(&core);
            let config = Arc::clone(&config);
            let rx = Arc::clone(&rx);
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                loop {
                    let id = tokio::select! {
                        () = shutdown.cancelled() => break,
                        id = async {
                            let mut guard = rx.lock().await;
                            guard.recv().await
                        } => id,
                    };
                    let Some(id) = id else { break };
                    if shutdown.is_cancelled() {
                        break;
                    }
                    run_one(&store, &core, &config, id).await;
                }
            });
        }
        let janitor = Arc::clone(&store);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(Duration::from_secs(5)) => {
                        janitor.expire_jobs().await;
                    }
                }
            }
        });
        store
    }

    pub async fn submit(
        &self,
        app: AuthedApp,
        method: String,
        biz_content: serde_json::Value,
        request_id: String,
        log: JobLogMeta,
    ) -> String {
        let id = Uuid::new_v4();
        let job = Job {
            id,
            method,
            biz_content,
            status: JobStatus::Pending,
            result: None,
            error: None,
            created: Instant::now(),
            app,
            request_id,
            log,
        };
        self.jobs.write().await.insert(id, job);
        let _ = self.tx.send(id).await;
        id.to_string()
    }

    pub async fn get(&self, id: &str, client_id: &str) -> Option<JobView> {
        let Ok(uuid) = Uuid::parse_str(id) else {
            return None;
        };
        let jobs = self.jobs.read().await;
        let job = jobs.get(&uuid)?;
        if job.status == JobStatus::Expired || job.app.client_id != client_id {
            return None;
        }
        Some(JobView {
            job_id: job.id.to_string(),
            status: job.status.as_str().to_string(),
            result: job.result.clone(),
            error: if job.status == JobStatus::Failed {
                Some("failed".to_string())
            } else {
                None
            },
            message: job.error.clone(),
        })
    }

    async fn expire_jobs(&self) {
        let mut jobs = self.jobs.write().await;
        let expire = self.expire;
        jobs.retain(|_, job| {
            if job.created.elapsed() > expire {
                emit_job_line(
                    self,
                    job,
                    LogEvent::JobExpired,
                    200,
                    0,
                    Some("expired".to_string()),
                    Some("expired".to_string()),
                    Some(serde_json::json!("expired")),
                    job.biz_content.clone(),
                    serde_json::Value::Null,
                    None,
                    None,
                );
                return false;
            }
            true
        });
    }
}

fn emit_job_line(
    store: &JobStore,
    job: &Job,
    event: LogEvent,
    status: u16,
    duration: u64,
    error: Option<String>,
    error_kind: Option<String>,
    code: Option<serde_json::Value>,
    input: serde_json::Value,
    output: serde_json::Value,
    duration_core: Option<u64>,
    duration_latency: Option<u64>,
) {
    logger::log(&LogEntry {
        request_id: job.request_id.clone(),
        ts: String::new(),
        level: String::new(),
        event,
        method: "POST".to_string(),
        path: "/jobs".to_string(),
        query: job.log.query.clone(),
        status,
        code,
        duration,
        duration_auth: None,
        duration_core,
        duration_latency,
        ip: job.log.ip.clone(),
        client_id: Some(job.app.client_id.clone()),
        bytes_in: None,
        bytes_out: None,
        error,
        error_kind,
        input,
        out: output,
        front: store.front.as_str().to_string(),
        port: store.port,
        rpc: Some(job.method.clone()),
        job_id: Some(job.id.to_string()),
        user_agent: job.log.user_agent.clone(),
        referer: job.log.referer.clone(),
        content_type: job.log.content_type.clone(),
        auth: job.log.auth.clone(),
        pid: 0,
        host: None,
    });
}

async fn run_one(store: &JobStore, core: &Core, config: &AppConfig, id: Uuid) {
    {
        let mut jobs = store.jobs.write().await;
        let drop_expired = {
            let Some(job) = jobs.get_mut(&id) else {
                return;
            };
            if job.created.elapsed() > store.expire {
                job.status = JobStatus::Expired;
                emit_job_line(
                    store,
                    job,
                    LogEvent::JobExpired,
                    200,
                    0,
                    Some("expired".to_string()),
                    Some("expired".to_string()),
                    Some(serde_json::json!("expired")),
                    job.biz_content.clone(),
                    serde_json::Value::Null,
                    None,
                    None,
                );
                true
            } else {
                job.status = JobStatus::Running;
                false
            }
        };
        if drop_expired {
            jobs.remove(&id);
            return;
        }
    }
    let latency = apply_latency(&config.latency).await;
    let start = Instant::now();
    let (method, biz, app) = {
        let jobs = store.jobs.read().await;
        let Some(job) = jobs.get(&id) else { return };
        (job.method.clone(), job.biz_content.clone(), job.app.clone())
    };
    let call = Call::from_app(&app, method, biz.clone());
    let outcome = core.dispatch(call);
    let core_ms = logger::elapsed_ms(start);
    let duration = core_ms.saturating_add(latency);
    {
        let mut jobs = store.jobs.write().await;
        if let Some(job) = jobs.get_mut(&id) {
            match &outcome {
                Ok(ok) => {
                    job.status = JobStatus::Succeeded;
                    job.result = Some(ok.value.clone());
                    emit_job_line(
                        store,
                        job,
                        LogEvent::JobDone,
                        200,
                        duration,
                        None,
                        None,
                        None,
                        biz,
                        ok.value.clone(),
                        Some(core_ms),
                        Some(latency),
                    );
                }
                Err(err) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(err.message().to_string());
                    emit_job_line(
                        store,
                        job,
                        LogEvent::JobDone,
                        200,
                        duration,
                        Some(err.message().to_string()),
                        Some(err.kind.as_str().to_string()),
                        Some(serde_json::json!(err.kind.as_str())),
                        biz,
                        serde_json::Value::Null,
                        Some(core_ms),
                        Some(latency),
                    );
                }
            }
        }
    }
}
