//! Entrypoint binary for `x-upt`: CLI dispatch, control-plane server, and graceful signal drain.

mod api_assets;
mod api_control;
mod cli;
mod router;
mod security;
mod state;
mod static_files;

use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use x_upt_fronts::contracts::{graphql_sdl, grpc_fds, grpc_proto};
use x_upt_fronts::jobstore::JobStore;
use x_upt_fronts::logger::AccessLogger;
use x_upt_fronts::snapshot::SharedRuntime;
use x_upt_fronts::supervisor::FrontSupervisor;
use x_upt_store::Store;

use crate::cli::{Command, RunConfig, help_text, parse_cli};
use crate::router::build_admin_router;
use crate::state::AdminState;

fn main() -> ExitCode {
    init_tracing();
    let cmd = match parse_cli(std::env::args()) {
        Ok(c) => c,
        Err(msg) => {
            drop(writeln!(
                std::io::stderr(),
                "error: {msg}\n\n{}",
                help_text()
            ));
            return ExitCode::FAILURE;
        }
    };

    match cmd {
        Command::Help => {
            drop(write!(std::io::stdout(), "{}", help_text()));
            ExitCode::SUCCESS
        }
        Command::Version => {
            drop(writeln!(
                std::io::stdout(),
                "x-upt {}",
                env!("CARGO_PKG_VERSION")
            ));
            ExitCode::SUCCESS
        }
        Command::Check(cfg) => run_check(&cfg),
        Command::Serve(cfg) => {
            let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            else {
                error!("failed to build tokio runtime");
                return ExitCode::FAILURE;
            };
            rt.block_on(run_serve(cfg))
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    drop(
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .try_init(),
    );
}

fn run_check(cfg: &RunConfig) -> ExitCode {
    let store = match Store::open(&cfg.data_dir) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "database initialization check failed");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = store.build_snapshot() {
        error!(error = %e, "snapshot build check failed");
        return ExitCode::FAILURE;
    }
    if grpc_proto().is_empty() || grpc_fds().is_empty() || graphql_sdl().is_err() {
        error!("embedded protocol contract verification failed");
        return ExitCode::FAILURE;
    }
    info!(data_dir = %cfg.data_dir.display(), "x-upt check passed");
    ExitCode::SUCCESS
}

async fn run_serve(cfg: RunConfig) -> ExitCode {
    let store = match Store::open(&cfg.data_dir) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            error!(error = %e, "failed to open SQLite store");
            return ExitCode::FAILURE;
        }
    };
    let initial_snap = match store.build_snapshot() {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "failed to load initial snapshot");
            return ExitCode::FAILURE;
        }
    };
    let logger = match AccessLogger::open(&cfg.data_dir) {
        Ok(l) => l,
        Err(e) => {
            error!(error = %e, "failed to open access logger");
            return ExitCode::FAILURE;
        }
    };

    let root_cancel = CancellationToken::new();
    let runtime = SharedRuntime::new(initial_snap);
    let jobs = Arc::new(JobStore::new());
    let supervisor = FrontSupervisor::new(runtime, logger, jobs, root_cancel.clone());

    if let Ok(fronts) = store.raw_fronts() {
        supervisor.restore_enabled(&fronts).await;
    }

    let listener = match TcpListener::bind(cfg.admin_addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(addr = %cfg.admin_listen, error = %e, "failed to bind control-plane listener");
            supervisor.shutdown_all().await;
            return ExitCode::FAILURE;
        }
    };
    let local_port = listener
        .local_addr()
        .map_or(cfg.admin_addr.port(), |a| a.port());
    let admin_state = AdminState::new(
        cfg.admin_listen.clone(),
        local_port,
        cfg.admin_token,
        store,
        supervisor.clone(),
    );
    let app = build_admin_router(admin_state);

    info!(
        admin_listen = %cfg.admin_listen,
        data_dir = %cfg.data_dir.display(),
        "x-upt control plane ready"
    );

    let shutdown_cancel = root_cancel.clone();
    tokio::spawn(async move {
        wait_for_signal().await;
        info!("shutdown signal received, draining listeners");
        shutdown_cancel.cancel();
    });

    let serve_res = axum::serve(listener, app)
        .with_graceful_shutdown(root_cancel.cancelled_owned())
        .await;

    drop(tokio::time::timeout(Duration::from_secs(8), supervisor.shutdown_all()).await);

    if let Err(e) = serve_res {
        error!(error = %e, "control-plane server error");
        return ExitCode::FAILURE;
    }
    info!("x-upt stopped cleanly");
    ExitCode::SUCCESS
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut sigterm) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
            return;
        }
    }
    drop(tokio::signal::ctrl_c().await);
}
