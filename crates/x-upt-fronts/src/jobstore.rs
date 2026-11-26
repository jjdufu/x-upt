//! Client-isolated asynchronous job store and worker pool for the `job` front.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use x_upt_core::domain::{AuthedApp, FrontName};
use x_upt_core::logger_types::LogEvent;
use x_upt_core::services::Call;

use crate::logger::{AccessLogger, LogOutcome, ReqMeta, build_log_entry};
use crate::snapshot::{SharedRuntime, simulate_latency};

const MAX_JOBS: usize = 2000;

/// Summary item returned by `GET /api/v1/jobs`.
#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    /// Unique job UUID.
    pub job_id: String,
    /// Submitting client ID.
    pub client_id: String,
    /// Invoked business method.
    pub method: String,
    /// Lifecycle state (`pending`, `running`, `succeeded`, `failed`, `expired`).
    pub status: String,
    /// Optional error code if failed/expired.
    pub error: Option<String>,
}

/// 5-field poll payload returned by `GET /jobs/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct JobPollView {
    /// Unique job UUID.
    pub job_id: String,
    /// Lifecycle state.
    pub status: String,
    /// Completed result object when `status == "succeeded"`.
    pub result: Option<serde_json::Value>,
    /// Error code when `status == "failed"`.
    pub error: Option<String>,
    /// Error message when `status == "failed"`.
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
struct StoredJob {
    job_id: String,
    client_id: String,
    allowed_methods: std::collections::HashSet<String>,
    method: String,
    biz_content: serde_json::Value,
    request_id: String,
    status: &'static str,
    result: Option<serde_json::Value>,
    error: Option<String>,
    message: Option<String>,
    created_at: Instant,
}

#[derive(Debug, Default)]
struct JobInner {
    by_id: HashMap<String, StoredJob>,
    order: VecDeque<String>,
    pending: VecDeque<String>,
}

/// Shared asynchronous job queue and state store.
#[derive(Debug, Default)]
pub struct JobStore {
    inner: Mutex<JobInner>,
    notify: Notify,
}

impl JobStore {
    /// Creates an empty [`JobStore`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueues a validated business call and returns its `job_id`.
    #[must_use]
    pub fn submit(
        &self,
        app: &AuthedApp,
        method: String,
        biz_content: serde_json::Value,
        request_id: String,
    ) -> String {
        let job_id = Uuid::new_v4().to_string();
        let entry = StoredJob {
            job_id: job_id.clone(),
            client_id: app.client_id.clone(),
            allowed_methods: app.methods.clone(),
            method,
            biz_content,
            request_id,
            status: "pending",
            result: None,
            error: None,
            message: None,
            created_at: Instant::now(),
        };
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        while guard.order.len() >= MAX_JOBS {
            let Some(oldest) = guard.order.pop_front() else {
                break;
            };
            guard.by_id.remove(&oldest);
        }
        guard.order.push_back(job_id.clone());
        guard.pending.push_back(job_id.clone());
        guard.by_id.insert(job_id.clone(), entry);
        drop(guard);
        self.notify.notify_one();
        job_id
    }

    /// Polls a job owned by `client_id`, refreshing `expired` state against `expire_seconds`.
    #[must_use]
    pub fn poll(&self, job_id: &str, client_id: &str, expire_seconds: u64) -> Option<JobPollView> {
        let ttl = Duration::from_secs(expire_seconds.max(1));
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let job = guard.by_id.get_mut(job_id)?;
        if job.client_id != client_id {
            return None;
        }
        if job.created_at.elapsed() > ttl
            && matches!(job.status, "pending" | "succeeded" | "failed")
        {
            job.status = "expired";
            job.result = None;
        }
        Some(JobPollView {
            job_id: job.job_id.clone(),
            status: job.status.to_string(),
            result: job.result.clone(),
            error: job.error.clone(),
            message: job.message.clone(),
        })
    }

    /// Lists newest-first job summaries for the WebUI `/api/v1/jobs` view.
    #[must_use]
    pub fn list_recent(&self, expire_seconds: u64) -> Vec<JobSummary> {
        let ttl = Duration::from_secs(expire_seconds.max(1));
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let ids: Vec<String> = guard.order.iter().rev().take(200).cloned().collect();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(job) = guard.by_id.get_mut(&id) {
                if job.created_at.elapsed() > ttl
                    && matches!(job.status, "pending" | "succeeded" | "failed")
                {
                    job.status = "expired";
                }
                out.push(JobSummary {
                    job_id: job.job_id.clone(),
                    client_id: job.client_id.clone(),
                    method: job.method.clone(),
                    status: job.status.to_string(),
                    error: job.error.clone(),
                });
            }
        }
        out
    }

    /// Spawns `workers` background tasks bound to `cancel`.
    pub fn spawn_workers(
        self: &Arc<Self>,
        workers: usize,
        port: u16,
        runtime: SharedRuntime,
        logger: AccessLogger,
        cancel: CancellationToken,
    ) {
        for _ in 0..workers.max(1) {
            let store = Arc::clone(self);
            let rt = runtime.clone();
            let log = logger.clone();
            let tok = cancel.clone();
            tokio::spawn(async move {
                worker_loop(store, port, rt, log, tok).await;
            });
        }
    }

    fn claim_next(&self, expire_seconds: u64) -> Option<(StoredJob, bool)> {
        let ttl = Duration::from_secs(expire_seconds.max(1));
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some(id) = guard.pending.pop_front() {
            let Some(job) = guard.by_id.get_mut(&id) else {
                continue;
            };
            if job.created_at.elapsed() > ttl {
                job.status = "expired";
                return Some((job.clone(), true));
            }
            job.status = "running";
            return Some((job.clone(), false));
        }
        None
    }

    fn finish_job(&self, job_id: &str, result: Result<serde_json::Value, (String, String)>) {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(job) = guard.by_id.get_mut(job_id) else {
            return;
        };
        match result {
            Ok(val) => {
                job.status = "succeeded";
                job.result = Some(val);
            }
            Err((code, msg)) => {
                job.status = "failed";
                job.error = Some(code);
                job.message = Some(msg);
            }
        }
    }
}

async fn worker_loop(
    store: Arc<JobStore>,
    port: u16,
    runtime: SharedRuntime,
    logger: AccessLogger,
    cancel: CancellationToken,
) {
    while !cancel.is_cancelled() {
        let snap = runtime.load();
        if let Some((job, expired)) = store.claim_next(snap.settings.job_expire_seconds) {
            execute_claimed_job(&store, port, &snap, &logger, job, expired).await;
            continue;
        }
        tokio::select! {
            () = cancel.cancelled() => break,
            () = store.notify.notified() => {}
        }
    }
}

async fn execute_claimed_job(
    store: &JobStore,
    port: u16,
    snap: &x_upt_store::dto::RuntimeSnapshotData,
    logger: &AccessLogger,
    job: StoredJob,
    expired: bool,
) {
    let meta = ReqMeta {
        request_id: job.request_id.clone(),
        method: "JOB".to_string(),
        path: "/jobs".to_string(),
        query: None,
        ip: None,
        user_agent: None,
        referer: None,
        content_type: None,
        host: None,
        bytes_in: None,
        started: Instant::now(),
    };
    if expired {
        logger.record(build_log_entry(
            FrontName::Job,
            port,
            meta,
            LogOutcome {
                event: Some(LogEvent::JobExpired),
                status: 408,
                client_id: Some(job.client_id),
                rpc: Some(job.method),
                job_id: Some(job.job_id),
                error: Some("job expired".to_string()),
                error_kind: Some("expired".to_string()),
                input: job.biz_content,
                ..LogOutcome::default()
            },
        ));
        return;
    }
    let lat_ms = simulate_latency(&snap.settings).await;
    let t_core = Instant::now();
    let res = snap.dataset.dispatch(Call {
        client_id: job.client_id.clone(),
        allowed_methods: job.allowed_methods.clone(),
        method: job.method.clone(),
        biz_content: job.biz_content.clone(),
    });
    let core_ms = u64::try_from(t_core.elapsed().as_millis()).unwrap_or(0);
    match res {
        Ok(val) => {
            store.finish_job(&job.job_id, Ok(val.clone()));
            logger.record(build_log_entry(
                FrontName::Job,
                port,
                meta,
                LogOutcome {
                    event: Some(LogEvent::JobDone),
                    status: 200,
                    duration_core: Some(core_ms),
                    duration_latency: Some(lat_ms),
                    client_id: Some(job.client_id),
                    rpc: Some(job.method),
                    job_id: Some(job.job_id),
                    input: job.biz_content,
                    out: val,
                    ..LogOutcome::default()
                },
            ));
        }
        Err(err) => {
            let kind = err.kind.as_str().to_string();
            let msg = err.message().to_string();
            store.finish_job(&job.job_id, Err((kind.clone(), msg.clone())));
            logger.record(build_log_entry(
                FrontName::Job,
                port,
                meta,
                LogOutcome {
                    event: Some(LogEvent::JobDone),
                    status: 422,
                    duration_core: Some(core_ms),
                    duration_latency: Some(lat_ms),
                    client_id: Some(job.client_id),
                    rpc: Some(job.method),
                    job_id: Some(job.job_id),
                    error: Some(msg),
                    error_kind: Some(kind),
                    input: job.biz_content,
                    ..LogOutcome::default()
                },
            ));
        }
    }
}
