//! Envelope request pipeline, token store, and `{ok, reason, hit, fields, digest}` mock response.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use x_upt_core::domain::{FrontName, secrets_equal_bytes};

use crate::canon;
use crate::crypt::{self, CryptError};
use crate::crypt_asym::{unwrap_key, wrap_key};
use crate::nonce::NonceStore;
use crate::profile::{CryptAlg, FailIn, IvFrom, Kind, Order, Profile};
use crate::slots::{self, Incoming, SlotFail};
use crate::table::{FieldTable, Role, SlotValue, biz_object, parse_json};

/// Deterministic mock response body returned by the 6 envelope fronts.
#[derive(Clone, Debug, PartialEq)]
pub struct MockBody {
    /// Whether authentication/decryption/shape succeeded.
    pub ok: bool,
    /// Empty string on success, or one of `bad_token` / `bad_sign` / `bad_credential` / `bad_decrypt` / `bad_shape`.
    pub reason: String,
    /// `true` when `fields` has at least 1 business entry.
    pub hit: bool,
    /// Extracted business fields.
    pub fields: Vec<(String, SlotValue)>,
    /// Lowercase SHA-256 hex digest of canonical `fields` JSON.
    pub digest: String,
}

impl MockBody {
    /// Builds a successful [`MockBody`] from extracted `biz` pairs.
    #[must_use]
    pub fn success(biz: &[(String, SlotValue)]) -> Self {
        let hit = !biz.is_empty();
        let fields = if hit { biz.to_vec() } else { Vec::new() };
        Self {
            ok: true,
            reason: String::new(),
            hit,
            digest: digest_of(&fields),
            fields,
        }
    }

    /// Builds a failed [`MockBody`] with `reason`.
    #[must_use]
    pub fn failure(reason: &str) -> Self {
        Self {
            ok: false,
            reason: reason.to_string(),
            hit: false,
            fields: Vec::new(),
            digest: String::new(),
        }
    }

    /// Converts into a [`SlotValue::Object`].
    #[must_use]
    pub fn to_value(&self) -> SlotValue {
        SlotValue::Object(vec![
            ("ok".to_string(), SlotValue::Bool(self.ok)),
            ("reason".to_string(), SlotValue::String(self.reason.clone())),
            ("hit".to_string(), SlotValue::Bool(self.hit)),
            ("fields".to_string(), biz_object(&self.fields)),
            ("digest".to_string(), SlotValue::String(self.digest.clone())),
        ])
    }
}

/// Computes lowercase SHA-256 hex of canonical UTF-8 sorted JSON for `biz`.
#[must_use]
pub fn digest_of(biz: &[(String, SlotValue)]) -> String {
    hex::encode(Sha256::digest(biz_object(biz).canonical().as_bytes()))
}

/// Front-isolated token store for `envelope_token_plain` and `envelope_token_crypt`.
#[derive(Clone, Debug)]
pub struct EnvelopeTokenStore {
    expire: Duration,
    inner: Arc<Mutex<HashMap<String, Instant>>>,
}

impl EnvelopeTokenStore {
    /// Creates an isolated token store with expiration `expire_minutes`.
    #[must_use]
    pub fn new(expire_minutes: u64) -> Self {
        Self {
            expire: Duration::from_secs(expire_minutes.saturating_mul(60).max(1)),
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Issues a random 32-byte URL-safe Base64 token and returns `(token, expires_in_secs)`.
    #[must_use]
    pub fn issue(&self) -> (String, u64) {
        let mut bytes = [0u8; 32];
        rand::fill(&mut bytes);
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        guard.retain(|_, exp| *exp > now);
        guard.insert(token.clone(), now + self.expire);
        (token, self.expire.as_secs())
    }

    /// Checks whether `token` exists and has not expired.
    #[must_use]
    pub fn contains(&self, token: &str) -> bool {
        let now = Instant::now();
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        guard.retain(|_, exp| *exp > now);
        guard.contains_key(token)
    }
}

/// Output produced by [`drive`].
#[derive(Clone, Debug)]
pub struct PipeOut {
    /// HTTP status code (`200`, `400`, `401`, `404`, `405`, `413`).
    pub status: u16,
    /// HTTP `Content-Type` header value.
    pub content_type: &'static str,
    /// Response body bytes.
    pub body: Vec<u8>,
    /// Optional error code for `access_log`.
    pub error: Option<String>,
    /// Redacted inbound `biz` JSON for `access_log`.
    pub log_in: Value,
    /// Unsealed outbound JSON for `access_log`.
    pub log_out: Value,
    /// Auth label (`envelope_token`, `envelope_sign`, `envelope_cred`).
    pub auth: &'static str,
}

/// Context passed into [`drive`].
pub struct DriveCtx<'a> {
    /// Envelope front name.
    pub front: FrontName,
    /// Active compiled profiles on this front.
    pub profiles: &'a [Profile],
    /// Optional front-scoped token store.
    pub tokens: Option<&'a EnvelopeTokenStore>,
    /// Front-scoped replay nonce store.
    pub nonces: &'a NonceStore,
    /// Allowed timestamp skew in seconds.
    pub sign_window_seconds: u64,
}

/// Executes the synchronous envelope verification, decryption, mock generation, and sealing pipeline.
#[must_use]
pub fn drive(ctx: &DriveCtx<'_>, req: &Incoming) -> PipeOut {
    let auth = Profile::auth_label(ctx.front);
    let path_hits: Vec<&Profile> = ctx.profiles.iter().filter(|p| p.path == req.path).collect();
    if path_hits.is_empty() {
        return plain(404, "not found", auth);
    }
    let Some(profile) = path_hits.into_iter().find(|p| p.allows(&req.method)) else {
        return plain(405, "method not allowed", auth);
    };
    let table = match slots::collect(profile, req) {
        Ok(t) => t,
        Err(SlotFail::TooLarge) => {
            return fail_http("bad_shape", 413, auth, Value::Object(Map::new()));
        }
        Err(SlotFail::Shape) => {
            return fail(profile, "bad_shape", false, auth, Value::Object(Map::new()));
        }
    };
    let log_in = biz_log(&table);
    match run_profile(profile, ctx, table) {
        Ok((body, log_out)) => PipeOut {
            status: 200,
            content_type: "application/json",
            body,
            error: None,
            log_in,
            log_out,
            auth,
        },
        Err((reason, can_env)) => fail(profile, reason, can_env, auth, log_in),
    }
}

fn run_profile(
    profile: &Profile,
    ctx: &DriveCtx<'_>,
    mut table: FieldTable,
) -> Result<(Vec<u8>, Value), (&'static str, bool)> {
    let mut can_env = profile.fail_in == FailIn::Envelope && profile.wrap.is_none();
    if profile.kind == Kind::Issue {
        check_credentials(profile, &table).map_err(|r| (r, false))?;
        let store = ctx.tokens.ok_or(("bad_token", false))?;
        let (token, exp) = store.issue();
        let field = profile
            .token_response_field
            .clone()
            .unwrap_or_else(|| "access_token".to_string());
        let val = SlotValue::Object(vec![
            (field, SlotValue::String(token)),
            ("expires_in".to_string(), SlotValue::Number(exp.to_string())),
        ]);
        return Ok((val.compact().into_bytes(), val.to_serde()));
    }
    if profile.order == Some(Order::EncryptThenSign) {
        decrypt_into(profile, &mut table).map_err(|r| (r, false))?;
        can_env = profile.fail_in == FailIn::Envelope;
    }
    verify_envelope_auth(profile, &table, ctx).map_err(|r| (r, can_env))?;
    if profile.crypt_alg.is_some() && profile.order != Some(Order::EncryptThenSign) {
        decrypt_into(profile, &mut table).map_err(|r| (r, false))?;
    }
    let ctrl: Vec<String> = profile
        .fields
        .iter()
        .filter(|f| f.role != Role::Biz)
        .map(|f| f.name.clone())
        .collect();
    table.strip_named(&ctrl);
    let mock = MockBody::success(table.biz());
    let log_out = mock.to_value().to_serde();
    let body = seal(profile, &mock.to_value()).map_err(|_| ("bad_decrypt", false))?;
    Ok((body, log_out))
}

fn verify_envelope_auth(
    profile: &Profile,
    table: &FieldTable,
    ctx: &DriveCtx<'_>,
) -> Result<(), &'static str> {
    if profile.sign_alg.is_some() && profile.order != Some(Order::EncryptThenSign) {
        check_sign(profile, table, ctx.nonces, ctx.sign_window_seconds, None)?;
    }
    if table.first(Role::Token).is_some() || profile.token_prefix.is_some() {
        let store = ctx.tokens.ok_or("bad_token")?;
        let raw = table
            .first(Role::Token)
            .map(SlotValue::text)
            .ok_or("bad_shape")?;
        let token = match profile.token_prefix.as_deref() {
            Some(pfx) => raw.strip_prefix(pfx).ok_or("bad_token")?,
            None => raw.as_str(),
        };
        if !store.contains(token) {
            return Err("bad_token");
        }
    }
    if !profile.secrets.is_empty() {
        check_credentials(profile, table)?;
    }
    if profile.sign_alg.is_some() && profile.order == Some(Order::EncryptThenSign) {
        let plain = table.raw_cipher.clone();
        check_sign(
            profile,
            table,
            ctx.nonces,
            ctx.sign_window_seconds,
            plain.as_deref(),
        )?;
    }
    Ok(())
}

fn check_credentials(profile: &Profile, table: &FieldTable) -> Result<(), &'static str> {
    if profile.secrets.is_empty() {
        return Err("bad_credential");
    }
    for (name, expect) in &profile.secrets {
        let got = table
            .get(Role::Auth, name)
            .map(SlotValue::text)
            .ok_or("bad_shape")?;
        if !secrets_equal_bytes(got.as_bytes(), expect.as_bytes()) {
            return Err("bad_credential");
        }
    }
    Ok(())
}

fn check_sign(
    profile: &Profile,
    table: &FieldTable,
    nonces: &NonceStore,
    window_secs: u64,
    plain_override: Option<&[u8]>,
) -> Result<(), &'static str> {
    if let Some(val) = table.first(Role::Timestamp) {
        let ts = val.text().parse::<i64>().map_err(|_| "bad_sign")?;
        let now = chrono::Utc::now().timestamp();
        let span = i64::try_from(window_secs).unwrap_or(i64::MAX);
        if (now - ts).abs() > span {
            return Err("bad_sign");
        }
    }
    if let Some(val) = table.first(Role::Nonce) {
        let n = val.text();
        if n.is_empty() || !nonces.insert(&n) {
            return Err("bad_sign");
        }
    }
    let provided = table
        .first(Role::Sign)
        .map(SlotValue::text)
        .ok_or("bad_shape")?;
    let bytes = canon::material(profile, table, plain_override);
    match crypt::verify(profile, &bytes, &provided) {
        Ok(true) => Ok(()),
        _ => Err("bad_sign"),
    }
}

fn decrypt_into(profile: &Profile, table: &mut FieldTable) -> Result<(), &'static str> {
    if profile.crypt_alg.is_none() {
        return Ok(());
    }
    let key = session_key(profile, table)?;
    let mut working = profile.clone();
    apply_fixed_iv(&mut working, table, &key)?;
    let mut plains = Vec::new();
    if profile.cipher_flags.scope_body {
        let raw = table.raw_cipher.clone().ok_or("bad_decrypt")?;
        plains.push(crypt::decrypt(&working, &key, &raw).map_err(map_crypt)?);
    } else {
        let names = cipher_names(profile);
        if names.is_empty() {
            return Err("bad_decrypt");
        }
        for name in names {
            let text = table
                .get(Role::Cipher, &name)
                .map(SlotValue::text)
                .ok_or("bad_shape")?;
            let decoded = crypt::decode_crypt(profile, &text).map_err(map_crypt)?;
            plains.push(crypt::decrypt(&working, &key, &decoded).map_err(map_crypt)?);
        }
    }
    if profile.order == Some(Order::EncryptThenSign) {
        table.raw_cipher = plains.first().cloned();
    }
    for plain in plains {
        let text = std::str::from_utf8(&plain).map_err(|_| "bad_decrypt")?;
        let SlotValue::Object(pairs) = parse_json(text).map_err(|_| "bad_decrypt")? else {
            return Err("bad_decrypt");
        };
        for (k, v) in pairs {
            table.insert(Role::Biz, k, v);
        }
    }
    Ok(())
}

fn session_key(profile: &Profile, table: &FieldTable) -> Result<Vec<u8>, &'static str> {
    if profile.wrap.is_some() {
        let text = table
            .first(Role::WrapKey)
            .map(SlotValue::text)
            .ok_or("bad_shape")?;
        let wrapped = crypt::decode_crypt(profile, &text).map_err(|_| "bad_decrypt")?;
        return unwrap_key(profile, &wrapped).map_err(|_| "bad_decrypt");
    }
    if matches!(
        profile.crypt_alg,
        Some(CryptAlg::RsaPkcs1Private | CryptAlg::RsaPkcs1Public | CryptAlg::Sm2Encrypt)
    ) {
        return Ok(Vec::new());
    }
    profile.key.clone().ok_or("bad_decrypt")
}

fn apply_fixed_iv(
    profile: &mut Profile,
    table: &FieldTable,
    key: &[u8],
) -> Result<(), &'static str> {
    let Some(alg) = profile.crypt_alg else {
        return Ok(());
    };
    match profile.iv_from {
        Some(IvFrom::Key) => {
            let len = if alg == CryptAlg::DesCbcPkcs5 { 8 } else { 16 };
            profile.fixed_iv = Some(key.get(..len).ok_or("bad_decrypt")?.to_vec());
        }
        Some(IvFrom::Field) => {
            let text = table
                .get(Role::Auth, "iv")
                .map(SlotValue::text)
                .ok_or("bad_shape")?;
            profile.fixed_iv =
                Some(crypt::decode_crypt(profile, &text).map_err(|_| "bad_decrypt")?);
        }
        _ => {}
    }
    Ok(())
}

fn seal(profile: &Profile, result: &SlotValue) -> Result<Vec<u8>, CryptError> {
    let plain = result.compact().into_bytes();
    let crypt_alg = if profile.cipher_flags.response_plain {
        None
    } else {
        profile.response_crypt.or(profile.crypt_alg)
    };
    let Some(alg) = crypt_alg else {
        return Ok(sign_plain(profile, result));
    };
    let key = profile.key.clone().unwrap_or_default();
    let mut working = profile.clone();
    if matches!(
        alg,
        CryptAlg::AesCfb | CryptAlg::AesGcm | CryptAlg::DesCbcPkcs5 | CryptAlg::Sm4CbcPkcs7
    ) {
        working.fixed_iv = Some(crypt::prepare_iv(&working, alg)?);
    }
    working.crypt_alg = Some(alg);
    let sealed = crypt::encrypt(&working, &key, &plain)?;
    let encoded = crypt::encode_crypt(profile, &sealed);
    let mut pairs = vec![(cipher_field(profile), SlotValue::String(encoded.clone()))];
    if profile.wrap.is_some() {
        let wrapped = wrap_key(profile, &key)?;
        pairs.push((
            "wrap_key".to_string(),
            SlotValue::String(crypt::encode_crypt(profile, &wrapped)),
        ));
    }
    if profile.sign_alg.is_some() {
        let mat = if profile.order == Some(Order::EncryptThenSign) {
            plain
        } else {
            encoded.into_bytes()
        };
        pairs.push((
            sign_field(profile),
            SlotValue::String(crypt::sign(profile, &mat)?),
        ));
    }
    Ok(SlotValue::Object(pairs).compact().into_bytes())
}

fn sign_plain(profile: &Profile, result: &SlotValue) -> Vec<u8> {
    if profile.sign_alg.is_none() {
        return result.compact().into_bytes();
    }
    let mut signed = result.clone();
    let SlotValue::Object(pairs) = &mut signed else {
        return result.compact().into_bytes();
    };
    let mut scratch = FieldTable::default();
    for (k, v) in pairs.iter() {
        scratch.insert(Role::Biz, k.clone(), v.clone());
    }
    let bytes = if profile.canon == Some(crate::profile::CanonKind::Raw)
        && profile
            .canon_fields
            .first()
            .is_some_and(|n| scratch.biz().iter().all(|(k, _)| k != n))
    {
        result.compact().into_bytes()
    } else {
        canon::material(profile, &scratch, None)
    };
    if let Ok(sig) = crypt::sign(profile, &bytes) {
        pairs.push((sign_field(profile), SlotValue::String(sig)));
    }
    signed.compact().into_bytes()
}

fn cipher_names(profile: &Profile) -> Vec<String> {
    if !profile.cipher_fields.is_empty() {
        return profile.cipher_fields.clone();
    }
    profile
        .fields
        .iter()
        .filter(|f| f.role == Role::Cipher)
        .map(|f| f.name.clone())
        .collect()
}

fn cipher_field(profile: &Profile) -> String {
    cipher_names(profile)
        .into_iter()
        .next()
        .unwrap_or_else(|| "payload".to_string())
}

fn sign_field(profile: &Profile) -> String {
    profile
        .fields
        .iter()
        .find(|f| f.role == Role::Sign)
        .map_or_else(|| "sign".to_string(), |f| f.name.clone())
}

fn map_crypt(err: CryptError) -> &'static str {
    match err {
        CryptError::Mismatch => "bad_sign",
        CryptError::Bad => "bad_decrypt",
    }
}

fn biz_log(table: &FieldTable) -> Value {
    let mut map = Map::new();
    for (k, v) in table.biz() {
        map.insert(k.clone(), v.to_serde());
    }
    Value::Object(map)
}

fn fail(
    profile: &Profile,
    reason: &str,
    can_env: bool,
    auth: &'static str,
    log_in: Value,
) -> PipeOut {
    let mock = MockBody::failure(reason);
    if can_env
        && profile.fail_in == FailIn::Envelope
        && let Ok(body) = seal(profile, &mock.to_value())
    {
        return PipeOut {
            status: 200,
            content_type: "application/json",
            body,
            error: Some(reason.to_string()),
            log_in,
            log_out: mock.to_value().to_serde(),
            auth,
        };
    }
    let status = if matches!(reason, "bad_token" | "bad_sign" | "bad_credential") {
        401
    } else {
        400
    };
    fail_http(reason, status, auth, log_in)
}

fn fail_http(reason: &str, status: u16, auth: &'static str, log_in: Value) -> PipeOut {
    let mock = MockBody::failure(reason);
    PipeOut {
        status,
        content_type: "application/json",
        body: mock.to_value().compact().into_bytes(),
        error: Some(reason.to_string()),
        log_in,
        log_out: mock.to_value().to_serde(),
        auth,
    }
}

fn plain(status: u16, msg: &str, auth: &'static str) -> PipeOut {
    PipeOut {
        status,
        content_type: "text/plain",
        body: msg.as_bytes().to_vec(),
        error: Some(msg.to_string()),
        log_in: Value::Object(Map::new()),
        log_out: Value::Null,
        auth,
    }
}
