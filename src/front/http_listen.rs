use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::extract::connect_info::ConnectInfo;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tower::Service;

use crate::config::{CommonFront, FrontName};
use crate::front::access::AccessMeta;

#[derive(Clone, Copy)]
pub enum HttpMode {
    Http1Only,
    Auto,
    Http2Only,
}

#[must_use]
pub fn http_mode(common: &CommonFront, grpc: bool) -> HttpMode {
    if grpc {
        return HttpMode::Http2Only;
    }
    let h2 = if common.tls.enabled {
        common.http2
    } else {
        common.h2c
    };
    if h2 {
        HttpMode::Auto
    } else {
        HttpMode::Http1Only
    }
}

pub async fn bind(host: &str, port: u16) -> Result<TcpListener, String> {
    TcpListener::bind((host, port))
        .await
        .map_err(|e| format!("Failed to bind {host}:{port}: {e}"))
}

pub fn load_tls(common: &CommonFront, grpc: bool) -> Result<Option<Arc<ServerConfig>>, String> {
    if !common.tls.enabled {
        return Ok(None);
    }
    let certs = load_certs(&common.tls.cert)?;
    let key = load_key(&common.tls.key)?;
    let builder = rustls::ServerConfig::builder();
    let mut config = if let Some(ca) = &common.tls.client_ca {
        let mut roots = RootCertStore::empty();
        for cert in load_certs(ca)? {
            roots
                .add(cert)
                .map_err(|e| format!("invalid client_ca: {e}"))?;
        }
        let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|e| format!("client verifier: {e}"))?;
        builder
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, key)
            .map_err(|e| format!("tls cert: {e}"))?
    } else {
        builder
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| format!("tls cert: {e}"))?
    };
    config.alpn_protocols = if grpc {
        vec![b"h2".to_vec()]
    } else if common.http2 {
        vec![b"h2".to_vec(), b"http/1.1".to_vec()]
    } else {
        vec![b"http/1.1".to_vec()]
    };
    Ok(Some(Arc::new(config)))
}

fn load_certs(path: &std::path::Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("parse certs {}: {e}", path.display()))
}

fn load_key(path: &std::path::Path) -> Result<PrivateKeyDer<'static>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| format!("parse key {}: {e}", path.display()))?
        .ok_or_else(|| format!("no private key in {}", path.display()))
}

pub fn spawn_http(
    listener: TcpListener,
    tls: Option<Arc<ServerConfig>>,
    mode: HttpMode,
    router: Router,
    meta: AccessMeta,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        serve_loop(listener, tls, mode, router, meta, shutdown).await;
    })
}

async fn serve_loop(
    listener: TcpListener,
    tls: Option<Arc<ServerConfig>>,
    mode: HttpMode,
    router: Router,
    meta: AccessMeta,
    shutdown: CancellationToken,
) {
    let acceptor = tls.map(TlsAcceptor::from);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, addr)) = accepted else { continue };
                let router = router.clone();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Some(acceptor) = acceptor {
                        let Ok(tls_stream) = acceptor.accept(stream).await else { return };
                        serve_io(TokioIo::new(tls_stream), addr, router, meta, mode).await;
                    } else {
                        serve_io(TokioIo::new(stream), addr, router, meta, mode).await;
                    }
                });
            }
        }
    }
}

async fn serve_io<I>(
    io: TokioIo<I>,
    addr: SocketAddr,
    router: Router,
    meta: AccessMeta,
    mode: HttpMode,
) where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |req: Request<Incoming>| {
        let router = router.clone();
        async move {
            let mut req = req.map(Body::new);
            req.extensions_mut().insert(ConnectInfo(addr));
            req.extensions_mut().insert(meta);
            let mut router = router;
            router.call(req).await
        }
    });

    let builder = AutoBuilder::new(TokioExecutor::new());
    let result = match mode {
        HttpMode::Http1Only => {
            builder
                .http1_only()
                .serve_connection_with_upgrades(io, service)
                .await
        }
        HttpMode::Http2Only => {
            builder
                .http2_only()
                .serve_connection_with_upgrades(io, service)
                .await
        }
        HttpMode::Auto => builder.serve_connection_with_upgrades(io, service).await,
    };
    let _ = result;
}

pub fn install_crypto() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "failed to install rustls crypto provider".to_string())
}

pub fn print_front(name: FrontName, common: &CommonFront) {
    let auth = if common.auth.is_empty() {
        "-".to_string()
    } else {
        common
            .auth
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(",")
    };
    println!(
        "front {} {}:{} tls={} auth={}",
        name.as_str(),
        common.host,
        common.port,
        common.tls.enabled,
        auth
    );
}
