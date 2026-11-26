//! Envelope profile compilation and secret material validation against `SQLite` records.

use std::collections::{HashMap, HashSet};

use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{PrivateKey as RsaPrivateKey, PublicKey as RsaPublicKey};
use x_upt_core::domain::{EnvelopeFieldDto, EnvelopeProfileSpec, EnvelopeSecretRecord, FrontName};

use crate::table::Role;

/// Envelope profile kind (`issue` or `call`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Issues a front-scoped token.
    Issue,
    /// Executes an authenticated/signed/encrypted business mock call.
    Call,
}

/// Allowed HTTP method on an envelope profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// HTTP `GET`.
    Get,
    /// HTTP `POST`.
    Post,
}

impl Method {
    /// Canonical uppercase verb.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// Request/response body structure mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// `application/json`.
    Json,
    /// `application/x-www-form-urlencoded`.
    Form,
    /// Raw whole-body bytes.
    Raw,
    /// Empty body (`0` bytes).
    Empty,
}

/// Outer body byte encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyEncoding {
    /// Raw UTF-8 / binary bytes.
    Identity,
    /// Base64-encoded body.
    Base64,
}

/// Parameter extraction slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// HTTP header.
    Header,
    /// URL query string.
    Query,
    /// Form body.
    Form,
    /// JSON object body.
    Json,
}

/// Signing string canonicalization mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonKind {
    /// UTF-8 byte-sorted `k=v` pairs joined by separator.
    SortedKv,
    /// Explicit field list values joined by separator.
    Concat,
    /// Single raw field value or decrypted plaintext.
    Raw,
}

/// Closed-set envelope signing / digest algorithms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignAlg {
    /// MD5 hex digest.
    Md5,
    /// HMAC-SHA256.
    HmacSha256,
    /// SM3 hex digest.
    Sm3,
    /// HMAC-SM3.
    HmacSm3,
    /// PKCS#1 v1.5 SHA256WithRSA.
    RsaSha256,
    /// SM2 signature with user ID.
    Sm2Sign,
}

/// Closed-set envelope encryption algorithms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptAlg {
    /// AES-128/192/256 ECB PKCS#7.
    AesEcbPkcs5,
    /// AES-128/192/256 CFB.
    AesCfb,
    /// AES-128/192/256 GCM.
    AesGcm,
    /// DES-CBC PKCS#5 (8-byte key & IV).
    DesCbcPkcs5,
    /// SM4-ECB PKCS#7 (16-byte key).
    Sm4EcbPkcs7,
    /// SM4-CBC PKCS#7 (16-byte key & IV).
    Sm4CbcPkcs7,
    /// SM4-ECB NoPadding.
    Sm4EcbNoPadding,
    /// RSA PKCS#1 v1.5 public-key encryption.
    RsaPkcs1Public,
    /// RSA PKCS#1 type-1 private-key encryption.
    RsaPkcs1Private,
    /// SM2 public-key encryption.
    Sm2Encrypt,
}

/// Asymmetric key wrapping mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapAlg {
    /// RSA PKCS#1 v1.5 public key wrap.
    RsaPkcs1Public,
    /// SM2 public key wrap.
    Sm2Encrypt,
}

/// Text encoding of signatures and ciphertexts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Hexadecimal string.
    Hex,
    /// Standard Base64 string.
    Base64,
}

/// Hexadecimal letter case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HexCase {
    /// Uppercase `0-9A-F`.
    Upper,
    /// Lowercase `0-9a-f`.
    Lower,
}

/// Initialization vector source for block/AEAD modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IvFrom {
    /// Prefix bytes of the symmetric key.
    Key,
    /// Random IV prepended to ciphertext.
    RandomPrefix,
    /// Dedicated `iv` auth field.
    Field,
}

/// SM2 ciphertext component order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sm2Mode {
    /// `C1 || C3 || C2`.
    C1C3C2,
    /// `C1 || C2 || C3`.
    C1C2C3,
}

/// Failure response placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailIn {
    /// Plain HTTP `400` / `401` status.
    Http,
    /// HTTP `200` sealed inside response envelope when key material is available.
    Envelope,
}

/// Relative order of signing and encryption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Request verifies signature before decrypting; response signs ciphertext.
    SignThenEncrypt,
    /// Request decrypts before verifying signature; response signs plaintext.
    EncryptThenSign,
}

/// Validated parameter slot mapping.
#[derive(Clone, Debug)]
pub struct FieldSpec {
    /// Field or header name.
    pub name: String,
    /// Extraction slot.
    pub slot: Slot,
    /// Field role.
    pub role: Role,
}

/// Cipher and padding flags grouped to satisfy struct boolean limits.
#[derive(Clone, Copy, Debug, Default)]
pub struct CipherFlags {
    /// `true` when `cipher_scope == "body"`.
    pub scope_body: bool,
    /// `true` when `aes_cfb` uses PKCS#5/7 padding.
    pub padding_pkcs5: bool,
    /// `true` when response omits encryption (`response_crypt_alg == "none"`).
    pub response_plain: bool,
}

/// Canonicalization affix flags.
#[derive(Clone, Copy, Debug, Default)]
pub struct CanonAffix {
    /// Prepend key to canonical material.
    pub prefix_key: bool,
    /// Append key to canonical material.
    pub suffix_key: bool,
}

/// Validated, key-resolved runtime envelope profile.
#[derive(Clone, Debug)]
pub struct Profile {
    /// Profile identifier.
    pub name: String,
    /// Issue or call.
    pub kind: Kind,
    /// Allowed HTTP methods.
    pub methods: Vec<Method>,
    /// Request path.
    pub path: String,
    /// Inbound body kind.
    pub body_kind: BodyKind,
    /// Inbound outer encoding.
    pub body_encoding: BodyEncoding,
    /// 4-slot priority array.
    pub slot_priority: [Slot; 4],
    /// Explicit field mappings.
    pub fields: Vec<FieldSpec>,
    /// Whether unnamed non-header fields enter `biz`.
    pub biz_rest: bool,
    /// Header token prefix.
    pub token_prefix: Option<String>,
    /// Token response field name.
    pub token_response_field: Option<String>,
    /// Resolved static credentials (`field_name -> expected_secret_text`).
    pub secrets: HashMap<String, String>,
    /// Canonicalization rule.
    pub canon: Option<CanonKind>,
    /// Skipped canonicalization field names.
    pub canon_skip: Vec<String>,
    /// Canonicalization delimiter.
    pub canon_sep: String,
    /// Key prefix/suffix flags.
    pub canon_affix: CanonAffix,
    /// Explicit canonicalization field order.
    pub canon_fields: Vec<String>,
    /// Signing algorithm.
    pub sign_alg: Option<SignAlg>,
    /// Hex case.
    pub hex_case: HexCase,
    /// Signature encoding.
    pub sign_encoding: Encoding,
    /// Request encryption algorithm.
    pub crypt_alg: Option<CryptAlg>,
    /// Cipher scope, CFB padding, and plaintext-response flags.
    pub cipher_flags: CipherFlags,
    /// Encrypted field names.
    pub cipher_fields: Vec<String>,
    /// Ciphertext encoding.
    pub crypt_encoding: Encoding,
    /// CFB segment bits.
    pub segment_bits: Option<usize>,
    /// GCM IV length.
    pub iv_len: Option<usize>,
    /// IV source.
    pub iv_from: Option<IvFrom>,
    /// GCM tag bits.
    pub tag_bits: Option<usize>,
    /// SM2 user ID.
    pub user_id: Option<String>,
    /// SM2 layout mode.
    pub sm2_mode: Sm2Mode,
    /// Symmetric key bytes.
    pub key: Option<Vec<u8>>,
    /// Signing key bytes.
    pub sign_key: Option<Vec<u8>>,
    /// Parsed RSA private key.
    pub rsa_private: Option<RsaPrivateKey>,
    /// Parsed RSA public key.
    pub rsa_public: Option<RsaPublicKey>,
    /// Raw 32-byte SM2 secret key.
    pub sm2_private: Option<Vec<u8>>,
    /// Raw 65-byte uncompressed SM2 public key.
    pub sm2_public: Option<Vec<u8>>,
    /// Key wrapping mode.
    pub wrap: Option<WrapAlg>,
    /// Sign/encrypt order.
    pub order: Option<Order>,
    /// Failure delivery mode.
    pub fail_in: FailIn,
    /// Response body kind.
    pub response_body_kind: BodyKind,
    /// Response body encoding.
    pub response_encoding: BodyEncoding,
    /// Optional response encryption override.
    pub response_crypt: Option<CryptAlg>,
    /// Working IV populated per request/response.
    pub fixed_iv: Option<Vec<u8>>,
}

impl Profile {
    /// Checks if `name` is explicitly mapped in `slot`.
    #[must_use]
    pub fn names_field(&self, name: &str, slot: Slot) -> bool {
        self.fields.iter().any(|f| {
            f.slot == slot
                && if slot == Slot::Header {
                    f.name.eq_ignore_ascii_case(name)
                } else {
                    f.name == name
                }
        })
    }

    /// Checks if `name` is mapped in any slot.
    #[must_use]
    pub fn named_anywhere(&self, name: &str) -> bool {
        self.fields.iter().any(|f| {
            if f.slot == Slot::Header {
                f.name.eq_ignore_ascii_case(name)
            } else {
                f.name == name
            }
        })
    }

    /// Returns signing/cipher key bytes used for canonical string affixing.
    #[must_use]
    pub fn canon_key(&self) -> Vec<u8> {
        self.sign_key
            .clone()
            .or_else(|| self.key.clone())
            .unwrap_or_default()
    }

    /// Checks whether HTTP `method` is permitted on this profile.
    #[must_use]
    pub fn allows(&self, method: &str) -> bool {
        self.methods.iter().any(|m| m.as_str() == method)
    }

    /// Access-log `auth` label for an envelope front.
    #[must_use]
    pub fn auth_label(front: FrontName) -> &'static str {
        match front {
            FrontName::EnvelopeTokenPlain | FrontName::EnvelopeTokenCrypt => "envelope_token",
            FrontName::EnvelopeSignPlain | FrontName::EnvelopeSignCrypt => "envelope_sign",
            FrontName::EnvelopeCredPlain | FrontName::EnvelopeCredCrypt => "envelope_cred",
            _ => "envelope",
        }
    }
}

/// Validates profile syntax alone (allowing saving profiles before uploading secrets).
///
/// # Errors
/// Returns an error string if structural fields, enums, or duplicate routes are invalid.
pub fn validate_profile_specs(
    front: FrontName,
    specs: &[EnvelopeProfileSpec],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for item in specs {
        let base = parse_base(front, item)?;
        for m in &base.methods {
            if !seen.insert((m.as_str().to_string(), base.path.clone())) {
                return Err(format!(
                    "画像 {} 重复定义了方法与路径 {} {}",
                    base.name,
                    m.as_str(),
                    base.path
                ));
            }
        }
        let _ = parse_crypto_meta(&base.ctx, item)?;
    }
    Ok(())
}

/// Compiles runtime [`Profile`]s with full cryptographic key/secret resolution against `store`.
///
/// # Errors
/// Returns an error string when any profile syntax or referenced secret/key is missing or invalid.
pub fn compile_profiles(
    front: FrontName,
    specs: &[EnvelopeProfileSpec],
    store: &HashMap<String, EnvelopeSecretRecord>,
) -> Result<Vec<Profile>, String> {
    validate_profile_specs(front, specs)?;
    specs
        .iter()
        .map(|item| compile_one(front, item, store))
        .collect()
}

struct ParsedBase {
    ctx: String,
    name: String,
    kind: Kind,
    methods: Vec<Method>,
    path: String,
    body_kind: BodyKind,
    body_encoding: BodyEncoding,
    slot_priority: [Slot; 4],
    fields: Vec<FieldSpec>,
}

fn parse_base(front: FrontName, raw: &EnvelopeProfileSpec) -> Result<ParsedBase, String> {
    let ctx = format!("{} 画像 {}", front.as_str(), raw.name);
    let kind = match raw.kind.as_str() {
        "issue" => Kind::Issue,
        "call" => Kind::Call,
        other => return Err(format!("{ctx}: 未知 kind {other}")),
    };
    if kind == Kind::Issue && !front.is_token_envelope() {
        return Err(format!("{ctx}: 仅换票信封面可使用 issue 画像"));
    }
    if raw.name.trim().is_empty() || raw.methods.is_empty() || !raw.path.starts_with('/') {
        return Err(format!("{ctx}: name/methods 不能为空且 path 须以 / 开头"));
    }
    let methods = raw
        .methods
        .iter()
        .map(|m| parse_method(m).map_err(|e| format!("{ctx}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let body_kind = parse_body_kind(&raw.body_kind).map_err(|e| format!("{ctx}: {e}"))?;
    let body_encoding = parse_body_enc(&raw.body_encoding).map_err(|e| format!("{ctx}: {e}"))?;
    let slot_priority = match &raw.slot_priority {
        Some(list) => parse_priority(list).map_err(|e| format!("{ctx}: {e}"))?,
        None => [Slot::Json, Slot::Form, Slot::Query, Slot::Header],
    };
    let fields = raw
        .fields
        .iter()
        .map(|f| parse_field(f).map_err(|e| format!("{ctx}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    if fields
        .iter()
        .any(|f| f.role == Role::Token && f.slot == Slot::Header)
        && raw.token_prefix.is_none()
    {
        return Err(format!("{ctx}: 头部 Token 必须指定 token_prefix"));
    }
    if kind == Kind::Issue && raw.token_response_field.is_none() {
        return Err(format!("{ctx}: issue 画像必须指定 token_response_field"));
    }
    Ok(ParsedBase {
        ctx,
        name: raw.name.clone(),
        kind,
        methods,
        path: raw.path.clone(),
        body_kind,
        body_encoding,
        slot_priority,
        fields,
    })
}

struct CryptoMeta {
    canon: Option<CanonKind>,
    sign_alg: Option<SignAlg>,
    hex_case: HexCase,
    sign_encoding: Encoding,
    crypt_alg: Option<CryptAlg>,
    cipher_flags: CipherFlags,
    crypt_encoding: Encoding,
    iv_from: Option<IvFrom>,
    sm2_mode: Sm2Mode,
    wrap: Option<WrapAlg>,
    order: Option<Order>,
    fail_in: FailIn,
    response_crypt: Option<CryptAlg>,
}

fn parse_crypto_meta(ctx: &str, raw: &EnvelopeProfileSpec) -> Result<CryptoMeta, String> {
    let sign_alg = opt_parse(raw.sign_alg.as_deref(), parse_sign, ctx)?;
    let crypt_alg = opt_parse(raw.crypt_alg.as_deref(), parse_crypt, ctx)?;
    let wrap = opt_parse(raw.wrap.as_deref(), parse_wrap, ctx)?;
    let order = opt_parse(raw.order.as_deref(), parse_order, ctx)?;
    let canon = opt_parse(raw.canon.as_deref(), parse_canon, ctx)?;
    let iv_from = opt_parse(raw.iv_from.as_deref(), parse_iv, ctx)?;
    let fail_in = match raw.fail_in.as_str() {
        "http" => FailIn::Http,
        "envelope" => FailIn::Envelope,
        other => return Err(format!("{ctx}: 未知 fail_in {other}")),
    };
    let hex_case = match raw.hex_case.as_deref() {
        Some("upper") => HexCase::Upper,
        Some("lower") | None => HexCase::Lower,
        Some(other) => return Err(format!("{ctx}: 未知 hex_case {other}")),
    };
    let sign_encoding = parse_enc(raw.sign_encoding.as_deref(), Encoding::Hex, ctx)?;
    let crypt_encoding = parse_enc(raw.crypt_encoding.as_deref(), Encoding::Base64, ctx)?;
    let sm2_mode = match raw.sm2_mode.as_deref() {
        Some("c1c2c3") => Sm2Mode::C1C2C3,
        Some("c1c3c2") | None => Sm2Mode::C1C3C2,
        Some(other) => return Err(format!("{ctx}: 未知 sm2_mode {other}")),
    };
    let scope_body = match raw.cipher_scope.as_deref() {
        Some("body") => true,
        Some("fields") | None => false,
        Some(other) => return Err(format!("{ctx}: 未知 cipher_scope {other}")),
    };
    if crypt_alg.is_some() && sign_alg.is_some() && order.is_none() {
        return Err(format!("{ctx}: 同时启用签名与加密时必须指定 order"));
    }
    if crypt_alg.is_none() && order.is_some() {
        return Err(format!("{ctx}: 明文画像不可设置 order"));
    }
    if matches!(sign_alg, Some(SignAlg::Sm2Sign)) && raw.user_id.is_none() {
        return Err(format!("{ctx}: sm2_sign 必须指定 user_id"));
    }
    validate_alg_params(ctx, crypt_alg, raw, iv_from)?;
    let (response_crypt, response_plain) = match raw.response_crypt_alg.as_deref() {
        Some("none") => (None, true),
        Some(text) => (
            Some(parse_crypt(text).map_err(|e| format!("{ctx}: {e}"))?),
            false,
        ),
        None => (crypt_alg, false),
    };
    Ok(CryptoMeta {
        canon,
        sign_alg,
        hex_case,
        sign_encoding,
        crypt_alg,
        cipher_flags: CipherFlags {
            scope_body,
            padding_pkcs5: raw.padding.as_deref() == Some("pkcs5"),
            response_plain,
        },
        crypt_encoding,
        iv_from,
        sm2_mode,
        wrap,
        order,
        fail_in,
        response_crypt,
    })
}

fn compile_one(
    front: FrontName,
    raw: &EnvelopeProfileSpec,
    store: &HashMap<String, EnvelopeSecretRecord>,
) -> Result<Profile, String> {
    let base = parse_base(front, raw)?;
    let meta = parse_crypto_meta(&base.ctx, raw)?;
    let key = resolve_bytes(store, raw.key.as_deref(), &base.ctx, "key")?;
    let sign_key = resolve_bytes(store, raw.sign_key.as_deref(), &base.ctx, "sign_key")?;
    check_key_len(&base.ctx, meta.crypt_alg, key.as_deref())?;
    if matches!(meta.sign_alg, Some(SignAlg::HmacSha256 | SignAlg::HmacSm3)) && sign_key.is_none() {
        return Err(format!("{}: HMAC 算法需要配置 sign_key", base.ctx));
    }
    let asym = resolve_asym(store, raw, &base.ctx, &meta)?;
    let secrets = resolve_secrets(store, &raw.secrets, &base.ctx)?;
    let response_body_kind = match raw.response_body_kind.as_deref() {
        Some(t) => parse_body_kind(t).map_err(|e| format!("{}: {e}", base.ctx))?,
        None => base.body_kind,
    };
    let response_encoding = match raw.response_encoding.as_deref() {
        Some(t) => parse_body_enc(t).map_err(|e| format!("{}: {e}", base.ctx))?,
        None => base.body_encoding,
    };
    Ok(Profile {
        name: base.name,
        kind: base.kind,
        methods: base.methods,
        path: base.path,
        body_kind: base.body_kind,
        body_encoding: base.body_encoding,
        slot_priority: base.slot_priority,
        fields: base.fields,
        biz_rest: raw.biz_rest.unwrap_or(false),
        token_prefix: raw.token_prefix.clone(),
        token_response_field: raw.token_response_field.clone(),
        secrets,
        canon: meta.canon,
        canon_skip: raw.canon_skip.clone().unwrap_or_default(),
        canon_sep: raw.canon_sep.clone().unwrap_or_default(),
        canon_affix: CanonAffix {
            prefix_key: raw.canon_prefix_key.unwrap_or(false),
            suffix_key: raw.canon_suffix_key.unwrap_or(false),
        },
        canon_fields: raw.canon_fields.clone().unwrap_or_default(),
        sign_alg: meta.sign_alg,
        hex_case: meta.hex_case,
        sign_encoding: meta.sign_encoding,
        crypt_alg: meta.crypt_alg,
        cipher_flags: meta.cipher_flags,
        cipher_fields: raw.cipher_fields.clone().unwrap_or_default(),
        crypt_encoding: meta.crypt_encoding,
        segment_bits: raw.segment_bits,
        iv_len: raw.iv_len,
        iv_from: meta.iv_from,
        tag_bits: raw.tag_bits,
        user_id: raw.user_id.clone(),
        sm2_mode: meta.sm2_mode,
        key,
        sign_key,
        rsa_private: asym.rsa_private,
        rsa_public: asym.rsa_public,
        sm2_private: asym.sm2_private,
        sm2_public: asym.sm2_public,
        wrap: meta.wrap,
        order: meta.order,
        fail_in: meta.fail_in,
        response_body_kind,
        response_encoding,
        response_crypt: meta.response_crypt,
        fixed_iv: None,
    })
}

struct AsymResolved {
    rsa_private: Option<RsaPrivateKey>,
    rsa_public: Option<RsaPublicKey>,
    sm2_private: Option<Vec<u8>>,
    sm2_public: Option<Vec<u8>>,
}

fn resolve_asym(
    store: &HashMap<String, EnvelopeSecretRecord>,
    raw: &EnvelopeProfileSpec,
    ctx: &str,
    meta: &CryptoMeta,
) -> Result<AsymResolved, String> {
    let need_rsa = matches!(meta.sign_alg, Some(SignAlg::RsaSha256))
        || matches!(
            meta.crypt_alg,
            Some(CryptAlg::RsaPkcs1Public | CryptAlg::RsaPkcs1Private)
        )
        || matches!(meta.wrap, Some(WrapAlg::RsaPkcs1Public));
    let need_sm2 = matches!(meta.sign_alg, Some(SignAlg::Sm2Sign))
        || matches!(meta.crypt_alg, Some(CryptAlg::Sm2Encrypt))
        || matches!(meta.wrap, Some(WrapAlg::Sm2Encrypt));

    if need_rsa {
        let priv_bytes = resolve_bytes(store, raw.private_key.as_deref(), ctx, "private_key")?
            .ok_or_else(|| format!("{ctx}: RSA 需要配置 private_key"))?;
        let pub_bytes = resolve_bytes(store, raw.public_key.as_deref(), ctx, "public_key")?
            .ok_or_else(|| format!("{ctx}: RSA 需要配置 public_key"))?;
        let priv_str =
            std::str::from_utf8(&priv_bytes).map_err(|_| format!("{ctx}: RSA 私钥非 UTF-8 PEM"))?;
        let pub_str =
            std::str::from_utf8(&pub_bytes).map_err(|_| format!("{ctx}: RSA 公钥非 UTF-8 PEM"))?;
        let rsa_private = RsaPrivateKey::from_pkcs8_pem(priv_str)
            .map_err(|e| format!("{ctx}: RSA 私钥无效: {e}"))?;
        let rsa_public = RsaPublicKey::from_public_key_pem(pub_str)
            .map_err(|e| format!("{ctx}: RSA 公钥无效: {e}"))?;
        return Ok(AsymResolved {
            rsa_private: Some(rsa_private),
            rsa_public: Some(rsa_public),
            sm2_private: None,
            sm2_public: None,
        });
    }
    if need_sm2 {
        let sk = resolve_bytes(store, raw.private_key.as_deref(), ctx, "private_key")?
            .ok_or_else(|| format!("{ctx}: SM2 需要配置 private_key"))?;
        let pk = resolve_bytes(store, raw.public_key.as_deref(), ctx, "public_key")?
            .ok_or_else(|| format!("{ctx}: SM2 需要配置 public_key"))?;
        if sk.len() != 32 || pk.len() != 65 {
            return Err(format!("{ctx}: SM2 私钥须 32 字节且公钥须 65 字节"));
        }
        let sig_ctx = libsm::sm2::signature::SigCtx::new();
        sig_ctx
            .load_seckey(&sk)
            .map_err(|_| format!("{ctx}: SM2 私钥解析失败"))?;
        sig_ctx
            .load_pubkey(&pk)
            .map_err(|_| format!("{ctx}: SM2 公钥解析失败"))?;
        return Ok(AsymResolved {
            rsa_private: None,
            rsa_public: None,
            sm2_private: Some(sk),
            sm2_public: Some(pk),
        });
    }
    Ok(AsymResolved {
        rsa_private: None,
        rsa_public: None,
        sm2_private: None,
        sm2_public: None,
    })
}

fn resolve_bytes(
    store: &HashMap<String, EnvelopeSecretRecord>,
    secret_name: Option<&str>,
    ctx: &str,
    field: &str,
) -> Result<Option<Vec<u8>>, String> {
    let Some(name) = secret_name.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let rec = store
        .get(name)
        .ok_or_else(|| format!("{ctx}: 缺少密钥 {field} -> {name}"))?;
    Ok(Some(rec.raw_bytes.clone()))
}

fn resolve_secrets(
    store: &HashMap<String, EnvelopeSecretRecord>,
    map: &HashMap<String, String>,
    ctx: &str,
) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for (field_name, secret_name) in map {
        let rec = store
            .get(secret_name)
            .ok_or_else(|| format!("{ctx}: 缺少凭据密钥 {field_name} -> {secret_name}"))?;
        let text = std::str::from_utf8(&rec.raw_bytes)
            .map_err(|_| format!("{ctx}: 凭据 {secret_name} 必须是 UTF-8 文本"))?
            .trim_end_matches(['\r', '\n'])
            .to_string();
        out.insert(field_name.clone(), text);
    }
    Ok(out)
}

fn validate_alg_params(
    ctx: &str,
    crypt_alg: Option<CryptAlg>,
    raw: &EnvelopeProfileSpec,
    iv_from: Option<IvFrom>,
) -> Result<(), String> {
    if let Some(alg) = crypt_alg {
        match alg {
            CryptAlg::AesCfb => {
                if raw.padding.is_none() || raw.segment_bits.is_none() || iv_from.is_none() {
                    return Err(format!(
                        "{ctx}: aes_cfb 需要 padding、segment_bits 与 iv_from"
                    ));
                }
                if !matches!(raw.padding.as_deref(), Some("none" | "pkcs5")) {
                    return Err(format!("{ctx}: 未知 padding"));
                }
            }
            CryptAlg::AesGcm => {
                if raw.iv_len.is_none() || raw.tag_bits.is_none() {
                    return Err(format!("{ctx}: aes_gcm 需要 iv_len 与 tag_bits"));
                }
            }
            CryptAlg::DesCbcPkcs5 | CryptAlg::Sm4CbcPkcs7 if iv_from.is_none() => {
                return Err(format!("{ctx}: CBC 模式需要配置 iv_from"));
            }
            _ => {}
        }
        if raw.cipher_scope.as_deref() == Some("fields")
            && raw.cipher_fields.as_ref().is_none_or(Vec::is_empty)
            && !raw.fields.iter().any(|f| f.role == "cipher")
        {
            return Err(format!("{ctx}: fields 加密范围必须列出密文字段"));
        }
    }
    Ok(())
}

fn check_key_len(ctx: &str, alg: Option<CryptAlg>, key: Option<&[u8]>) -> Result<(), String> {
    let Some(alg) = alg else { return Ok(()) };
    if matches!(
        alg,
        CryptAlg::RsaPkcs1Public | CryptAlg::RsaPkcs1Private | CryptAlg::Sm2Encrypt
    ) {
        return Ok(());
    }
    let Some(key) = key else {
        return Err(format!("{ctx}: 对称加密算法必须配置 key"));
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
            "{ctx}: 密钥长度 {} 字节不符合当前算法要求",
            key.len()
        ))
    }
}

fn opt_parse<T>(
    raw: Option<&str>,
    f: fn(&str) -> Result<T, String>,
    ctx: &str,
) -> Result<Option<T>, String> {
    raw.filter(|s| !s.is_empty())
        .map(|s| f(s).map_err(|e| format!("{ctx}: {e}")))
        .transpose()
}

fn parse_enc(raw: Option<&str>, default: Encoding, ctx: &str) -> Result<Encoding, String> {
    match raw {
        Some("hex") => Ok(Encoding::Hex),
        Some("base64") => Ok(Encoding::Base64),
        None => Ok(default),
        Some(other) => Err(format!("{ctx}: 未知编码 {other}")),
    }
}

fn parse_method(raw: &str) -> Result<Method, String> {
    match raw {
        "GET" => Ok(Method::Get),
        "POST" => Ok(Method::Post),
        other => Err(format!("未知 method {other}")),
    }
}

fn parse_body_kind(raw: &str) -> Result<BodyKind, String> {
    match raw {
        "json" => Ok(BodyKind::Json),
        "form" => Ok(BodyKind::Form),
        "raw" => Ok(BodyKind::Raw),
        "empty" => Ok(BodyKind::Empty),
        other => Err(format!("未知 body_kind {other}")),
    }
}

fn parse_body_enc(raw: &str) -> Result<BodyEncoding, String> {
    match raw {
        "identity" => Ok(BodyEncoding::Identity),
        "base64" => Ok(BodyEncoding::Base64),
        other => Err(format!("未知 body_encoding {other}")),
    }
}

fn parse_slot(raw: &str) -> Result<Slot, String> {
    match raw {
        "header" => Ok(Slot::Header),
        "query" => Ok(Slot::Query),
        "form" => Ok(Slot::Form),
        "json" => Ok(Slot::Json),
        other => Err(format!("未知 slot {other}")),
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
        other => Err(format!("未知 role {other}")),
    }
}

fn parse_field(raw: &EnvelopeFieldDto) -> Result<FieldSpec, String> {
    Ok(FieldSpec {
        name: raw.name.clone(),
        slot: parse_slot(&raw.slot)?,
        role: parse_role(&raw.role)?,
    })
}

fn parse_priority(list: &[String]) -> Result<[Slot; 4], String> {
    if list.len() != 4 {
        return Err("slot_priority 必须包含 4 个槽位".to_string());
    }
    Ok([
        parse_slot(list.first().map_or("", String::as_str))?,
        parse_slot(list.get(1).map_or("", String::as_str))?,
        parse_slot(list.get(2).map_or("", String::as_str))?,
        parse_slot(list.get(3).map_or("", String::as_str))?,
    ])
}

fn parse_sign(raw: &str) -> Result<SignAlg, String> {
    match raw {
        "md5" => Ok(SignAlg::Md5),
        "hmac_sha256" => Ok(SignAlg::HmacSha256),
        "sm3" => Ok(SignAlg::Sm3),
        "hmac_sm3" => Ok(SignAlg::HmacSm3),
        "rsa_sha256" => Ok(SignAlg::RsaSha256),
        "sm2_sign" => Ok(SignAlg::Sm2Sign),
        other => Err(format!("未知 sign_alg {other}")),
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
        other => Err(format!("未知 crypt_alg {other}")),
    }
}

fn parse_wrap(raw: &str) -> Result<WrapAlg, String> {
    match raw {
        "rsa_pkcs1_public" => Ok(WrapAlg::RsaPkcs1Public),
        "sm2_encrypt" => Ok(WrapAlg::Sm2Encrypt),
        other => Err(format!("未知 wrap {other}")),
    }
}

fn parse_order(raw: &str) -> Result<Order, String> {
    match raw {
        "sign_then_encrypt" => Ok(Order::SignThenEncrypt),
        "encrypt_then_sign" => Ok(Order::EncryptThenSign),
        other => Err(format!("未知 order {other}")),
    }
}

fn parse_iv(raw: &str) -> Result<IvFrom, String> {
    match raw {
        "key" => Ok(IvFrom::Key),
        "random_prefix" => Ok(IvFrom::RandomPrefix),
        "field" => Ok(IvFrom::Field),
        other => Err(format!("未知 iv_from {other}")),
    }
}

fn parse_canon(raw: &str) -> Result<CanonKind, String> {
    match raw {
        "sorted_kv" => Ok(CanonKind::SortedKv),
        "concat" => Ok(CanonKind::Concat),
        "raw" => Ok(CanonKind::Raw),
        other => Err(format!("未知 canon {other}")),
    }
}
