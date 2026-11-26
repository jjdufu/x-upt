#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]
//! Dynamic protocol routers, same-source contracts, and `FrontSupervisor` for `x-upt`.

pub mod contracts;
mod fronts_gateway;
mod fronts_graphql;
mod fronts_grpc;
mod fronts_rest_rpc;
mod fronts_soap;
mod fronts_ws_job_env;
pub mod http_guard;
pub mod jobstore;
pub mod logger;
pub mod snapshot;
pub mod supervisor;
pub mod token;

use axum::Router;
use x_upt_core::domain::{FrontName, FrontRecord};

use crate::http_guard::FrontCtx;

/// Builds the protocol-specific `axum::Router` for `record`.
pub fn build_front_router(record: &FrontRecord, ctx: FrontCtx) -> Router {
    match record.name {
        FrontName::OauthGateway => fronts_gateway::build_oauth_gateway_router(ctx),
        FrontName::SignGateway => fronts_gateway::build_sign_gateway_router(ctx),
        FrontName::Rest => fronts_rest_rpc::build_rest_router(ctx),
        FrontName::Jsonrpc => fronts_rest_rpc::build_jsonrpc_router(ctx),
        FrontName::Graphql => fronts_graphql::build_graphql_router(ctx),
        FrontName::Grpc => fronts_grpc::build_grpc_router(ctx),
        FrontName::Soap => fronts_soap::build_soap_router(ctx),
        FrontName::Ws => {
            fronts_ws_job_env::build_ws_router(ctx, record.ws_max_conn, record.ws_ping_secs)
        }
        FrontName::Job => fronts_ws_job_env::build_job_router(ctx),
        FrontName::EnvelopeTokenPlain
        | FrontName::EnvelopeTokenCrypt
        | FrontName::EnvelopeSignPlain
        | FrontName::EnvelopeSignCrypt
        | FrontName::EnvelopeCredPlain
        | FrontName::EnvelopeCredCrypt => fronts_ws_job_env::build_envelope_router(ctx),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tower::ServiceExt;
    use x_upt_core::domain::{FrontName, FrontRecord};
    use x_upt_store::dto::RuntimeSnapshotData;

    use crate::build_front_router;
    use crate::contracts::{graphql_sdl, grpc_fds, grpc_proto, soap_wsdl};
    use crate::http_guard::FrontCtx;
    use crate::jobstore::JobStore;
    use crate::logger::AccessLogger;
    use crate::snapshot::SharedRuntime;
    use crate::token::FrontSecurityState;

    #[tokio::test]
    async fn upstream_fronts_do_not_expose_admin_or_health_and_contracts_render()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(grpc_proto().contains("service XUpt"));
        assert!(!grpc_fds().is_empty());
        let soap_rec = FrontRecord::default_for(FrontName::Soap);
        assert!(soap_wsdl(&soap_rec).contains("127.0.0.1:16006/soap"));
        assert!(graphql_sdl()?.contains("postsSearch"));

        let rest_rec = FrontRecord::default_for(FrontName::Rest);
        let ctx = FrontCtx {
            front: FrontName::Rest,
            port: rest_rec.port,
            runtime: SharedRuntime::new(RuntimeSnapshotData::default()),
            sec: FrontSecurityState::new(30, 600),
            logger: AccessLogger::in_memory(),
            jobs: Arc::new(JobStore::new()),
        };
        let router = build_front_router(&rest_rec, ctx);

        for forbidden_path in ["/health", "/api/v1/status"] {
            let req = http::Request::builder()
                .method("GET")
                .uri(forbidden_path)
                .body(axum::body::Body::empty())?;
            let resp = router.clone().oneshot(req).await?;
            assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
        }
        Ok(())
    }
}
