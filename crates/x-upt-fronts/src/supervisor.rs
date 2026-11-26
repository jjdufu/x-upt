//! `FrontSupervisor`: zero-restart dynamic TCP/TLS listener binding, hot stop, and graceful drain.

use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use rustls::pki_types::CertificateDer;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use x_upt_core::domain::{FrontName, FrontRecord};
use x_upt_core::error::AppError;

use crate::build_front_router;
use crate::http_guard::FrontCtx;
use crate::jobstore::JobStore;
use crate::logger::AccessLogger;
use crate::snapshot::SharedRuntime;
use crate::token::FrontSecurityState;

struct RunningFront {
    record: FrontRecord,
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

/// Manages the live TCP/TLS listeners and background workers of all 15 simulated upstream fronts.
#[derive(Clone)]
pub struct FrontSupervisor {
    runtime: SharedRuntime,
    logger: AccessLogger,
    jobs: Arc<JobStore>,
    root_cancel: CancellationToken,
    active: Arc<Mutex<HashMap<FrontName, RunningFront>>>,
}

impl std::fmt::Debug for FrontSupervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrontSupervisor").finish_non_exhaustive()
    }
}

impl FrontSupervisor {
    /// Creates a new supervisor sharing `runtime`, `logger`, `jobs`, and `root_cancel`.
    #[must_use]
    pub fn new(
        runtime: SharedRuntime,
        logger: AccessLogger,
        jobs: Arc<JobStore>,
        root_cancel: CancellationToken,
    ) -> Self {
        Self {
            runtime,
            logger,
            jobs,
            root_cancel,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Accesses the shared runtime snapshot holder.
    #[must_use]
    pub fn runtime(&self) -> &SharedRuntime {
        &self.runtime
    }

    /// Accesses the shared access logger.
    #[must_use]
    pub fn logger(&self) -> &AccessLogger {
        &self.logger
    }

    /// Accesses the shared job store.
    #[must_use]
    pub fn jobs(&self) -> &Arc<JobStore> {
        &self.jobs
    }

    /// Returns the set of fronts whose listeners are currently bound and active.
    #[must_use]
    pub fn running_set(&self) -> HashSet<FrontName> {
        let guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        guard.keys().copied().collect()
    }

    /// Binds and starts `record` if not already running (or rebinds if already running).
    ///
    /// # Errors
    /// Returns [`AppError::InvalidInput`] on malformed TLS PEMs or [`AppError::Conflict`] if `host:port` cannot bind.
    pub async fn start_front(&self, record: FrontRecord) -> Result<(), AppError> {
        self.stop_front(record.name).await;

        let tls_acceptor = if record.tls.enabled {
            Some(build_tls_acceptor(&record)?)
        } else {
            None
        };

        let bind_addr = format!("{}:{}", record.host.trim(), record.port);
        let listener = TcpListener::bind(&bind_addr).await.map_err(|e| {
            AppError::precondition(
                format!(
                    "无法绑定接入面 {} 监听地址 {bind_addr}: {e}",
                    record.name.as_str()
                ),
                vec![format!("端口或地址不可用: {bind_addr} ({e})")],
            )
        })?;

        let child_cancel = self.root_cancel.child_token();
        let snap = self.runtime.load();
        let sec =
            FrontSecurityState::new(record.expire_minutes, snap.settings.sign_nonce_ttl_seconds);
        let ctx = FrontCtx {
            front: record.name,
            port: record.port,
            runtime: self.runtime.clone(),
            sec,
            logger: self.logger.clone(),
            jobs: Arc::clone(&self.jobs),
        };

        if record.name == FrontName::Job {
            self.jobs.spawn_workers(
                record.job_workers,
                record.port,
                self.runtime.clone(),
                self.logger.clone(),
                child_cancel.clone(),
            );
        }

        let router = build_front_router(&record, ctx);
        let loop_cancel = child_cancel.clone();
        let h2_only = record.name == FrontName::Grpc && !record.tls.enabled && record.proto.h2c;
        let front_name = record.name;

        let handle = tokio::spawn(async move {
            accept_loop(listener, tls_acceptor, router, h2_only, loop_cancel).await;
            info!(
                front = front_name.as_str(),
                "upstream front listener stopped"
            );
        });

        info!(
            front = record.name.as_str(),
            addr = %bind_addr,
            tls = record.tls.enabled,
            "upstream front listener bound"
        );

        let mut guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        guard.insert(
            record.name,
            RunningFront {
                record,
                cancel: child_cancel,
                handle,
            },
        );
        Ok(())
    }

    /// Cancels `front`'s listener and waits for the socket to close and release its port.
    pub async fn stop_front(&self, front: FrontName) {
        let removed = {
            let mut guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
            guard.remove(&front)
        };
        if let Some(running) = removed {
            running.cancel.cancel();
            drop(tokio::time::timeout(Duration::from_secs(5), running.handle).await);
        }
    }

    /// Rebinds `record` if listener-level parameters changed while active.
    ///
    /// # Errors
    /// Returns [`AppError`] if rebinding the updated address/TLS fails.
    pub async fn rebind_if_needed(&self, record: FrontRecord) -> Result<(), AppError> {
        let needs_rebind = {
            let guard = self.active.lock().unwrap_or_else(PoisonError::into_inner);
            guard.get(&record.name).is_some_and(|cur| {
                cur.record.host != record.host
                    || cur.record.port != record.port
                    || cur.record.proto != record.proto
                    || cur.record.tls != record.tls
                    || cur.record.ws_max_conn != record.ws_max_conn
                    || cur.record.ws_ping_secs != record.ws_ping_secs
                    || cur.record.job_workers != record.job_workers
            })
        };
        if needs_rebind {
            self.start_front(record).await?;
        }
        Ok(())
    }

    /// Restores any fronts marked `enabled = true` in `SQLite` on startup.
    pub async fn restore_enabled(&self, fronts: &[FrontRecord]) {
        for f in fronts.iter().filter(|f| f.enabled) {
            if let Err(err) = self.start_front(f.clone()).await {
                warn!(
                    front = f.name.as_str(),
                    error = %err,
                    "failed to restore enabled front on startup"
                );
            }
        }
    }

    /// Gracefully stops all active upstream listeners.
    pub async fn shutdown_all(&self) {
        let names: Vec<FrontName> = self.running_set().into_iter().collect();
        for name in names {
            self.stop_front(name).await;
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    router: Router,
    h2c_only: bool,
    cancel: CancellationToken,
) {
    let graceful = GracefulShutdown::new();
    loop {
        let accepted = tokio::select! {
            () = cancel.cancelled() => break,
            res = listener.accept() => res,
        };
        let Ok((tcp, _peer)) = accepted else {
            continue;
        };
        drop(tcp.set_nodelay(true));
        let svc = TowerToHyperService::new(router.clone());
        let tls_clone = tls.clone();
        let watcher = graceful.watcher();

        tokio::spawn(async move {
            let builder = AutoBuilder::new(TokioExecutor::new());
            if h2c_only {
                let h2_builder = builder.http2_only();
                if let Some(acceptor) = tls_clone {
                    if let Ok(stream) = acceptor.accept(tcp).await {
                        let conn = h2_builder.serve_connection(TokioIo::new(stream), svc);
                        drop(watcher.watch(conn).await);
                    }
                } else {
                    let conn = h2_builder.serve_connection(TokioIo::new(tcp), svc);
                    drop(watcher.watch(conn).await);
                }
                return;
            }
            if let Some(acceptor) = tls_clone {
                if let Ok(stream) = acceptor.accept(tcp).await {
                    let conn = builder.serve_connection_with_upgrades(TokioIo::new(stream), svc);
                    drop(watcher.watch(conn).await);
                }
            } else {
                let conn = builder.serve_connection_with_upgrades(TokioIo::new(tcp), svc);
                drop(watcher.watch(conn).await);
            }
        });
    }
    drop(tokio::time::timeout(Duration::from_secs(3), graceful.shutdown()).await);
}

/// Validates and builds a `tokio_rustls::TlsAcceptor` from `record.tls`.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] if certificate, private key, or client CA PEMs are invalid.
pub fn build_tls_acceptor(record: &FrontRecord) -> Result<TlsAcceptor, AppError> {
    drop(rustls::crypto::ring::default_provider().install_default());
    let cert_pem = record
        .tls
        .cert_pem
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput("缺少 TLS 证书 PEM".to_string()))?;
    let key_pem = record
        .tls
        .key_pem
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput("缺少 TLS 私钥 PEM".to_string()))?;

    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(cert_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::InvalidInput(format!("TLS 证书 PEM 解析失败: {e}")))?;
    if certs.is_empty() {
        return Err(AppError::InvalidInput("TLS 证书链为空".to_string()));
    }

    let priv_key = rustls_pemfile::private_key(&mut BufReader::new(key_pem.as_bytes()))
        .map_err(|e| AppError::InvalidInput(format!("TLS 私钥 PEM 解析失败: {e}")))?
        .ok_or_else(|| AppError::InvalidInput("未找到有效的 TLS 私钥".to_string()))?;

    let builder = ServerConfig::builder();
    let mut cfg = if let Some(ca_pem) = record
        .tls
        .client_ca_pem
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let mut roots = rustls::RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut BufReader::new(ca_pem.as_bytes())) {
            let c = cert.map_err(|e| AppError::InvalidInput(format!("客户端 CA 解析失败: {e}")))?;
            roots
                .add(c)
                .map_err(|e| AppError::InvalidInput(format!("客户端 CA 无效: {e}")))?;
        }
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|e| AppError::InvalidInput(format!("mTLS 校验器构建失败: {e}")))?;
        builder
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, priv_key)
            .map_err(|e| AppError::InvalidInput(format!("TLS 证书与私钥不匹配: {e}")))?
    } else {
        builder
            .with_no_client_auth()
            .with_single_cert(certs, priv_key)
            .map_err(|e| AppError::InvalidInput(format!("TLS 证书与私钥不匹配: {e}")))?
    };

    if record.name == FrontName::Grpc {
        cfg.alpn_protocols = vec![b"h2".to_vec()];
    } else if record.proto.http2 {
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    } else {
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    }
    Ok(TlsAcceptor::from(Arc::new(cfg)))
}
