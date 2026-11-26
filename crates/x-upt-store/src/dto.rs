//! Control-plane DTOs and compiled in-memory runtime snapshot types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use x_upt_core::domain::{
    AppRegistry, EnvelopeProfileSpec, FrontName, FrontRecord, GlobalSettings,
};
use x_upt_core::preconditions::{MethodStatus, SceneCounts};
use x_upt_core::services::{CoreDataset, Post};
use x_upt_crypto::profile::Profile;

/// Readiness and configuration view of a single upstream front (`FrontStatus`).
#[derive(Debug, Clone, Serialize)]
pub struct FrontStatusView {
    /// Canonical snake_case front name.
    pub name: String,
    /// `"core"` or `"envelope"`.
    pub category: &'static str,
    /// Protocol display label.
    pub protocol: &'static str,
    /// Persisted switch state.
    pub enabled: bool,
    /// Whether the listener task is currently bound and running.
    pub running: bool,
    /// Configured bind host.
    pub host: String,
    /// Configured bind TCP port.
    pub port: u16,
    /// HTTP/2 over TLS flag.
    pub http2: bool,
    /// HTTP/2 cleartext (h2c) flag.
    pub h2c: bool,
    /// Per-front token TTL in minutes.
    pub expire_minutes: u64,
    /// Configured authentication methods.
    pub auth: Vec<String>,
    /// Supported authentication methods for this front.
    pub supported_auth: Vec<String>,
    /// Whether TLS is enabled.
    pub tls_enabled: bool,
    /// Whether server certificate PEM is configured.
    pub has_tls_cert: bool,
    /// Whether server private key PEM is configured.
    pub has_tls_key: bool,
    /// Whether optional client CA PEM is configured.
    pub has_tls_client_ca: bool,
    /// Max concurrent WebSocket connections.
    pub ws_max_conn: usize,
    /// WebSocket ping interval in seconds.
    pub ws_ping_secs: u64,
    /// Async job worker count.
    pub job_workers: usize,
    /// Configured envelope profile count.
    pub profile_count: usize,
    /// Whether all preconditions are met to enable this front.
    pub can_enable: bool,
    /// Human-readable missing precondition reasons.
    pub missing_preconditions: Vec<String>,
}

/// Request payload for `PUT /api/v1/fronts/{front}`.
#[derive(Debug, Clone, Deserialize)]
pub struct FrontUpdatePayload {
    /// Bind host.
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Enable HTTP/2 ALPN.
    pub http2: bool,
    /// Enable HTTP/2 cleartext.
    pub h2c: bool,
    /// Token expiration minutes.
    pub expire_minutes: u64,
    /// Allowed authentication modes.
    #[serde(default)]
    pub auth: Vec<String>,
    /// Enable TLS listener.
    pub tls_enabled: bool,
    /// Optional server certificate PEM (`None` preserves existing when non-empty not provided, `Some("")` clears).
    pub tls_cert_pem: Option<String>,
    /// Optional server private key PEM.
    pub tls_key_pem: Option<String>,
    /// Optional client CA PEM.
    pub tls_client_ca_pem: Option<String>,
    /// WebSocket connection limit.
    pub ws_max_conn: usize,
    /// WebSocket ping interval seconds.
    pub ws_ping_secs: u64,
    /// Async job worker count.
    pub job_workers: usize,
}

/// Counts summary inside [`SystemStatusView`].
#[derive(Debug, Clone, Serialize)]
pub struct SystemCountsView {
    /// Total imported images.
    pub images: usize,
    /// Imported images grouped by scene.
    pub images_by_scene: SceneCounts,
    /// Total imported posts.
    pub posts: usize,
    /// Registered caller credentials.
    pub apps: usize,
    /// Stored envelope secrets.
    pub envelope_secrets: usize,
    /// Stored envelope profiles across all 6 envelope fronts.
    pub envelope_profiles: usize,
}

/// Response payload for `GET /api/v1/status`.
#[derive(Debug, Clone, Serialize)]
pub struct SystemStatusView {
    /// Binary version string.
    pub version: String,
    /// Control-plane bind address.
    pub admin_listen: String,
    /// Asset and credential counts.
    pub counts: SystemCountsView,
    /// Global simulation parameters.
    pub settings: GlobalSettings,
    /// Business method readiness states.
    pub methods: Vec<MethodStatus>,
    /// All 15 upstream front readiness states.
    pub fronts: Vec<FrontStatusView>,
}

/// Input payload for creating or updating a post.
#[derive(Debug, Clone, Deserialize)]
pub struct PostInput {
    /// Optional explicit post ID (auto-assigned when omitted on create).
    pub id: Option<u64>,
    /// Author user ID (`userId` in JSON).
    #[serde(rename = "userId", alias = "user_id")]
    pub user_id: u64,
    /// Post title.
    pub title: String,
    /// Post body text.
    pub body: String,
}

/// Paginated post list response for `GET /api/v1/posts`.
#[derive(Debug, Clone, Serialize)]
pub struct PostsPageView {
    /// Page items.
    pub items: Vec<Post>,
    /// Total matching rows.
    pub total: usize,
    /// 1-based page index.
    pub page: u32,
    /// Page size.
    pub page_size: u32,
}

/// Redacted caller credential metadata returned by `/api/v1/apps`.
#[derive(Debug, Clone, Serialize)]
pub struct AppMetaView {
    /// Unique client identifier.
    pub client_id: String,
    /// Display name.
    pub name: String,
    /// Always `true` when secret is set.
    pub has_secret: bool,
    /// Masked secret preview (never reveals full secret).
    pub secret_preview: String,
    /// Whether RSA public key PEM is configured.
    pub has_rsa_public_pem: bool,
    /// Granted business method wire names.
    pub methods: Vec<String>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// Payload for creating or updating a caller credential.
#[derive(Debug, Clone, Deserialize)]
pub struct AppUpsertPayload {
    /// Unique client identifier.
    pub client_id: String,
    /// Shared secret (`8`..=`128` bytes; on update, empty string preserves existing secret).
    #[serde(default)]
    pub client_secret: String,
    /// Display name.
    pub name: String,
    /// Optional RSA public key PEM.
    pub rsa_public_pem: Option<String>,
    /// Granted business method wire names.
    pub methods: Vec<String>,
}

/// Redacted envelope secret metadata returned by `/api/v1/envelopes/secrets`.
#[derive(Debug, Clone, Serialize)]
pub struct SecretMetaView {
    /// Unique secret name.
    pub name: String,
    /// Secret kind (`text`, `bytes_hex`, `pem`).
    pub secret_kind: String,
    /// Raw byte length.
    pub byte_len: usize,
    /// Short SHA-256 fingerprint.
    pub fingerprint: String,
    /// Public key material when safe to expose.
    pub public_material: Option<String>,
    /// RFC 3339 update timestamp.
    pub updated_at: String,
}

/// Payload for uploading or generating an envelope secret.
#[derive(Debug, Clone, Deserialize)]
pub struct SecretUpsertPayload {
    /// Base secret name.
    pub name: String,
    /// Optional generation mode (`bytes_8`, `bytes_16`, `bytes_24`, `bytes_32`, `text_24`, `rsa_2048`, `sm2`).
    pub generate: Option<String>,
    /// Explicit secret kind when uploading raw material (`text`, `bytes_hex`, `pem`).
    pub secret_kind: Option<String>,
    /// Raw secret value when uploading (`text` UTF-8, `bytes_hex` hex string, or `pem` text).
    pub value: Option<String>,
}

/// Envelope profiles response for `GET/PUT /api/v1/envelopes/{front}/profiles`.
#[derive(Debug, Clone, Serialize)]
pub struct EnvelopeProfilesView {
    /// Front snake_case name.
    pub front: String,
    /// Ordered profile specifications.
    pub profiles: Vec<EnvelopeProfileSpec>,
}

/// Immutable runtime snapshot built from `SQLite` and `<data-dir>/images/` for lock-free request serving.
#[derive(Debug, Clone, Default)]
pub struct RuntimeSnapshotData {
    /// Global simulation parameters.
    pub settings: GlobalSettings,
    /// Deterministic business dataset and method switches.
    pub dataset: CoreDataset,
    /// Registered caller credentials.
    pub apps: AppRegistry,
    /// Persisted front configurations indexed by name.
    pub fronts: HashMap<FrontName, FrontRecord>,
    /// Persisted fronts in canonical order.
    pub front_list: Vec<FrontRecord>,
    /// Raw envelope profile specs per front (used by live docs & OpenAPI generator).
    pub profiles_by_front: Vec<(FrontName, Vec<EnvelopeProfileSpec>)>,
    /// Compiled envelope cryptographic profiles per front.
    pub compiled_envelopes: HashMap<FrontName, Vec<Profile>>,
}
