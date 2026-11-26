pub mod access;
pub mod envelope;
pub mod graphql;
pub mod grpc;
pub mod http_listen;
pub mod job;
pub mod jsonrpc;
pub mod oauth_gateway;
pub mod rest;
pub mod sign_gateway;
pub mod soap;
pub mod util;
pub mod ws;

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::auth::token_hub::TokenHub;
use crate::config::{AppConfig, FrontName};
use crate::core::Core;
use crate::front::access::AccessMeta;
use crate::front::http_listen::{bind, http_mode, load_tls, print_front, spawn_http};
use crate::jobstore::JobStore;

pub async fn spawn_all(
    config: Arc<AppConfig>,
    core: Arc<Core>,
    tokens: Arc<TokenHub>,
    shutdown: CancellationToken,
) -> Result<Vec<JoinHandle<()>>, String> {
    let mut bound: Vec<(FrontName, TcpListener)> = Vec::new();
    for name in FrontName::all() {
        let common = config.fronts.common(name);
        if common.enabled {
            match bind(&common.host, common.port).await {
                Ok(listener) => bound.push((name, listener)),
                Err(err) => {
                    drop(bound);
                    return Err(err);
                }
            }
        } else {
            println!("front={} enabled=false", name.as_str());
        }
    }

    let mut handles = Vec::new();
    for (name, listener) in bound {
        let common = config.fronts.common(name);
        print_front(name, common);
        let meta = AccessMeta {
            front: name,
            port: common.port,
        };
        let bucket = tokens.bucket(name);
        let handle = match name {
            FrontName::OauthGateway => {
                let Some(bucket) = bucket else {
                    return Err("oauth_gateway requires a token bucket".to_string());
                };
                let tls = load_tls(common, false)?;
                let router =
                    oauth_gateway::router(Arc::clone(&config), Arc::clone(&core), bucket, meta);
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
            FrontName::SignGateway => {
                let tls = load_tls(common, false)?;
                let router = sign_gateway::router(Arc::clone(&config), Arc::clone(&core), meta);
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
            FrontName::Rest => {
                let tls = load_tls(common, false)?;
                let router = rest::router(Arc::clone(&config), Arc::clone(&core), bucket, meta);
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
            FrontName::Jsonrpc => {
                let tls = load_tls(common, false)?;
                let router = jsonrpc::router(Arc::clone(&config), Arc::clone(&core), bucket, meta);
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
            FrontName::Graphql => {
                let tls = load_tls(common, false)?;
                let router = graphql::router(Arc::clone(&config), Arc::clone(&core), bucket, meta);
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
            FrontName::Grpc => grpc::spawn(
                listener,
                Arc::clone(&config),
                Arc::clone(&core),
                bucket,
                shutdown.clone(),
                meta,
            )?,
            FrontName::Soap => {
                let tls = load_tls(common, false)?;
                let router = soap::router(Arc::clone(&config), Arc::clone(&core), meta);
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
            FrontName::Ws => {
                let tls = load_tls(common, false)?;
                let router = ws::router(Arc::clone(&config), Arc::clone(&core), bucket, meta);
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
            FrontName::Job => {
                let jobs = JobStore::new(
                    name,
                    config.job.expire_seconds,
                    Arc::clone(&core),
                    Arc::clone(&config),
                    config.fronts.job.workers,
                    shutdown.clone(),
                );
                let tls = load_tls(common, false)?;
                let router = job::router(Arc::clone(&config), bucket, jobs, meta);
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
            FrontName::EnvelopeTokenPlain
            | FrontName::EnvelopeTokenCrypt
            | FrontName::EnvelopeSignPlain
            | FrontName::EnvelopeSignCrypt
            | FrontName::EnvelopeCredPlain
            | FrontName::EnvelopeCredCrypt => {
                let token_front = matches!(
                    name,
                    FrontName::EnvelopeTokenPlain | FrontName::EnvelopeTokenCrypt
                );
                if token_front && bucket.is_none() {
                    return Err(format!("{} requires a token bucket", name.as_str()));
                }
                let tls = load_tls(common, false)?;
                let profiles = config.fronts.envelope(name).profiles.clone();
                let router = envelope::router(
                    Arc::clone(&config),
                    name,
                    profiles,
                    if token_front { bucket } else { None },
                    meta,
                );
                spawn_http(
                    listener,
                    tls,
                    http_mode(common, false),
                    router,
                    meta,
                    shutdown.clone(),
                )
            }
        };
        handles.push(handle);
    }
    Ok(handles)
}
