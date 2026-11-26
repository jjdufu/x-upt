//! Same-source protocol contract providers (`grpc.proto`, `grpc.fds`, `soap.wsdl`, `graphql.sdl`, `upstream-openapi.json`).

use x_upt_core::docs_model::build_upstream_openapi;
use x_upt_core::domain::FrontRecord;
use x_upt_store::dto::RuntimeSnapshotData;

use crate::fronts_graphql::export_graphql_sdl;

const GRPC_PROTO_SRC: &str = include_str!("../../../proto/x_upt.proto");
const GRPC_FDS_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/x_upt_descriptor.bin"));
const SOAP_WSDL_TEMPLATE: &str = include_str!("../../../spec/x_upt.wsdl");

/// Returns the canonical `proto/x_upt.proto` source string.
#[must_use]
pub fn grpc_proto() -> &'static str {
    GRPC_PROTO_SRC
}

/// Returns the compiled binary `FileDescriptorSet` for `proto/x_upt.proto`.
#[must_use]
pub fn grpc_fds() -> &'static [u8] {
    GRPC_FDS_BYTES
}

/// Renders `spec/x_upt.wsdl` with `soap` front's live configured scheme, host, and port.
#[must_use]
pub fn soap_wsdl(front: &FrontRecord) -> String {
    let scheme = if front.tls.enabled { "https" } else { "http" };
    let host_raw = front.host.trim();
    let host = if host_raw.is_empty() || host_raw == "0.0.0.0" || host_raw == "::" {
        "127.0.0.1"
    } else {
        host_raw
    };
    let loc = format!("{scheme}://{host}:{}/soap", front.port);
    SOAP_WSDL_TEMPLATE.replace("__SOAP_LOCATION__", &loc)
}

/// Exports the live dynamic GraphQL SDL string.
///
/// # Errors
/// Returns an error string if schema construction fails.
pub fn graphql_sdl() -> Result<String, String> {
    export_graphql_sdl()
}

/// Generates the live OpenAPI 3.0 JSON specification across all HTTP-accessible upstream fronts.
#[must_use]
pub fn upstream_openapi(snap: &RuntimeSnapshotData) -> serde_json::Value {
    build_upstream_openapi(&snap.front_list, &snap.profiles_by_front)
}
