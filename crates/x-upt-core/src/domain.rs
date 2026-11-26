//! Core domain models, protocol front identifiers, and configuration structures.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

/// Method identifier for deterministic post search.
pub const METHOD_POSTS_SEARCH: &str = "x_upt.posts.search";
/// Method identifier for deterministic scene photo rendering.
pub const METHOD_PHOTOS_RENDER: &str = "x_upt.photos.render";
/// Method identifier for perceptual hash image comparison.
pub const METHOD_IMGDEF_COMPARE: &str = "x_upt.imgdef.compare";

/// Enumerates the 3 core deterministic business methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BusinessMethod {
    /// `x_upt.posts.search`
    #[serde(rename = "x_upt.posts.search")]
    PostsSearch,
    /// `x_upt.photos.render`
    #[serde(rename = "x_upt.photos.render")]
    PhotosRender,
    /// `x_upt.imgdef.compare`
    #[serde(rename = "x_upt.imgdef.compare")]
    ImgdefCompare,
}

impl BusinessMethod {
    /// Returns all 3 business methods in canonical display order.
    #[must_use]
    pub fn all() -> [Self; 3] {
        [Self::PostsSearch, Self::PhotosRender, Self::ImgdefCompare]
    }

    /// Canonical wire string for the method.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PostsSearch => METHOD_POSTS_SEARCH,
            Self::PhotosRender => METHOD_PHOTOS_RENDER,
            Self::ImgdefCompare => METHOD_IMGDEF_COMPARE,
        }
    }

    /// Short display label for UI cards.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::PostsSearch => "帖子检索 (posts.search)",
            Self::PhotosRender => "图片渲染 (photos.render)",
            Self::ImgdefCompare => "图片比对 (imgdef.compare)",
        }
    }

    /// Parses a method wire identifier (also accepts short suffix such as `posts.search`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            METHOD_POSTS_SEARCH | "posts.search" => Some(Self::PostsSearch),
            METHOD_PHOTOS_RENDER | "photos.render" => Some(Self::PhotosRender),
            METHOD_IMGDEF_COMPARE | "imgdef.compare" => Some(Self::ImgdefCompare),
            _ => None,
        }
    }
}

/// Checks whether a wire string matches one of the 3 known business methods.
#[must_use]
pub fn is_known_method(method: &str) -> bool {
    matches!(
        method,
        METHOD_POSTS_SEARCH | METHOD_PHOTOS_RENDER | METHOD_IMGDEF_COMPARE
    )
}

/// Active switch flags for the 3 core business methods.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodsState {
    /// Whether `x_upt.posts.search` is enabled.
    pub posts_search: bool,
    /// Whether `x_upt.photos.render` is enabled.
    pub photos_render: bool,
    /// Whether `x_upt.imgdef.compare` is enabled.
    pub imgdef_compare: bool,
}

impl MethodsState {
    /// Checks whether a given method wire name is currently enabled.
    #[must_use]
    pub fn is_enabled(self, method: &str) -> bool {
        match method {
            METHOD_POSTS_SEARCH => self.posts_search,
            METHOD_PHOTOS_RENDER => self.photos_render,
            METHOD_IMGDEF_COMPARE => self.imgdef_compare,
            _ => false,
        }
    }

    /// Checks whether a typed [`BusinessMethod`] is enabled.
    #[must_use]
    pub fn get(self, method: BusinessMethod) -> bool {
        match method {
            BusinessMethod::PostsSearch => self.posts_search,
            BusinessMethod::PhotosRender => self.photos_render,
            BusinessMethod::ImgdefCompare => self.imgdef_compare,
        }
    }

    /// Updates a single method flag in memory.
    pub fn set(&mut self, method: BusinessMethod, enabled: bool) {
        match method {
            BusinessMethod::PostsSearch => self.posts_search = enabled,
            BusinessMethod::PhotosRender => self.photos_render = enabled,
            BusinessMethod::ImgdefCompare => self.imgdef_compare = enabled,
        }
    }

    /// Count of currently enabled business methods.
    #[must_use]
    pub fn enabled_count(self) -> usize {
        usize::from(self.posts_search)
            + usize::from(self.photos_render)
            + usize::from(self.imgdef_compare)
    }
}

/// Enumerates all 15 simulated upstream protocol fronts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontName {
    /// OAuth 2.0 client_credentials JSON envelope gateway.
    OauthGateway,
    /// Form-encoded HMAC-SHA256 signed gateway.
    SignGateway,
    /// Multi-auth REST JSON/form/query/multipart front.
    Rest,
    /// JSON-RPC 2.0 single & batch front.
    Jsonrpc,
    /// GraphQL POST `/graphql` + GET `/sdl` front.
    Graphql,
    /// HTTP/2 cleartext/TLS unary gRPC front.
    Grpc,
    /// SOAP 1.1 XML + WSDL front.
    Soap,
    /// WebSocket unary JSON frame front.
    Ws,
    /// Async job submit & poll front.
    Job,
    /// Envelope token issue + plaintext call front.
    EnvelopeTokenPlain,
    /// Envelope token issue + encrypted call front.
    EnvelopeTokenCrypt,
    /// Envelope per-request signature + plaintext call front.
    EnvelopeSignPlain,
    /// Envelope per-request signature + encrypted call front.
    EnvelopeSignCrypt,
    /// Envelope static credential + plaintext call front.
    EnvelopeCredPlain,
    /// Envelope static credential + encrypted call front.
    EnvelopeCredCrypt,
}

impl FrontName {
    /// Returns all 15 fronts in canonical order.
    #[must_use]
    pub fn all() -> [Self; 15] {
        [
            Self::OauthGateway,
            Self::SignGateway,
            Self::Rest,
            Self::Jsonrpc,
            Self::Graphql,
            Self::Grpc,
            Self::Soap,
            Self::Ws,
            Self::Job,
            Self::EnvelopeTokenPlain,
            Self::EnvelopeTokenCrypt,
            Self::EnvelopeSignPlain,
            Self::EnvelopeSignCrypt,
            Self::EnvelopeCredPlain,
            Self::EnvelopeCredCrypt,
        ]
    }

    /// Canonical snake_case name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OauthGateway => "oauth_gateway",
            Self::SignGateway => "sign_gateway",
            Self::Rest => "rest",
            Self::Jsonrpc => "jsonrpc",
            Self::Graphql => "graphql",
            Self::Grpc => "grpc",
            Self::Soap => "soap",
            Self::Ws => "ws",
            Self::Job => "job",
            Self::EnvelopeTokenPlain => "envelope_token_plain",
            Self::EnvelopeTokenCrypt => "envelope_token_crypt",
            Self::EnvelopeSignPlain => "envelope_sign_plain",
            Self::EnvelopeSignCrypt => "envelope_sign_crypt",
            Self::EnvelopeCredPlain => "envelope_cred_plain",
            Self::EnvelopeCredCrypt => "envelope_cred_crypt",
        }
    }

    /// Parses a front name from snake_case string.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::all().into_iter().find(|f| f.as_str() == raw.trim())
    }

    /// Whether this front belongs to the 6 envelope fronts.
    #[must_use]
    pub fn is_envelope(self) -> bool {
        matches!(
            self,
            Self::EnvelopeTokenPlain
                | Self::EnvelopeTokenCrypt
                | Self::EnvelopeSignPlain
                | Self::EnvelopeSignCrypt
                | Self::EnvelopeCredPlain
                | Self::EnvelopeCredCrypt
        )
    }

    /// Whether this front is one of the 2 token-issuing envelope fronts.
    #[must_use]
    pub fn is_token_envelope(self) -> bool {
        matches!(self, Self::EnvelopeTokenPlain | Self::EnvelopeTokenCrypt)
    }

    /// Category string (`"core"` or `"envelope"`).
    #[must_use]
    pub fn category(self) -> &'static str {
        if self.is_envelope() {
            "envelope"
        } else {
            "core"
        }
    }

    /// Concise protocol display label.
    #[must_use]
    pub fn protocol_label(self) -> &'static str {
        match self {
            Self::OauthGateway => "HTTP OAuth Gateway",
            Self::SignGateway => "HTTP HMAC Form",
            Self::Rest => "REST HTTP/1.1",
            Self::Jsonrpc => "JSON-RPC 2.0",
            Self::Graphql => "GraphQL",
            Self::Grpc => "gRPC (HTTP/2)",
            Self::Soap => "SOAP 1.1 XML",
            Self::Ws => "WebSocket",
            Self::Job => "Async Job Poll",
            Self::EnvelopeTokenPlain => "Envelope Token + Plain",
            Self::EnvelopeTokenCrypt => "Envelope Token + Cipher",
            Self::EnvelopeSignPlain => "Envelope Sign + Plain",
            Self::EnvelopeSignCrypt => "Envelope Sign + Cipher",
            Self::EnvelopeCredPlain => "Envelope Cred + Plain",
            Self::EnvelopeCredCrypt => "Envelope Cred + Cipher",
        }
    }

    /// Default suggested listening port seeded into `SQLite` on initial setup.
    #[must_use]
    pub fn default_port(self) -> u16 {
        match self {
            Self::OauthGateway => 16000,
            Self::SignGateway => 16001,
            Self::Rest => 16002,
            Self::Jsonrpc => 16003,
            Self::Graphql => 16004,
            Self::Grpc => 16005,
            Self::Soap => 16006,
            Self::Ws => 16007,
            Self::Job => 16008,
            Self::EnvelopeTokenPlain => 16010,
            Self::EnvelopeTokenCrypt => 16011,
            Self::EnvelopeSignPlain => 16012,
            Self::EnvelopeSignCrypt => 16013,
            Self::EnvelopeCredPlain => 16014,
            Self::EnvelopeCredCrypt => 16015,
        }
    }

    /// Supported HTTP authentication modes for this front.
    #[must_use]
    pub fn supported_auth(self) -> &'static [AuthMethod] {
        match self {
            Self::OauthGateway => &[AuthMethod::OauthCc],
            Self::SignGateway => &[],
            Self::Rest => HTTP_AUTH_REST,
            Self::Jsonrpc | Self::Graphql | Self::Job => HTTP_AUTH_COMMON,
            Self::Grpc => &[
                AuthMethod::None,
                AuthMethod::ApiKey,
                AuthMethod::Bearer,
                AuthMethod::Jwt,
            ],
            Self::Soap => &[AuthMethod::None, AuthMethod::Basic, AuthMethod::ApiKey],
            Self::Ws => &[
                AuthMethod::None,
                AuthMethod::OauthCc,
                AuthMethod::ApiKey,
                AuthMethod::Bearer,
            ],
            Self::EnvelopeTokenPlain
            | Self::EnvelopeTokenCrypt
            | Self::EnvelopeSignPlain
            | Self::EnvelopeSignCrypt
            | Self::EnvelopeCredPlain
            | Self::EnvelopeCredCrypt => &[],
        }
    }

    /// Default authentication list seeded into `SQLite`.
    #[must_use]
    pub fn default_auth(self) -> Vec<AuthMethod> {
        match self {
            Self::OauthGateway => vec![AuthMethod::OauthCc],
            Self::SignGateway => Vec::new(),
            Self::Rest => vec![
                AuthMethod::OauthCc,
                AuthMethod::ApiKey,
                AuthMethod::Gettoken,
                AuthMethod::Basic,
                AuthMethod::Bearer,
                AuthMethod::Jwt,
                AuthMethod::Digest,
                AuthMethod::Hawk,
                AuthMethod::AwsSigv4,
            ],
            Self::Jsonrpc | Self::Graphql | Self::Job => vec![
                AuthMethod::OauthCc,
                AuthMethod::ApiKey,
                AuthMethod::Basic,
                AuthMethod::Bearer,
                AuthMethod::Jwt,
                AuthMethod::Digest,
                AuthMethod::Hawk,
                AuthMethod::AwsSigv4,
            ],
            Self::Grpc => vec![AuthMethod::ApiKey],
            Self::Soap => vec![AuthMethod::Basic],
            Self::Ws => vec![AuthMethod::OauthCc, AuthMethod::ApiKey],
            _ => Vec::new(),
        }
    }
}

const HTTP_AUTH_COMMON: &[AuthMethod] = &[
    AuthMethod::None,
    AuthMethod::OauthCc,
    AuthMethod::ApiKey,
    AuthMethod::Basic,
    AuthMethod::Bearer,
    AuthMethod::Jwt,
    AuthMethod::Digest,
    AuthMethod::Hawk,
    AuthMethod::AwsSigv4,
];

const HTTP_AUTH_REST: &[AuthMethod] = &[
    AuthMethod::None,
    AuthMethod::OauthCc,
    AuthMethod::ApiKey,
    AuthMethod::Gettoken,
    AuthMethod::Basic,
    AuthMethod::Bearer,
    AuthMethod::Jwt,
    AuthMethod::Digest,
    AuthMethod::Hawk,
    AuthMethod::AwsSigv4,
];

/// Supported HTTP/protocol authentication modes on core fronts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// Unauthenticated access.
    None,
    /// `X-API-Key` header/query or static Bearer secret.
    ApiKey,
    /// Static or OAuth `Authorization: Bearer`.
    Bearer,
    /// RFC 7617 HTTP Basic.
    Basic,
    /// RFC 6749 OAuth 2.0 `client_credentials` per-front token.
    OauthCc,
    /// Query `GET /gettoken?app_id=&secret=` token (REST front).
    Gettoken,
    /// JWT HS256/384/512 or RS256/384/512.
    Jwt,
    /// RFC 7616 HTTP Digest (MD5 and SHA-256 challenges).
    Digest,
    /// Hawk `Authorization: Hawk ...` SHA-256 MAC.
    Hawk,
    /// AWS Signature Version 4 (`AWS4-HMAC-SHA256`).
    AwsSigv4,
}

impl AuthMethod {
    /// Canonical snake_case string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ApiKey => "api_key",
            Self::Bearer => "bearer",
            Self::Basic => "basic",
            Self::OauthCc => "oauth_cc",
            Self::Gettoken => "gettoken",
            Self::Jwt => "jwt",
            Self::Digest => "digest",
            Self::Hawk => "hawk",
            Self::AwsSigv4 => "aws_sigv4",
        }
    }

    /// Parses an auth mode identifier.
    ///
    /// # Errors
    /// Returns an error string when `raw` is not a recognized authentication mode.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "none" => Ok(Self::None),
            "api_key" => Ok(Self::ApiKey),
            "bearer" => Ok(Self::Bearer),
            "basic" => Ok(Self::Basic),
            "oauth_cc" => Ok(Self::OauthCc),
            "gettoken" => Ok(Self::Gettoken),
            "jwt" => Ok(Self::Jwt),
            "digest" => Ok(Self::Digest),
            "hawk" => Ok(Self::Hawk),
            "aws_sigv4" => Ok(Self::AwsSigv4),
            other => Err(format!("unknown auth method: {other}")),
        }
    }
}

/// Global simulation settings persisted in `SQLite`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalSettings {
    /// Minimum simulated latency in milliseconds.
    pub latency_min_ms: u64,
    /// Maximum simulated latency in milliseconds.
    pub latency_max_ms: u64,
    /// Default token expiration duration in minutes.
    pub token_expire_minutes: u64,
    /// Background expired token cleanup interval in seconds.
    pub token_cleanup_seconds: u64,
    /// Maximum active tokens per client (`0` means unlimited).
    pub token_max_per_client: u64,
    /// Allowed clock skew window in seconds for signed requests.
    pub sign_window_seconds: u64,
    /// Replay-protection nonce retention TTL in seconds.
    pub sign_nonce_ttl_seconds: u64,
    /// Async job expiration timeout in seconds.
    pub job_expire_seconds: u64,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            latency_min_ms: 20,
            latency_max_ms: 100,
            token_expire_minutes: 30,
            token_cleanup_seconds: 300,
            token_max_per_client: 0,
            sign_window_seconds: 300,
            sign_nonce_ttl_seconds: 600,
            job_expire_seconds: 600,
        }
    }
}

/// TLS configuration flags for a front listener.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontTlsConfig {
    /// Whether TLS is enabled on the listener.
    pub enabled: bool,
    /// Server certificate chain in PEM format.
    pub cert_pem: Option<String>,
    /// Server private key in PEM format.
    pub key_pem: Option<String>,
    /// Optional client CA root certificate in PEM format for mTLS transport.
    pub client_ca_pem: Option<String>,
}

/// Protocol listener configuration for a single upstream front.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontProtocolFlags {
    /// Enable HTTP/2 over TLS ALPN.
    pub http2: bool,
    /// Enable HTTP/2 cleartext (h2c).
    pub h2c: bool,
}

/// Persisted configuration of one upstream front.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontRecord {
    /// Front identifier.
    pub name: FrontName,
    /// Persisted enabled switch state.
    pub enabled: bool,
    /// Bind IP or hostname.
    pub host: String,
    /// Bind TCP port.
    pub port: u16,
    /// HTTP/2 and h2c flags.
    pub proto: FrontProtocolFlags,
    /// Per-front token TTL override in minutes.
    pub expire_minutes: u64,
    /// Allowed HTTP authentication modes.
    pub auth: Vec<AuthMethod>,
    /// TLS configuration.
    pub tls: FrontTlsConfig,
    /// Max concurrent WebSocket connections (`ws` front).
    pub ws_max_conn: usize,
    /// WebSocket ping interval in seconds (`ws` front).
    pub ws_ping_secs: u64,
    /// Async worker task count (`job` front).
    pub job_workers: usize,
}

impl FrontRecord {
    /// Builds the initial disabled default record for `name`.
    #[must_use]
    pub fn default_for(name: FrontName) -> Self {
        let is_grpc = name == FrontName::Grpc;
        Self {
            name,
            enabled: false,
            host: "0.0.0.0".to_string(),
            port: name.default_port(),
            proto: FrontProtocolFlags {
                http2: is_grpc,
                h2c: is_grpc,
            },
            expire_minutes: 30,
            auth: name.default_auth(),
            tls: FrontTlsConfig::default(),
            ws_max_conn: 1024,
            ws_ping_secs: 30,
            job_workers: 4,
        }
    }

    /// Whether this front needs an OAuth/Gettoken bucket when running.
    #[must_use]
    pub fn needs_token_bucket(&self) -> bool {
        self.name.is_token_envelope()
            || self
                .auth
                .iter()
                .any(|a| matches!(*a, AuthMethod::OauthCc | AuthMethod::Gettoken))
    }
}

/// Persisted caller credential (`apps` table).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRecord {
    /// Unique caller identifier (`1`..=`64` chars).
    pub client_id: String,
    /// Shared secret (`8`..=`128` bytes).
    pub client_secret: String,
    /// Display name (`<= 64` chars).
    pub name: String,
    /// Optional RSA public key PEM for RS* JWT verification.
    pub rsa_public_pem: Option<String>,
    /// Granted business method wire names.
    pub methods: Vec<String>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// Authenticated caller context attached to a verified request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthedApp {
    /// Verified client ID.
    pub client_id: String,
    /// Set of business method wire names granted to this caller.
    pub methods: HashSet<String>,
}

impl From<&AppRecord> for AuthedApp {
    fn from(app: &AppRecord) -> Self {
        Self {
            client_id: app.client_id.clone(),
            methods: app.methods.iter().cloned().collect(),
        }
    }
}

/// In-memory lookup registry of caller credentials.
#[derive(Debug, Clone, Default)]
pub struct AppRegistry {
    by_id: HashMap<String, AppRecord>,
}

impl AppRegistry {
    /// Builds a registry from persisted [`AppRecord`] items.
    #[must_use]
    pub fn from_records(records: Vec<AppRecord>) -> Self {
        let mut by_id = HashMap::new();
        for item in records {
            by_id.insert(item.client_id.clone(), item);
        }
        Self { by_id }
    }

    /// Looks up a caller by `client_id`.
    #[must_use]
    pub fn get(&self, client_id: &str) -> Option<&AppRecord> {
        self.by_id.get(client_id)
    }

    /// Finds a caller by constant-time `client_secret` comparison.
    #[must_use]
    pub fn find_by_secret(&self, secret: &str) -> Option<&AppRecord> {
        self.by_id
            .values()
            .find(|app| secrets_equal(&app.client_secret, secret))
    }

    /// Iterates over all registered callers.
    pub fn iter(&self) -> impl Iterator<Item = &AppRecord> {
        self.by_id.values()
    }

    /// Number of registered callers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether zero callers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Constant-time string equality for secrets.
#[must_use]
pub fn secrets_equal(left: &str, right: &str) -> bool {
    secrets_equal_bytes(left.as_bytes(), right.as_bytes())
}

/// Constant-time byte-slice equality for secrets and MAC tags.
#[must_use]
pub fn secrets_equal_bytes(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.ct_eq(right).into()
}

/// Filters caller-granted methods by globally enabled business methods.
#[must_use]
pub fn granted_methods(app_methods: &[String], state: MethodsState) -> HashSet<String> {
    app_methods
        .iter()
        .filter(|m| state.is_enabled(m))
        .cloned()
        .collect()
}

/// OAuth token request form fields.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TokenForm {
    /// OAuth grant_type (`client_credentials`).
    #[serde(default)]
    pub grant_type: String,
    /// Optional form client_id.
    pub client_id: Option<String>,
    /// Optional form client_secret.
    pub client_secret: Option<String>,
    /// Optional space-delimited scope string.
    pub scope: Option<String>,
}

/// OAuth 2.0 token success body.
#[derive(Debug, Clone, Serialize)]
pub struct TokenResponse {
    /// Issued bearer token.
    pub access_token: String,
    /// Token type (`Bearer`).
    pub token_type: String,
    /// Lifetime in seconds.
    pub expires_in: u64,
    /// Granted space-delimited method scope.
    pub scope: String,
}

/// OAuth 2.0 error body.
#[derive(Debug, Clone, Serialize)]
pub struct OAuthError {
    /// RFC 6749 error code.
    pub error: String,
    /// Error description.
    pub error_description: String,
}

/// Stored OAuth token entry.
#[derive(Debug, Clone)]
pub struct TokenEntry {
    /// Owning client ID.
    pub client_id: String,
    /// Granted methods for this token.
    pub methods: HashSet<String>,
    /// Expiration timestamp.
    pub expires_at: DateTime<Utc>,
}

/// Standard JSON gateway envelope returned by `oauth_gateway`.
#[derive(Debug, Clone, Serialize)]
pub struct GatewayEnvelope<T: Serialize> {
    /// Numeric business status code (`0` on success).
    pub code: i32,
    /// Status message.
    pub msg: String,
    /// Echoed or generated request ID.
    pub request_id: String,
    /// Response payload.
    pub data: T,
}

/// Envelope status code: success.
pub const CODE_OK: i32 = 0;
/// Envelope status code: bad request.
pub const CODE_BAD_REQUEST: i32 = 40001;
/// Envelope status code: missing/malformed credentials.
pub const CODE_UNAUTHORIZED: i32 = 40100;
/// Envelope status code: invalid/expired token.
pub const CODE_INVALID_TOKEN: i32 = 40102;
/// Envelope status code: method not granted.
pub const CODE_FORBIDDEN: i32 = 40300;
/// Envelope status code: unknown or disabled method.
pub const CODE_UNKNOWN_METHOD: i32 = 40400;
/// Envelope status code: resource not found.
pub const CODE_NOT_FOUND: i32 = 40401;
/// Envelope status code: schema validation failure.
pub const CODE_UNPROCESSABLE: i32 = 42200;
/// Envelope status code: internal failure.
pub const CODE_INTERNAL: i32 = 50000;
/// Envelope message: success.
pub const MSG_OK: &str = "ok";

/// Kind of secret material stored for envelope profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    /// Plain UTF-8 credential string.
    Text,
    /// Raw binary symmetric key represented in hex on upload.
    BytesHex,
    /// PEM-encoded asymmetric key.
    Pem,
}

impl SecretKind {
    /// Canonical snake_case label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::BytesHex => "bytes_hex",
            Self::Pem => "pem",
        }
    }

    /// Parses a secret kind string.
    ///
    /// # Errors
    /// Returns an error string if `raw` is unknown.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "text" => Ok(Self::Text),
            "bytes_hex" => Ok(Self::BytesHex),
            "pem" => Ok(Self::Pem),
            other => Err(format!("unknown secret_kind: {other}")),
        }
    }
}

/// Persisted envelope key/secret record.
#[derive(Debug, Clone)]
pub struct EnvelopeSecretRecord {
    /// Unique secret name referenced by envelope profiles.
    pub name: String,
    /// Secret encoding kind.
    pub kind: SecretKind,
    /// Raw key or UTF-8 credential bytes.
    pub raw_bytes: Vec<u8>,
    /// Short SHA-256 hex fingerprint for redacted display.
    pub fingerprint: String,
    /// Optional public material (PEM or hex) when safe to expose.
    pub public_material: Option<String>,
    /// RFC 3339 update timestamp.
    pub updated_at: String,
}

/// Serializable field slot specification inside an envelope profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeFieldDto {
    /// Parameter or header field name.
    pub name: String,
    /// Parameter location (`header`, `query`, `form`, `json`).
    pub slot: String,
    /// Semantic role (`auth`, `cipher`, `biz`, `sign`, `token`, `wrap_key`, `timestamp`, `nonce`).
    pub role: String,
}

/// Serializable envelope profile specification stored in `SQLite`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeProfileSpec {
    /// Unique profile name within the front.
    pub name: String,
    /// Profile kind (`issue` or `call`).
    pub kind: String,
    /// Allowed HTTP methods (`GET`, `POST`).
    pub methods: Vec<String>,
    /// Request path starting with `/`.
    pub path: String,
    /// Request body kind (`json`, `form`, `raw`, `empty`).
    pub body_kind: String,
    /// Outer body encoding (`identity`, `base64`).
    pub body_encoding: String,
    /// Optional 4-slot priority override.
    #[serde(default)]
    pub slot_priority: Option<Vec<String>>,
    /// Explicit slot field mappings.
    #[serde(default)]
    pub fields: Vec<EnvelopeFieldDto>,
    /// Whether unnamed body/query fields enter `biz`.
    #[serde(default)]
    pub biz_rest: Option<bool>,
    /// Required header token prefix (e.g. `"Bearer "`).
    #[serde(default)]
    pub token_prefix: Option<String>,
    /// Response JSON field name for issued tokens.
    #[serde(default)]
    pub token_response_field: Option<String>,
    /// Map of `auth_field_name -> secret_name` in `envelope_secrets`.
    #[serde(default)]
    pub secrets: HashMap<String, String>,
    /// Canonicalization mode (`sorted_kv`, `concat`, `raw`).
    #[serde(default)]
    pub canon: Option<String>,
    /// Fields skipped during canonicalization.
    #[serde(default)]
    pub canon_skip: Option<Vec<String>>,
    /// Delimiter between canonicalized parts.
    #[serde(default)]
    pub canon_sep: Option<String>,
    /// Prepend signing/cipher key to canonical string.
    #[serde(default)]
    pub canon_prefix_key: Option<bool>,
    /// Append signing/cipher key to canonical string.
    #[serde(default)]
    pub canon_suffix_key: Option<bool>,
    /// Explicit field order for `concat` or `raw`.
    #[serde(default)]
    pub canon_fields: Option<Vec<String>>,
    /// Signature algorithm name.
    #[serde(default)]
    pub sign_alg: Option<String>,
    /// Hex casing (`lower` or `upper`).
    #[serde(default)]
    pub hex_case: Option<String>,
    /// Signature output encoding (`hex` or `base64`).
    #[serde(default)]
    pub sign_encoding: Option<String>,
    /// Encryption algorithm name.
    #[serde(default)]
    pub crypt_alg: Option<String>,
    /// Cipher scope (`fields` or `body`).
    #[serde(default)]
    pub cipher_scope: Option<String>,
    /// Encrypted field names.
    #[serde(default)]
    pub cipher_fields: Option<Vec<String>>,
    /// Ciphertext encoding (`base64` or `hex`).
    #[serde(default)]
    pub crypt_encoding: Option<String>,
    /// Padding mode for `aes_cfb` (`none` or `pkcs5`).
    #[serde(default)]
    pub padding: Option<String>,
    /// CFB segment bits (`8`..=`128`).
    #[serde(default)]
    pub segment_bits: Option<usize>,
    /// GCM IV length in bytes (`12` or `16`).
    #[serde(default)]
    pub iv_len: Option<usize>,
    /// IV source (`key`, `random_prefix`, `field`).
    #[serde(default)]
    pub iv_from: Option<String>,
    /// GCM tag length in bits (`128`).
    #[serde(default)]
    pub tag_bits: Option<usize>,
    /// SM2 signing user ID.
    #[serde(default)]
    pub user_id: Option<String>,
    /// SM2 ciphertext layout (`c1c3c2` or `c1c2c3`).
    #[serde(default)]
    pub sm2_mode: Option<String>,
    /// Symmetric encryption secret name in `envelope_secrets`.
    #[serde(default)]
    pub key: Option<String>,
    /// HMAC signing secret name in `envelope_secrets`.
    #[serde(default)]
    pub sign_key: Option<String>,
    /// Asymmetric private key secret name in `envelope_secrets`.
    #[serde(default)]
    pub private_key: Option<String>,
    /// Asymmetric public key secret name in `envelope_secrets`.
    #[serde(default)]
    pub public_key: Option<String>,
    /// Key wrapping algorithm (`rsa_pkcs1_public` or `sm2_encrypt`).
    #[serde(default)]
    pub wrap: Option<String>,
    /// Sign/encrypt execution order (`sign_then_encrypt` or `encrypt_then_sign`).
    #[serde(default)]
    pub order: Option<String>,
    /// Error delivery channel (`http` or `envelope`).
    pub fail_in: String,
    /// Optional response body kind override.
    #[serde(default)]
    pub response_body_kind: Option<String>,
    /// Optional response body encoding override.
    #[serde(default)]
    pub response_encoding: Option<String>,
    /// Optional response encryption override (`none` for plaintext response).
    #[serde(default)]
    pub response_crypt_alg: Option<String>,
}
