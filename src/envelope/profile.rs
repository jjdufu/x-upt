use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rsa::{PrivateKey as RsaPrivateKey, PublicKey as RsaPublicKey};
use serde::Deserialize;

use crate::config::FrontName;
use crate::envelope::crypt::{load_rsa_private, load_rsa_public};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Issue,
    Call,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Json,
    Form,
    Raw,
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyEncoding {
    Identity,
    Base64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Header,
    Query,
    Form,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Auth,
    Cipher,
    Biz,
    Sign,
    Token,
    WrapKey,
    Timestamp,
    Nonce,
}

impl From<Role> for crate::envelope::table::Role {
    fn from(role: Role) -> Self {
        match role {
            Role::Auth => Self::Auth,
            Role::Cipher => Self::Cipher,
            Role::Biz => Self::Biz,
            Role::Sign => Self::Sign,
            Role::Token => Self::Token,
            Role::WrapKey => Self::WrapKey,
            Role::Timestamp => Self::Timestamp,
            Role::Nonce => Self::Nonce,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonKind {
    SortedKv,
    Concat,
    Raw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignAlg {
    Md5,
    HmacSha256,
    Sm3,
    HmacSm3,
    RsaSha256,
    Sm2Sign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptAlg {
    AesEcbPkcs5,
    AesCfb,
    AesGcm,
    DesCbcPkcs5,
    Sm4EcbPkcs7,
    Sm4CbcPkcs7,
    Sm4EcbNoPadding,
    RsaPkcs1Public,
    RsaPkcs1Private,
    Sm2Encrypt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapAlg {
    RsaPkcs1Public,
    Sm2Encrypt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Hex,
    Base64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HexCase {
    Upper,
    Lower,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IvFrom {
    Key,
    RandomPrefix,
    Field,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sm2Mode {
    C1C3C2,
    C1C2C3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailIn {
    Http,
    Envelope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    SignThenEncrypt,
    EncryptThenSign,
}

#[derive(Clone, Debug)]
pub struct FieldSpec {
    pub name: String,
    pub slot: Slot,
    pub role: Role,
}

impl FieldSpec {
    #[cfg(test)]
    #[must_use]
    pub fn new(name: &str, slot: Slot, role: Role) -> Self {
        Self {
            name: name.to_string(),
            slot,
            role,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub kind: Kind,
    pub methods: Vec<Method>,
    pub path: String,
    pub body_kind: BodyKind,
    pub body_encoding: BodyEncoding,
    pub slot_priority: [Slot; 4],
    pub fields: Vec<FieldSpec>,
    pub biz_rest: bool,
    pub token_prefix: Option<String>,
    pub token_response_field: Option<String>,
    pub secrets: HashMap<String, String>,
    pub canon: Option<CanonKind>,
    pub canon_skip: Vec<String>,
    pub canon_sep: String,
    pub canon_prefix_key: bool,
    pub canon_suffix_key: bool,
    pub canon_fields: Vec<String>,
    pub sign_alg: Option<SignAlg>,
    pub hex_case: HexCase,
    pub sign_encoding: Encoding,
    pub crypt_alg: Option<CryptAlg>,
    pub cipher_scope_body: bool,
    pub cipher_fields: Vec<String>,
    pub crypt_encoding: Encoding,
    pub padding_pkcs5: bool,
    pub segment_bits: Option<usize>,
    pub iv_len: Option<usize>,
    pub iv_from: Option<IvFrom>,
    pub tag_bits: Option<usize>,
    pub user_id: Option<String>,
    pub sm2_mode: Sm2Mode,
    pub key: Option<Vec<u8>>,
    pub sign_key: Option<Vec<u8>>,
    pub rsa_private: Option<RsaPrivateKey>,
    pub rsa_public: Option<RsaPublicKey>,
    pub sm2_private: Option<Vec<u8>>,
    pub sm2_public: Option<Vec<u8>>,
    pub wrap: Option<WrapAlg>,
    pub order: Option<Order>,
    pub fail_in: FailIn,
    pub response_body_kind: BodyKind,
    pub response_encoding: BodyEncoding,
    pub response_crypt: Option<CryptAlg>,
    pub response_plain: bool,
    pub fixed_iv: Option<Vec<u8>>,
}

impl Profile {
    #[must_use]
    pub fn cipher_scope_body(&self) -> bool {
        self.cipher_scope_body
    }

    #[must_use]
    pub fn names_field(&self, name: &str, slot: Slot) -> bool {
        self.fields.iter().any(|field| {
            field.slot == slot
                && if slot == Slot::Header {
                    field.name.eq_ignore_ascii_case(name)
                } else {
                    field.name == name
                }
        })
    }

    #[must_use]
    pub fn named_anywhere(&self, name: &str) -> bool {
        self.fields.iter().any(|field| {
            if field.slot == Slot::Header {
                field.name.eq_ignore_ascii_case(name)
            } else {
                field.name == name
            }
        })
    }

    #[must_use]
    pub fn canon_key(&self) -> Vec<u8> {
        self.sign_key
            .clone()
            .or_else(|| self.key.clone())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn allows(&self, method: &str) -> bool {
        self.methods.iter().any(|item| item.as_str() == method)
    }

    #[must_use]
    pub fn auth_label(front: FrontName) -> &'static str {
        match front {
            FrontName::EnvelopeTokenPlain | FrontName::EnvelopeTokenCrypt => "envelope_token",
            FrontName::EnvelopeSignPlain | FrontName::EnvelopeSignCrypt => "envelope_sign",
            FrontName::EnvelopeCredPlain | FrontName::EnvelopeCredCrypt => "envelope_cred",
            _ => "envelope",
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn test_call_empty() -> Self {
        Self::test_call(BodyKind::Json, BodyEncoding::Identity, Vec::new(), true)
    }

    #[cfg(test)]
    #[must_use]
    pub fn test_call(
        body_kind: BodyKind,
        body_encoding: BodyEncoding,
        fields: Vec<FieldSpec>,
        biz_rest: bool,
    ) -> Self {
        Self {
            name: "test".to_string(),
            kind: Kind::Call,
            methods: vec![Method::Post],
            path: "/call".to_string(),
            body_kind,
            body_encoding,
            slot_priority: [Slot::Json, Slot::Form, Slot::Query, Slot::Header],
            fields,
            biz_rest,
            token_prefix: None,
            token_response_field: None,
            secrets: HashMap::new(),
            canon: None,
            canon_skip: Vec::new(),
            canon_sep: String::new(),
            canon_prefix_key: false,
            canon_suffix_key: false,
            canon_fields: Vec::new(),
            sign_alg: None,
            hex_case: HexCase::Lower,
            sign_encoding: Encoding::Hex,
            crypt_alg: None,
            cipher_scope_body: false,
            cipher_fields: Vec::new(),
            crypt_encoding: Encoding::Base64,
            padding_pkcs5: false,
            segment_bits: None,
            iv_len: None,
            iv_from: None,
            tag_bits: None,
            user_id: None,
            sm2_mode: Sm2Mode::C1C3C2,
            key: None,
            sign_key: None,
            rsa_private: None,
            rsa_public: None,
            sm2_private: None,
            sm2_public: None,
            wrap: None,
            order: None,
            fail_in: FailIn::Http,
            response_body_kind: body_kind,
            response_encoding: body_encoding,
            response_crypt: None,
            response_plain: false,
            fixed_iv: None,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn test_sign(canon: CanonKind) -> Self {
        let mut profile = Self::test_call_empty();
        profile.canon = Some(canon);
        profile
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldToml {
    name: String,
    slot: String,
    role: String,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum SecretsToml {
    Path(String),
    Map(HashMap<String, String>),
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileToml {
    name: String,
    kind: String,
    methods: Vec<String>,
    path: String,
    body_kind: String,
    body_encoding: String,
    slot_priority: Option<Vec<String>>,
    #[serde(default)]
    fields: Vec<FieldToml>,
    biz_rest: Option<bool>,
    token_prefix: Option<String>,
    token_response_field: Option<String>,
    secrets: Option<SecretsToml>,
    canon: Option<String>,
    canon_skip: Option<Vec<String>>,
    canon_sep: Option<String>,
    canon_prefix_key: Option<bool>,
    canon_suffix_key: Option<bool>,
    canon_fields: Option<Vec<String>>,
    sign_alg: Option<String>,
    hex_case: Option<String>,
    sign_encoding: Option<String>,
    crypt_alg: Option<String>,
    cipher_scope: Option<String>,
    cipher_fields: Option<Vec<String>>,
    crypt_encoding: Option<String>,
    padding: Option<String>,
    segment_bits: Option<usize>,
    iv_len: Option<usize>,
    iv_from: Option<String>,
    tag_bits: Option<usize>,
    user_id: Option<String>,
    sm2_mode: Option<String>,
    key: Option<String>,
    sign_key: Option<String>,
    private_key: Option<String>,
    public_key: Option<String>,
    wrap: Option<String>,
    order: Option<String>,
    fail_in: String,
    response_body_kind: Option<String>,
    response_encoding: Option<String>,
    response_slots: Option<Vec<FieldToml>>,
    response_crypt_alg: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeToml {
    #[serde(default)]
    pub profiles: Vec<ProfileToml>,
}

type AsymKeys = (
    Option<RsaPrivateKey>,
    Option<RsaPublicKey>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
);

pub fn compile_profiles(
    front: FrontName,
    raw: &[ProfileToml],
    bin_dir: &Path,
    enabled: bool,
) -> Result<Vec<Profile>, String> {
    if enabled && raw.is_empty() {
        return Err(format!(
            "front {} enabled requires at least one profile",
            front.as_str()
        ));
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let profile = compile_one(front, item, bin_dir)?;
        let key = (
            profile
                .methods
                .iter()
                .map(|method| method.as_str())
                .collect::<Vec<_>>()
                .join(","),
            profile.path.clone(),
        );
        if !seen.insert(key) {
            return Err(format!(
                "front {} profile {} repeats method and path {}",
                front.as_str(),
                profile.name,
                profile.path
            ));
        }
        out.push(profile);
    }
    Ok(out)
}

fn compile_one(front: FrontName, raw: &ProfileToml, bin_dir: &Path) -> Result<Profile, String> {
    let where_ = format!("front {} profile {}", front.as_str(), raw.name);
    let kind = parse_kind(&raw.kind).map_err(|e| format!("{where_} {e}"))?;
    if kind == Kind::Issue
        && !matches!(
            front,
            FrontName::EnvelopeTokenPlain | FrontName::EnvelopeTokenCrypt
        )
    {
        return Err(format!("{where_} kind issue is only valid on token fronts"));
    }
    if raw.methods.is_empty() {
        return Err(format!("{where_} methods is empty"));
    }
    let methods = raw
        .methods
        .iter()
        .map(|item| parse_method(item).map_err(|e| format!("{where_} {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    if !raw.path.starts_with('/') {
        return Err(format!("{where_} path must start with /"));
    }
    let body_kind = parse_body_kind(&raw.body_kind).map_err(|e| format!("{where_} {e}"))?;
    let body_encoding =
        parse_encoding_body(&raw.body_encoding).map_err(|e| format!("{where_} {e}"))?;
    let slot_priority = match &raw.slot_priority {
        Some(list) => parse_priority(list).map_err(|e| format!("{where_} {e}"))?,
        None => [Slot::Json, Slot::Form, Slot::Query, Slot::Header],
    };
    let mut fields = raw
        .fields
        .iter()
        .map(|field| parse_field(field).map_err(|e| format!("{where_} {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(response_slots) = &raw.response_slots {
        let _ = response_slots;
    }
    let sign_alg =
        opt_parse(raw.sign_alg.as_deref(), parse_sign).map_err(|e| format!("{where_} {e}"))?;
    let crypt_alg =
        opt_parse(raw.crypt_alg.as_deref(), parse_crypt).map_err(|e| format!("{where_} {e}"))?;
    let wrap = opt_parse(raw.wrap.as_deref(), parse_wrap).map_err(|e| format!("{where_} {e}"))?;
    let order =
        opt_parse(raw.order.as_deref(), parse_order).map_err(|e| format!("{where_} {e}"))?;
    let fail_in = parse_fail(&raw.fail_in).map_err(|e| format!("{where_} {e}"))?;
    let hex_case = match raw.hex_case.as_deref() {
        Some("upper") => HexCase::Upper,
        Some("lower") | None => HexCase::Lower,
        Some(other) => return Err(format!("{where_} unknown hex_case {other}")),
    };
    if raw.hex_case.is_none()
        && (raw.sign_encoding.as_deref() == Some("hex")
            || raw.crypt_encoding.as_deref() == Some("hex"))
    {
        return Err(format!("{where_} hex_case is required"));
    }
    let sign_encoding = match raw.sign_encoding.as_deref() {
        Some("base64") => Encoding::Base64,
        Some("hex") | None => Encoding::Hex,
        Some(other) => return Err(format!("{where_} unknown sign_encoding {other}")),
    };
    let crypt_encoding = match raw.crypt_encoding.as_deref() {
        Some("hex") => Encoding::Hex,
        Some("base64") | None => Encoding::Base64,
        Some(other) => return Err(format!("{where_} unknown crypt_encoding {other}")),
    };
    let iv_from =
        opt_parse(raw.iv_from.as_deref(), parse_iv).map_err(|e| format!("{where_} {e}"))?;
    let sm2_mode = match raw.sm2_mode.as_deref() {
        Some("c1c2c3") => Sm2Mode::C1C2C3,
        Some("c1c3c2") | None => Sm2Mode::C1C3C2,
        Some(other) => return Err(format!("{where_} unknown sm2_mode {other}")),
    };
    let cipher_scope_body = match raw.cipher_scope.as_deref() {
        Some("body") => true,
        Some("fields") | None => false,
        Some(other) => return Err(format!("{where_} unknown cipher_scope {other}")),
    };
    if crypt_alg.is_some() && sign_alg.is_some() && order.is_none() {
        return Err(format!("{where_} order is required"));
    }
    if crypt_alg.is_none() && order.is_some() {
        return Err(format!("{where_} plaintext profile must not set order"));
    }
    validate_alg_fields(&where_, crypt_alg, raw, iv_from)?;
    if fields
        .iter()
        .any(|field| field.role == Role::Token && field.slot == Slot::Header)
        && raw.token_prefix.is_none()
    {
        return Err(format!("{where_} token_prefix is required"));
    }
    if kind == Kind::Issue && raw.token_response_field.is_none() {
        return Err(format!("{where_} token_response_field is required"));
    }
    if matches!(sign_alg, Some(SignAlg::Sm2Sign)) && raw.user_id.is_none() {
        return Err(format!("{where_} user_id is required"));
    }
    let key = read_key(bin_dir, raw.key.as_deref(), &where_, "key")?;
    let sign_key = read_key(bin_dir, raw.sign_key.as_deref(), &where_, "sign_key")?;
    check_key_len(&where_, crypt_alg, key.as_deref())?;
    let (rsa_private, rsa_public, sm2_private, sm2_public) =
        load_asym(bin_dir, raw, &where_, sign_alg, crypt_alg, wrap)?;
    let secrets = load_secrets(bin_dir, raw.secrets.as_ref(), &fields, &where_)?;
    let response_body_kind = match raw.response_body_kind.as_deref() {
        Some(text) => parse_body_kind(text).map_err(|e| format!("{where_} {e}"))?,
        None => body_kind,
    };
    let response_encoding = match raw.response_encoding.as_deref() {
        Some(text) => parse_encoding_body(text).map_err(|e| format!("{where_} {e}"))?,
        None => body_encoding,
    };
    let (response_crypt, response_plain) = match raw.response_crypt_alg.as_deref() {
        Some("none") => (None, true),
        Some(text) => (
            Some(parse_crypt(text).map_err(|e| format!("{where_} {e}"))?),
            false,
        ),
        None => (crypt_alg, false),
    };
    if let Some(extra) = &raw.response_slots {
        for field in extra {
            fields.push(parse_field(field).map_err(|e| format!("{where_} {e}"))?);
        }
    }
    Ok(Profile {
        name: raw.name.clone(),
        kind,
        methods,
        path: raw.path.clone(),
        body_kind,
        body_encoding,
        slot_priority,
        fields,
        biz_rest: raw.biz_rest.unwrap_or(false),
        token_prefix: raw.token_prefix.clone(),
        token_response_field: raw.token_response_field.clone(),
        secrets,
        canon: opt_parse(raw.canon.as_deref(), parse_canon).map_err(|e| format!("{where_} {e}"))?,
        canon_skip: raw.canon_skip.clone().unwrap_or_default(),
        canon_sep: raw.canon_sep.clone().unwrap_or_default(),
        canon_prefix_key: raw.canon_prefix_key.unwrap_or(false),
        canon_suffix_key: raw.canon_suffix_key.unwrap_or(false),
        canon_fields: raw.canon_fields.clone().unwrap_or_default(),
        sign_alg,
        hex_case,
        sign_encoding,
        crypt_alg,
        cipher_scope_body,
        cipher_fields: raw.cipher_fields.clone().unwrap_or_default(),
        crypt_encoding,
        padding_pkcs5: raw.padding.as_deref() == Some("pkcs5"),
        segment_bits: raw.segment_bits,
        iv_len: raw.iv_len,
        iv_from,
        tag_bits: raw.tag_bits,
        user_id: raw.user_id.clone(),
        sm2_mode,
        key,
        sign_key,
        rsa_private,
        rsa_public,
        sm2_private,
        sm2_public,
        wrap,
        order,
        fail_in,
        response_body_kind,
        response_encoding,
        response_crypt,
        response_plain,
        fixed_iv: None,
    })
}

fn validate_alg_fields(
    where_: &str,
    crypt_alg: Option<CryptAlg>,
    raw: &ProfileToml,
    iv_from: Option<IvFrom>,
) -> Result<(), String> {
    let Some(alg) = crypt_alg else {
        return Ok(());
    };
    match alg {
        CryptAlg::AesCfb => {
            if raw.padding.is_none() || raw.segment_bits.is_none() || iv_from.is_none() {
                return Err(format!(
                    "{where_} aes_cfb requires padding, segment_bits and iv_from"
                ));
            }
            if !matches!(raw.padding.as_deref(), Some("none" | "pkcs5")) {
                return Err(format!("{where_} unknown padding"));
            }
        }
        CryptAlg::AesGcm => {
            if raw.iv_len.is_none() || raw.tag_bits.is_none() {
                return Err(format!("{where_} aes_gcm requires iv_len and tag_bits"));
            }
        }
        CryptAlg::DesCbcPkcs5 | CryptAlg::Sm4CbcPkcs7 if iv_from.is_none() => {
            return Err(format!("{where_} iv_from is required"));
        }
        CryptAlg::DesCbcPkcs5 | CryptAlg::Sm4CbcPkcs7 => {}
        _ => {}
    }
    if raw.cipher_scope.as_deref() == Some("fields")
        && raw.cipher_fields.as_ref().is_none_or(Vec::is_empty)
    {
        let named = raw.fields.iter().any(|field| field.role == "cipher");
        if !named {
            return Err(format!("{where_} cipher fields are required"));
        }
    }
    Ok(())
}

fn check_key_len(where_: &str, alg: Option<CryptAlg>, key: Option<&[u8]>) -> Result<(), String> {
    let Some(alg) = alg else {
        return Ok(());
    };
    let Some(key) = key else {
        if matches!(
            alg,
            CryptAlg::RsaPkcs1Public | CryptAlg::RsaPkcs1Private | CryptAlg::Sm2Encrypt
        ) {
            return Ok(());
        }
        return Err(format!("{where_} key is required"));
    };
    let ok = match alg {
        CryptAlg::AesEcbPkcs5 | CryptAlg::AesCfb | CryptAlg::AesGcm => {
            matches!(key.len(), 16 | 24 | 32)
        }
        CryptAlg::DesCbcPkcs5 => key.len() == 8,
        CryptAlg::Sm4EcbPkcs7 | CryptAlg::Sm4CbcPkcs7 | CryptAlg::Sm4EcbNoPadding => {
            key.len() == 16
        }
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{where_} key length {} is invalid for algorithm",
            key.len()
        ))
    }
}

fn load_asym(
    bin_dir: &Path,
    raw: &ProfileToml,
    where_: &str,
    sign_alg: Option<SignAlg>,
    crypt_alg: Option<CryptAlg>,
    wrap: Option<WrapAlg>,
) -> Result<AsymKeys, String> {
    let need_rsa = matches!(sign_alg, Some(SignAlg::RsaSha256))
        || matches!(
            crypt_alg,
            Some(CryptAlg::RsaPkcs1Public | CryptAlg::RsaPkcs1Private)
        )
        || matches!(wrap, Some(WrapAlg::RsaPkcs1Public));
    let need_sm2 = matches!(sign_alg, Some(SignAlg::Sm2Sign))
        || matches!(crypt_alg, Some(CryptAlg::Sm2Encrypt))
        || matches!(wrap, Some(WrapAlg::Sm2Encrypt));
    let private_text = read_text(bin_dir, raw.private_key.as_deref(), where_, "private_key")?;
    let public_text = read_text(bin_dir, raw.public_key.as_deref(), where_, "public_key")?;
    if need_rsa {
        let private = load_rsa_private(
            private_text
                .as_deref()
                .ok_or_else(|| format!("{where_} private_key is required"))?,
        )
        .map_err(|e| format!("{where_} {e}"))?;
        let public = load_rsa_public(
            public_text
                .as_deref()
                .ok_or_else(|| format!("{where_} public_key is required"))?,
        )
        .map_err(|e| format!("{where_} {e}"))?;
        return Ok((Some(private), Some(public), None, None));
    }
    if need_sm2 {
        let private = read_key(bin_dir, raw.private_key.as_deref(), where_, "private_key")?
            .ok_or_else(|| format!("{where_} private_key is required"))?;
        let public = read_key(bin_dir, raw.public_key.as_deref(), where_, "public_key")?
            .ok_or_else(|| format!("{where_} public_key is required"))?;
        if private.len() != 32 || public.len() != 65 {
            return Err(format!("{where_} sm2 key length is invalid"));
        }
        let ctx = libsm::sm2::signature::SigCtx::new();
        ctx.load_seckey(&private)
            .map_err(|_| format!("{where_} sm2 private key cannot be parsed"))?;
        ctx.load_pubkey(&public)
            .map_err(|_| format!("{where_} sm2 public key cannot be parsed"))?;
        return Ok((None, None, Some(private), Some(public)));
    }
    Ok((None, None, None, None))
}

fn load_secrets(
    bin_dir: &Path,
    raw: Option<&SecretsToml>,
    fields: &[FieldSpec],
    where_: &str,
) -> Result<HashMap<String, String>, String> {
    let Some(raw) = raw else {
        return Ok(HashMap::new());
    };
    let mut out = HashMap::new();
    match raw {
        SecretsToml::Path(path) => {
            let auth: Vec<_> = fields
                .iter()
                .filter(|field| field.role == Role::Auth)
                .collect();
            if auth.len() != 1 {
                return Err(format!(
                    "{where_} secrets path needs exactly one auth field"
                ));
            }
            out.insert(auth[0].name.clone(), read_secret(bin_dir, path, where_)?);
        }
        SecretsToml::Map(map) => {
            for (name, path) in map {
                out.insert(name.clone(), read_secret(bin_dir, path, where_)?);
            }
        }
    }
    Ok(out)
}

fn read_secret(bin_dir: &Path, path: &str, where_: &str) -> Result<String, String> {
    let text = read_text(bin_dir, Some(path), where_, "secrets")?
        .ok_or_else(|| missing(where_, "secrets", path))?;
    Ok(text.trim_end_matches(['\r', '\n']).to_string())
}

fn read_key(
    bin_dir: &Path,
    path: Option<&str>,
    where_: &str,
    field: &str,
) -> Result<Option<Vec<u8>>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    let full = abs_path(bin_dir, path);
    let bytes =
        std::fs::read(&full).map_err(|_| missing(where_, field, &full.display().to_string()))?;
    Ok(Some(bytes))
}

fn read_text(
    bin_dir: &Path,
    path: Option<&str>,
    where_: &str,
    field: &str,
) -> Result<Option<String>, String> {
    let Some(bytes) = read_key(bin_dir, path, where_, field)? else {
        return Ok(None);
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{where_} {field} is not utf-8"))
}

fn missing(where_: &str, field: &str, path: &str) -> String {
    format!("{where_} {field} file not found: {path}. Run scripts/gen_envelope_keys.sh")
}

fn abs_path(bin_dir: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw.trim());
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        bin_dir.join(path)
    }
}

fn opt_parse<T>(
    raw: Option<&str>,
    parse: fn(&str) -> Result<T, String>,
) -> Result<Option<T>, String> {
    raw.map(parse).transpose()
}

fn parse_kind(raw: &str) -> Result<Kind, String> {
    match raw {
        "issue" => Ok(Kind::Issue),
        "call" => Ok(Kind::Call),
        other => Err(format!("unknown kind {other}")),
    }
}

fn parse_method(raw: &str) -> Result<Method, String> {
    match raw {
        "GET" => Ok(Method::Get),
        "POST" => Ok(Method::Post),
        other => Err(format!("unknown method {other}")),
    }
}

fn parse_body_kind(raw: &str) -> Result<BodyKind, String> {
    match raw {
        "json" => Ok(BodyKind::Json),
        "form" => Ok(BodyKind::Form),
        "raw" => Ok(BodyKind::Raw),
        "empty" => Ok(BodyKind::Empty),
        other => Err(format!("unknown body_kind {other}")),
    }
}

fn parse_encoding_body(raw: &str) -> Result<BodyEncoding, String> {
    match raw {
        "identity" => Ok(BodyEncoding::Identity),
        "base64" => Ok(BodyEncoding::Base64),
        other => Err(format!("unknown body_encoding {other}")),
    }
}

fn parse_priority(list: &[String]) -> Result<[Slot; 4], String> {
    if list.len() != 4 {
        return Err("slot_priority must list four slots".to_string());
    }
    let mut out = [Slot::Json; 4];
    for (i, item) in list.iter().enumerate() {
        out[i] = parse_slot(item)?;
    }
    Ok(out)
}

fn parse_slot(raw: &str) -> Result<Slot, String> {
    match raw {
        "header" => Ok(Slot::Header),
        "query" => Ok(Slot::Query),
        "form" => Ok(Slot::Form),
        "json" => Ok(Slot::Json),
        other => Err(format!("unknown slot {other}")),
    }
}

fn parse_role(raw: &str) -> Result<Role, String> {
    match raw {
        "auth" => Ok(Role::Auth),
        "cipher" => Ok(Role::Cipher),
        "biz" => Ok(Role::Biz),
        "sign" => Ok(Role::Sign),
        "token" => Ok(Role::Token),
        "wrap_key" => Ok(Role::WrapKey),
        "timestamp" => Ok(Role::Timestamp),
        "nonce" => Ok(Role::Nonce),
        other => Err(format!("unknown role {other}")),
    }
}

fn parse_field(raw: &FieldToml) -> Result<FieldSpec, String> {
    Ok(FieldSpec {
        name: raw.name.clone(),
        slot: parse_slot(&raw.slot)?,
        role: parse_role(&raw.role)?,
    })
}

fn parse_sign(raw: &str) -> Result<SignAlg, String> {
    match raw {
        "md5" => Ok(SignAlg::Md5),
        "hmac_sha256" => Ok(SignAlg::HmacSha256),
        "sm3" => Ok(SignAlg::Sm3),
        "hmac_sm3" => Ok(SignAlg::HmacSm3),
        "rsa_sha256" => Ok(SignAlg::RsaSha256),
        "sm2_sign" => Ok(SignAlg::Sm2Sign),
        other => Err(format!("unknown sign_alg {other}")),
    }
}

fn parse_crypt(raw: &str) -> Result<CryptAlg, String> {
    match raw {
        "aes_ecb_pkcs5" => Ok(CryptAlg::AesEcbPkcs5),
        "aes_cfb" => Ok(CryptAlg::AesCfb),
        "aes_gcm" => Ok(CryptAlg::AesGcm),
        "des_cbc_pkcs5" => Ok(CryptAlg::DesCbcPkcs5),
        "sm4_ecb_pkcs7" => Ok(CryptAlg::Sm4EcbPkcs7),
        "sm4_cbc_pkcs7" => Ok(CryptAlg::Sm4CbcPkcs7),
        "sm4_ecb_nopadding" => Ok(CryptAlg::Sm4EcbNoPadding),
        "rsa_pkcs1_public" => Ok(CryptAlg::RsaPkcs1Public),
        "rsa_pkcs1_private" => Ok(CryptAlg::RsaPkcs1Private),
        "sm2_encrypt" => Ok(CryptAlg::Sm2Encrypt),
        other => Err(format!("unknown crypt_alg {other}")),
    }
}

fn parse_wrap(raw: &str) -> Result<WrapAlg, String> {
    match raw {
        "rsa_pkcs1_public" => Ok(WrapAlg::RsaPkcs1Public),
        "sm2_encrypt" => Ok(WrapAlg::Sm2Encrypt),
        other => Err(format!("unknown wrap {other}")),
    }
}

fn parse_order(raw: &str) -> Result<Order, String> {
    match raw {
        "sign_then_encrypt" => Ok(Order::SignThenEncrypt),
        "encrypt_then_sign" => Ok(Order::EncryptThenSign),
        other => Err(format!("unknown order {other}")),
    }
}

fn parse_fail(raw: &str) -> Result<FailIn, String> {
    match raw {
        "http" => Ok(FailIn::Http),
        "envelope" => Ok(FailIn::Envelope),
        other => Err(format!("unknown fail_in {other}")),
    }
}

fn parse_iv(raw: &str) -> Result<IvFrom, String> {
    match raw {
        "key" => Ok(IvFrom::Key),
        "random_prefix" => Ok(IvFrom::RandomPrefix),
        "field" => Ok(IvFrom::Field),
        other => Err(format!("unknown iv_from {other}")),
    }
}

fn parse_canon(raw: &str) -> Result<CanonKind, String> {
    match raw {
        "sorted_kv" => Ok(CanonKind::SortedKv),
        "concat" => Ok(CanonKind::Concat),
        "raw" => Ok(CanonKind::Raw),
        other => Err(format!("unknown canon {other}")),
    }
}
