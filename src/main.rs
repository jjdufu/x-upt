mod auth;
mod config;
mod core;
mod envelope;
mod front;
mod jobstore;
mod logger;
mod service;
mod types;

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::auth::token_hub::TokenHub;
use crate::core::Core;
use crate::front::http_listen::install_crypto;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let config = Arc::new(config::load()?);
    install_crypto()?;

    let enabled: Vec<&str> = config
        .fronts
        .enabled_names()
        .iter()
        .map(|n| n.as_str())
        .collect();
    logger::init(&config.log.path, config.log.split_by_front, &enabled)?;

    let core = Arc::new(Core {
        imgs_dir: config.imgs_dir.clone(),
        methods: config.methods,
    });
    let tokens = Arc::new(TokenHub::new(&config));
    let shutdown = CancellationToken::new();

    let handles = front::spawn_all(Arc::clone(&config), core, tokens, shutdown.clone()).await?;

    shutdown_signal().await;
    println!("Shutdown signal received, exiting gracefully...");
    shutdown.cancel();
    let join = async {
        for handle in handles {
            let _ = handle.await;
        }
    };
    if tokio::time::timeout(Duration::from_secs(10), join)
        .await
        .is_err()
    {
        eprintln!("graceful shutdown timed out, aborting");
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .unwrap_or_else(|_| eprintln!("Failed to install Ctrl+C handler"));
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                eprintln!("Failed to install SIGTERM handler: {e}");
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
