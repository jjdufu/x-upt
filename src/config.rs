use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::service::apps::{AppRegistry, AppToml};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrontName {
    OauthGateway,
    SignGateway,
    Rest,
    Jsonrpc,
    Graphql,
    Grpc,
    Soap,
    Ws,
    Job,
    EnvelopeTokenPlain,
    EnvelopeTokenCrypt,
    EnvelopeSignPlain,
    EnvelopeSignCrypt,
    EnvelopeCredPlain,
    EnvelopeCredCrypt,
}

impl FrontName {
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

    #[must_use]
    pub fn default_port(self) -> u16 {
        match self {
            Self::OauthGateway => 3000,
            Self::SignGateway => 3001,
            Self::Rest => 3002,
            Self::Jsonrpc => 3003,
            Self::Graphql => 3004,
            Self::Grpc => 3005,
            Self::Soap => 3006,
            Self::Ws => 3007,
            Self::Job => 3008,
            Self::EnvelopeTokenPlain => 3010,
            Self::EnvelopeTokenCrypt => 3011,
            Self::EnvelopeSignPlain => 3012,
            Self::EnvelopeSignCrypt => 3013,
            Self::EnvelopeCredPlain => 3014,
            Self::EnvelopeCredCrypt => 3015,
        }
    }

    #[must_use]
    pub fn supported_auth(self) -> &'static [AuthMethod] {
        match self {
            Self::OauthGateway => &[AuthMethod::OauthCc],
            Self::SignGateway => &[],
            Self::Rest => HTTP_AUTH_REST,
            Self::Jsonrpc | Self::Graphql | Self::Job => HTTP_AUTH,
            Self::Grpc => &[AuthMethod::ApiKey],
            Self::Soap => &[AuthMethod::Basic, AuthMethod::ApiKey],
            Self::Ws => &[AuthMethod::OauthCc, AuthMethod::ApiKey],
            Self::EnvelopeTokenPlain
            | Self::EnvelopeTokenCrypt
            | Self::EnvelopeSignPlain
            | Self::EnvelopeSignCrypt
            | Self::EnvelopeCredPlain
            | Self::EnvelopeCredCrypt => &[],
        }
    }

    #[must_use]
    pub fn default_auth(self) -> Vec<AuthMethod> {
        match self {
            Self::OauthGateway => vec![AuthMethod::OauthCc],
            Self::SignGateway => vec![],
            Self::Rest => HTTP_AUTH_REST.to_vec(),
            Self::Jsonrpc | Self::Graphql | Self::Job => HTTP_AUTH.to_vec(),
            Self::Grpc => vec![AuthMethod::ApiKey],
            Self::Soap => vec![AuthMethod::Basic],
            Self::Ws => vec![AuthMethod::OauthCc, AuthMethod::ApiKey],
            Self::EnvelopeTokenPlain
            | Self::EnvelopeTokenCrypt
            | Self::EnvelopeSignPlain
            | Self::EnvelopeSignCrypt
            | Self::EnvelopeCredPlain
            | Self::EnvelopeCredCrypt => Vec::new(),
        }
    }
}

const HTTP_AUTH: &[AuthMethod] = &[
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    None,
    ApiKey,
    Bearer,
    Basic,
    OauthCc,
    Gettoken,
    Jwt,
    Digest,
    Hawk,
    AwsSigv4,
}

impl AuthMethod {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
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
            other => Err(format!("unknown auth value {other}")),
        }
    }

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
}

#[derive(Clone, Debug)]
pub struct LatencyConfig {
    pub min_ms: u64,
    pub max_ms: u64,
}

#[derive(Clone, Debug)]
pub struct TokenConfig {
    pub expire_minutes: u64,
    pub cleanup_interval_seconds: u64,
    pub max_per_client: u64,
}

#[derive(Clone, Debug)]
pub struct LogSettings {
    pub path: String,
    pub split_by_front: bool,
}

#[derive(Clone, Debug, Copy)]
pub struct MethodsConfig {
    pub posts_search: bool,
    pub photos_render: bool,
    pub imgdef_compare: bool,
}

impl MethodsConfig {
    #[must_use]
    pub fn is_enabled(self, method: &str) -> bool {
        match method {
            crate::types::gateway::METHOD_POSTS_SEARCH => self.posts_search,
            crate::types::gateway::METHOD_PHOTOS_RENDER => self.photos_render,
            crate::types::gateway::METHOD_IMGDEF_COMPARE => self.imgdef_compare,
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SignWindow {
    pub window_seconds: u64,
    pub nonce_ttl_seconds: u64,
}

#[derive(Clone, Debug)]
pub struct JobSettings {
    pub expire_seconds: u64,
}

#[derive(Clone, Debug)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert: PathBuf,
    pub key: PathBuf,
    pub client_ca: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct CommonFront {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub http2: bool,
    pub h2c: bool,
    pub expire_minutes: u64,
    pub auth: Vec<AuthMethod>,
    pub tls: TlsConfig,
}

#[derive(Clone, Debug)]
pub struct OauthFront {
    pub common: CommonFront,
}

#[derive(Clone, Debug)]
pub struct SignFront {
    pub common: CommonFront,
}

#[derive(Clone, Debug)]
pub struct RestFront {
    pub common: CommonFront,
}

#[derive(Clone, Debug)]
pub struct JsonrpcFront {
    pub common: CommonFront,
}

#[derive(Clone, Debug)]
pub struct GraphqlFront {
    pub common: CommonFront,
}

#[derive(Clone, Debug)]
pub struct GrpcFront {
    pub common: CommonFront,
}

#[derive(Clone, Debug)]
pub struct SoapFront {
    pub common: CommonFront,
}

#[derive(Clone, Debug)]
pub struct WsFront {
    pub common: CommonFront,
    pub max_conn: usize,
    pub ping_secs: u64,
}

#[derive(Clone, Debug)]
pub struct JobFront {
    pub common: CommonFront,
    pub workers: usize,
}

#[derive(Clone, Debug)]
pub struct EnvelopeFront {
    pub common: CommonFront,
    pub profiles: Vec<crate::envelope::profile::Profile>,
}

#[derive(Clone, Debug)]
pub struct Fronts {
    pub oauth_gateway: OauthFront,
    pub sign_gateway: SignFront,
    pub rest: RestFront,
    pub jsonrpc: JsonrpcFront,
    pub graphql: GraphqlFront,
    pub grpc: GrpcFront,
    pub soap: SoapFront,
    pub ws: WsFront,
    pub job: JobFront,
    pub envelope_token_plain: EnvelopeFront,
    pub envelope_token_crypt: EnvelopeFront,
    pub envelope_sign_plain: EnvelopeFront,
    pub envelope_sign_crypt: EnvelopeFront,
    pub envelope_cred_plain: EnvelopeFront,
    pub envelope_cred_crypt: EnvelopeFront,
}

impl Fronts {
    #[must_use]
    pub fn common(&self, name: FrontName) -> &CommonFront {
        match name {
            FrontName::OauthGateway => &self.oauth_gateway.common,
            FrontName::SignGateway => &self.sign_gateway.common,
            FrontName::Rest => &self.rest.common,
            FrontName::Jsonrpc => &self.jsonrpc.common,
            FrontName::Graphql => &self.graphql.common,
            FrontName::Grpc => &self.grpc.common,
            FrontName::Soap => &self.soap.common,
            FrontName::Ws => &self.ws.common,
            FrontName::Job => &self.job.common,
            FrontName::EnvelopeTokenPlain => &self.envelope_token_plain.common,
            FrontName::EnvelopeTokenCrypt => &self.envelope_token_crypt.common,
            FrontName::EnvelopeSignPlain => &self.envelope_sign_plain.common,
            FrontName::EnvelopeSignCrypt => &self.envelope_sign_crypt.common,
            FrontName::EnvelopeCredPlain => &self.envelope_cred_plain.common,
            FrontName::EnvelopeCredCrypt => &self.envelope_cred_crypt.common,
        }
    }

    #[must_use]
    pub fn envelope(&self, name: FrontName) -> &EnvelopeFront {
        match name {
            FrontName::EnvelopeTokenPlain => &self.envelope_token_plain,
            FrontName::EnvelopeTokenCrypt => &self.envelope_token_crypt,
            FrontName::EnvelopeSignPlain => &self.envelope_sign_plain,
            FrontName::EnvelopeSignCrypt => &self.envelope_sign_crypt,
            FrontName::EnvelopeCredPlain => &self.envelope_cred_plain,
            FrontName::EnvelopeCredCrypt => &self.envelope_cred_crypt,
            _ => &self.envelope_token_plain,
        }
    }

    #[must_use]
    pub fn enabled_names(&self) -> Vec<FrontName> {
        FrontName::all()
            .into_iter()
            .filter(|name| self.common(*name).enabled)
            .collect()
    }
}

#[derive(Clone)]
pub struct AppConfig {
    pub imgs_dir: String,
    pub latency: LatencyConfig,
    pub token: TokenConfig,
    pub log: LogSettings,
    pub methods: MethodsConfig,
    pub sign: SignWindow,
    pub job: JobSettings,
    pub fronts: Fronts,
    pub apps: Arc<AppRegistry>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LatencyToml {
    min_ms: Option<u64>,
    max_ms: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TokenToml {
    expire_minutes: Option<u64>,
    cleanup_interval_seconds: Option<u64>,
    max_per_client: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LogToml {
    dir: Option<String>,
    file: Option<String>,
    split_by_front: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MethodsToml {
    #[serde(rename = "upupt.posts.search")]
    posts_search: Option<bool>,
    #[serde(rename = "upupt.photos.render")]
    photos_render: Option<bool>,
    #[serde(rename = "upupt.imgdef.compare")]
    imgdef_compare: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SignToml {
    window_seconds: Option<u64>,
    nonce_ttl_seconds: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct JobToml {
    expire_seconds: Option<u64>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TlsToml {
    enabled: Option<bool>,
    cert: Option<String>,
    key: Option<String>,
    client_ca: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CommonToml {
    enabled: Option<bool>,
    host: Option<String>,
    port: Option<u16>,
    http2: Option<bool>,
    h2c: Option<bool>,
    expire_minutes: Option<u64>,
    auth: Option<Vec<String>>,
    tls: Option<TlsToml>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct OauthToml {
    #[serde(flatten)]
    common: CommonToml,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SignTomlFront {
    #[serde(flatten)]
    common: CommonToml,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RestToml {
    #[serde(flatten)]
    common: CommonToml,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct JsonrpcToml {
    #[serde(flatten)]
    common: CommonToml,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct GraphqlToml {
    #[serde(flatten)]
    common: CommonToml,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct GrpcToml {
    #[serde(flatten)]
    common: CommonToml,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SoapToml {
    #[serde(flatten)]
    common: CommonToml,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct WsToml {
    #[serde(flatten)]
    common: CommonToml,
    max_conn: Option<usize>,
    ping_secs: Option<u64>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct JobFrontToml {
    #[serde(flatten)]
    common: CommonToml,
    workers: Option<usize>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct EnvelopeFrontToml {
    #[serde(flatten)]
    common: CommonToml,
    #[serde(flatten)]
    profiles: crate::envelope::profile::EnvelopeToml,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FrontsToml {
    oauth_gateway: Option<OauthToml>,
    sign_gateway: Option<SignTomlFront>,
    rest: Option<RestToml>,
    jsonrpc: Option<JsonrpcToml>,
    graphql: Option<GraphqlToml>,
    grpc: Option<GrpcToml>,
    soap: Option<SoapToml>,
    ws: Option<WsToml>,
    job: Option<JobFrontToml>,
    envelope_token_plain: Option<EnvelopeFrontToml>,
    envelope_token_crypt: Option<EnvelopeFrontToml>,
    envelope_sign_plain: Option<EnvelopeFrontToml>,
    envelope_sign_crypt: Option<EnvelopeFrontToml>,
    envelope_cred_plain: Option<EnvelopeFrontToml>,
    envelope_cred_crypt: Option<EnvelopeFrontToml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlFile {
    #[serde(default)]
    latency: LatencyToml,
    #[serde(default)]
    token: TokenToml,
    #[serde(default)]
    log: LogToml,
    #[serde(default)]
    methods: MethodsToml,
    #[serde(default)]
    sign: SignToml,
    #[serde(default)]
    job: JobToml,
    #[serde(default)]
    fronts: FrontsToml,
    apps: Vec<AppToml>,
}

pub fn load() -> Result<AppConfig, String> {
    let bin_dir = binary_dir()?;
    let config_path = bin_dir.join("config").join("app.toml");
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config file {}: {e}", config_path.display()))?;
    parse_config(&content, &bin_dir)
}

fn parse_config(content: &str, bin_dir: &Path) -> Result<AppConfig, String> {
    let table: toml::Table = content
        .parse()
        .map_err(|e| format!("Failed to parse config TOML: {e}"))?;
    if table.contains_key("server") {
        return Err("[server] is removed; move host/port to [fronts.oauth_gateway]".to_string());
    }
    if table.contains_key("callback") {
        return Err("[callback] is removed".to_string());
    }
    let toml_file: TomlFile = table
        .try_into()
        .map_err(|e| format!("Failed to parse config TOML: {e}"))?;

    let token = TokenConfig {
        expire_minutes: toml_file.token.expire_minutes.unwrap_or(30),
        cleanup_interval_seconds: toml_file.token.cleanup_interval_seconds.unwrap_or(300),
        max_per_client: toml_file.token.max_per_client.unwrap_or(0),
    };

    let fronts = resolve_fronts(&toml_file.fronts, &token, bin_dir)?;
    validate_fronts(&fronts)?;

    let log_path = {
        let dir = toml_file.log.dir.as_deref().unwrap_or("logs");
        let file = toml_file.log.file.as_deref().unwrap_or("upupt.log");
        Path::new(dir).join(file).to_string_lossy().into_owned()
    };

    Ok(AppConfig {
        imgs_dir: resolve_imgs_dir(bin_dir),
        latency: LatencyConfig {
            min_ms: toml_file.latency.min_ms.unwrap_or(20),
            max_ms: toml_file.latency.max_ms.unwrap_or(100),
        },
        token,
        log: LogSettings {
            path: log_path,
            split_by_front: toml_file.log.split_by_front.unwrap_or(false),
        },
        methods: MethodsConfig {
            posts_search: toml_file.methods.posts_search.unwrap_or(true),
            photos_render: toml_file.methods.photos_render.unwrap_or(true),
            imgdef_compare: toml_file.methods.imgdef_compare.unwrap_or(true),
        },
        sign: SignWindow {
            window_seconds: toml_file.sign.window_seconds.unwrap_or(300),
            nonce_ttl_seconds: toml_file.sign.nonce_ttl_seconds.unwrap_or(600),
        },
        job: JobSettings {
            expire_seconds: toml_file.job.expire_seconds.unwrap_or(600),
        },
        fronts,
        apps: Arc::new(AppRegistry::from_toml(toml_file.apps, bin_dir)?),
    })
}

fn resolve_fronts(
    toml: &FrontsToml,
    token: &TokenConfig,
    bin_dir: &Path,
) -> Result<Fronts, String> {
    let oauth = toml.oauth_gateway.as_ref().map(|f| &f.common);
    let sign = toml.sign_gateway.as_ref().map(|f| &f.common);
    let rest = toml.rest.as_ref().map(|f| &f.common);
    let jsonrpc = toml.jsonrpc.as_ref().map(|f| &f.common);
    let graphql = toml.graphql.as_ref().map(|f| &f.common);
    let grpc = toml.grpc.as_ref().map(|f| &f.common);
    let soap = toml.soap.as_ref().map(|f| &f.common);
    let ws = toml.ws.as_ref().map(|f| &f.common);
    let job = toml.job.as_ref().map(|f| &f.common);

    let oauth_common = common_from(FrontName::OauthGateway, oauth, token, bin_dir)?;
    let sign_common = common_from(FrontName::SignGateway, sign, token, bin_dir)?;
    let rest_common = common_from(FrontName::Rest, rest, token, bin_dir)?;
    let jsonrpc_common = common_from(FrontName::Jsonrpc, jsonrpc, token, bin_dir)?;
    let graphql_common = common_from(FrontName::Graphql, graphql, token, bin_dir)?;
    let mut grpc_common = common_from(FrontName::Grpc, grpc, token, bin_dir)?;
    grpc_common.http2 = true;
    grpc_common.h2c = true;
    let soap_common = common_from(FrontName::Soap, soap, token, bin_dir)?;
    let ws_common = common_from(FrontName::Ws, ws, token, bin_dir)?;
    let job_common = common_from(FrontName::Job, job, token, bin_dir)?;
    let envelope_token_plain = envelope_from(
        FrontName::EnvelopeTokenPlain,
        toml.envelope_token_plain.as_ref(),
        token,
        bin_dir,
    )?;
    let envelope_token_crypt = envelope_from(
        FrontName::EnvelopeTokenCrypt,
        toml.envelope_token_crypt.as_ref(),
        token,
        bin_dir,
    )?;
    let envelope_sign_plain = envelope_from(
        FrontName::EnvelopeSignPlain,
        toml.envelope_sign_plain.as_ref(),
        token,
        bin_dir,
    )?;
    let envelope_sign_crypt = envelope_from(
        FrontName::EnvelopeSignCrypt,
        toml.envelope_sign_crypt.as_ref(),
        token,
        bin_dir,
    )?;
    let envelope_cred_plain = envelope_from(
        FrontName::EnvelopeCredPlain,
        toml.envelope_cred_plain.as_ref(),
        token,
        bin_dir,
    )?;
    let envelope_cred_crypt = envelope_from(
        FrontName::EnvelopeCredCrypt,
        toml.envelope_cred_crypt.as_ref(),
        token,
        bin_dir,
    )?;

    Ok(Fronts {
        oauth_gateway: OauthFront {
            common: oauth_common,
        },
        sign_gateway: SignFront {
            common: sign_common,
        },
        rest: RestFront {
            common: rest_common,
        },
        jsonrpc: JsonrpcFront {
            common: jsonrpc_common,
        },
        graphql: GraphqlFront {
            common: graphql_common,
        },
        grpc: GrpcFront {
            common: grpc_common,
        },
        soap: SoapFront {
            common: soap_common,
        },
        ws: {
            let w = toml.ws.clone().unwrap_or_default();
            WsFront {
                common: ws_common,
                max_conn: w.max_conn.unwrap_or(1024),
                ping_secs: w.ping_secs.unwrap_or(30),
            }
        },
        job: {
            let j = toml.job.clone().unwrap_or_default();
            JobFront {
                common: job_common,
                workers: j.workers.unwrap_or(4),
            }
        },
        envelope_token_plain,
        envelope_token_crypt,
        envelope_sign_plain,
        envelope_sign_crypt,
        envelope_cred_plain,
        envelope_cred_crypt,
    })
}

fn envelope_from(
    name: FrontName,
    toml: Option<&EnvelopeFrontToml>,
    token: &TokenConfig,
    bin_dir: &Path,
) -> Result<EnvelopeFront, String> {
    let Some(raw) = toml else {
        return Err(format!("front {} section missing", name.as_str()));
    };
    let common = common_from(name, Some(&raw.common), token, bin_dir)?;
    let profiles = crate::envelope::profile::compile_profiles(
        name,
        &raw.profiles.profiles,
        bin_dir,
        common.enabled,
    )?;
    Ok(EnvelopeFront { common, profiles })
}

fn common_from(
    name: FrontName,
    toml: Option<&CommonToml>,
    token: &TokenConfig,
    bin_dir: &Path,
) -> Result<CommonFront, String> {
    let empty = CommonToml::default();
    let t = toml.unwrap_or(&empty);
    let auth = match &t.auth {
        Some(list) => list
            .iter()
            .map(|s| {
                let method = AuthMethod::parse(s)?;
                if name == FrontName::SignGateway {
                    return Err(format!(
                        "front {} does not use HTTP auth {}",
                        name.as_str(),
                        method.as_str()
                    ));
                }
                if !name.supported_auth().contains(&method) {
                    return Err(format!(
                        "front {} does not support auth {}",
                        name.as_str(),
                        method.as_str()
                    ));
                }
                Ok(method)
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => name.default_auth(),
    };
    Ok(CommonFront {
        enabled: t.enabled.unwrap_or(true),
        host: t.host.clone().unwrap_or_else(|| "0.0.0.0".to_string()),
        port: t.port.unwrap_or_else(|| name.default_port()),
        http2: t.http2.unwrap_or(false),
        h2c: t.h2c.unwrap_or(false),
        expire_minutes: t.expire_minutes.unwrap_or(token.expire_minutes),
        auth,
        tls: tls_from(t.tls.as_ref(), bin_dir),
    })
}

fn tls_from(toml: Option<&TlsToml>, bin_dir: &Path) -> TlsConfig {
    let empty = TlsToml::default();
    let t = toml.unwrap_or(&empty);
    let client_ca = t.client_ca.as_deref().unwrap_or("").trim().to_string();
    TlsConfig {
        enabled: t.enabled.unwrap_or(false),
        cert: abs_path(bin_dir, t.cert.as_deref().unwrap_or("")),
        key: abs_path(bin_dir, t.key.as_deref().unwrap_or("")),
        client_ca: if client_ca.is_empty() {
            None
        } else {
            Some(abs_path(bin_dir, &client_ca))
        },
    }
}

fn validate_fronts(fronts: &Fronts) -> Result<(), String> {
    let enabled = fronts.enabled_names();
    if enabled.is_empty() {
        return Err("at least one front must be enabled".to_string());
    }

    let mut seen: HashMap<(String, u16), FrontName> = HashMap::new();
    for name in FrontName::all() {
        let common = fronts.common(name);
        if !common.enabled {
            continue;
        }
        let key = (common.host.clone(), common.port);
        if let Some(other) = seen.insert(key.clone(), name) {
            return Err(format!(
                "front {} and {} both bind {}:{}",
                other.as_str(),
                name.as_str(),
                key.0,
                key.1
            ));
        }
        if common.tls.enabled {
            if common.tls.cert.as_os_str().is_empty() || common.tls.key.as_os_str().is_empty() {
                return Err(format!(
                    "front {} tls.enabled requires cert and key",
                    name.as_str()
                ));
            }
            require_file(&common.tls.cert, name, "tls.cert")?;
            require_file(&common.tls.key, name, "tls.key")?;
            if let Some(ca) = &common.tls.client_ca {
                require_file(ca, name, "tls.client_ca")?;
            }
        }
    }
    Ok(())
}

fn require_file(path: &Path, name: FrontName, field: &str) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!(
            "front {} {field} file not found: {}",
            name.as_str(),
            path.display()
        ))
    }
}

fn abs_path(bin_dir: &Path, raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw.is_empty() {
        return PathBuf::new();
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        bin_dir.join(p)
    }
}

fn binary_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot get executable path: {e}"))?;
    exe.parent()
        .map(PathBuf::from)
        .ok_or_else(|| "Cannot determine executable directory".to_string())
}

fn resolve_imgs_dir(bin_dir: &Path) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        let cwd_imgs = cwd.join("imgs");
        if cwd_imgs.is_dir() {
            return cwd_imgs.to_string_lossy().into_owned();
        }
    }
    bin_dir.join("imgs").to_string_lossy().into_owned()
}

#[must_use]
pub fn needs_token_bucket(name: FrontName, common: &CommonFront) -> bool {
    matches!(
        name,
        FrontName::EnvelopeTokenPlain | FrontName::EnvelopeTokenCrypt
    ) || common
        .auth
        .iter()
        .any(|a| matches!(*a, AuthMethod::OauthCc | AuthMethod::Gettoken))
}

pub async fn apply_latency(cfg: &LatencyConfig) -> u64 {
    if cfg.min_ms == 0 && cfg.max_ms == 0 {
        return 0;
    }
    let lo = cfg.min_ms.min(cfg.max_ms);
    let hi = cfg.min_ms.max(cfg.max_ms);
    let delay = if lo == hi {
        lo
    } else {
        rand::random_range(lo..=hi)
    };
    tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
    delay
}

#[must_use]
pub fn granted_methods(app_methods: &HashSet<String>, global: MethodsConfig) -> HashSet<String> {
    app_methods
        .iter()
        .filter(|m| global.is_enabled(m))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_config;
    use std::io::Write;

    fn apps(bin: &std::path::Path) -> String {
        let pem = bin.join("pub.pem");
        if !pem.exists() {
            std::fs::write(&pem, "not-a-key").unwrap();
        }
        format!(
            "\n[[apps]]\nclient_id = \"jwczjzl\"\nclient_secret = \"K9mQ2vXp#8Lr4TzYw7nR\"\nname = \"jwczjzl\"\nrsa_public_pem = \"{}\"\nmethods = [\"upupt.posts.search\"]\n",
            pem.display()
        )
    }

    fn fronts(skip: &str, extra: &str) -> String {
        let mut out = String::new();
        for (i, name) in [
            "envelope_token_plain",
            "envelope_token_crypt",
            "envelope_sign_plain",
            "envelope_sign_crypt",
            "envelope_cred_plain",
            "envelope_cred_crypt",
        ]
        .into_iter()
        .enumerate()
        {
            if name == skip {
                continue;
            }
            let port = 3010 + i;
            out.push_str(&format!(
                "[fronts.{name}]\nenabled = false\nauth = []\nport = {port}\n"
            ));
        }
        out.push_str(extra);
        out
    }

    #[test]
    fn missing_envelope_section_fails() {
        let dir = std::env::temp_dir().join("upupt-env-missing");
        std::fs::create_dir_all(&dir).unwrap();
        let err = match parse_config(&apps(&dir), &dir) {
            Err(err) => err,
            Ok(_) => panic!("missing section should fail"),
        };
        assert!(err.contains("section missing"), "{err}");
    }

    #[test]
    fn bad_key_unknown_alg_and_duplicate_path_fail() {
        let dir = std::env::temp_dir().join("upupt-env-bad");
        std::fs::create_dir_all(&dir).unwrap();
        let mut key = std::fs::File::create(dir.join("short.key")).unwrap();
        key.write_all(&[1u8; 15]).unwrap();
        let bad_len = format!(
            "{}\n[fronts.envelope_token_crypt]\nenabled = true\nauth = []\nport = 3011\n[[fronts.envelope_token_crypt.profiles]]\nname = \"call\"\nkind = \"call\"\nmethods = [\"POST\"]\npath = \"/call\"\nbody_kind = \"json\"\nbody_encoding = \"identity\"\nfail_in = \"http\"\ncrypt_alg = \"sm4_ecb_pkcs7\"\ncipher_scope = \"fields\"\ncipher_fields = [\"data\"]\nkey = \"short.key\"\n[[fronts.envelope_token_crypt.profiles.fields]]\nname = \"data\"\nslot = \"json\"\nrole = \"cipher\"\n",
            fronts("envelope_token_crypt", "")
        );
        let err = match parse_config(&format!("{}{bad_len}", apps(&dir)), &dir) {
            Err(err) => err,
            Ok(_) => panic!("short key should fail"),
        };
        assert!(err.contains("key length"), "{err}");

        let unknown = fronts(
            "envelope_sign_plain",
            "[fronts.envelope_sign_plain]\nenabled = true\nauth = []\nport = 3012\n[[fronts.envelope_sign_plain.profiles]]\nname = \"call\"\nkind = \"call\"\nmethods = [\"POST\"]\npath = \"/call\"\nbody_kind = \"json\"\nbody_encoding = \"identity\"\nfail_in = \"http\"\nsign_alg = \"sha1\"\n",
        );
        let err = match parse_config(&format!("{}{unknown}", apps(&dir)), &dir) {
            Err(err) => err,
            Ok(_) => panic!("unknown algorithm should fail"),
        };
        assert!(err.contains("unknown sign_alg"), "{err}");

        let dup = fronts(
            "envelope_cred_plain",
            "[fronts.envelope_cred_plain]\nenabled = true\nauth = []\nport = 3014\n[[fronts.envelope_cred_plain.profiles]]\nname = \"a\"\nkind = \"call\"\nmethods = [\"GET\"]\npath = \"/call\"\nbody_kind = \"empty\"\nbody_encoding = \"identity\"\nfail_in = \"http\"\n[[fronts.envelope_cred_plain.profiles]]\nname = \"b\"\nkind = \"call\"\nmethods = [\"GET\"]\npath = \"/call\"\nbody_kind = \"empty\"\nbody_encoding = \"identity\"\nfail_in = \"http\"\n",
        );
        let err = match parse_config(&format!("{}{dup}", apps(&dir)), &dir) {
            Err(err) => err,
            Ok(_) => panic!("duplicate path should fail"),
        };
        assert!(err.contains("repeats method and path"), "{err}");
    }
}
