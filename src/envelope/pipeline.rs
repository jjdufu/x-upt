use serde_json::{Map, Value};

use crate::auth::nonce::NonceStore;
use crate::config::{FrontName, LatencyConfig, SignWindow, apply_latency};
use crate::envelope::auth::{self, AuthFail};
use crate::envelope::crypt::{self, CryptError};
use crate::envelope::mock::MockBody;
use crate::envelope::profile::{FailIn, Kind, Order, Profile, Role};
use crate::envelope::slots::{self, Incoming, SlotFail};
use crate::envelope::table::{FieldTable, SlotValue, parse_json};
use crate::envelope::token_store::EnvelopeTokenStore;

#[derive(Clone, Debug)]
pub struct PipeOut {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub error: Option<String>,
    pub log_in: Value,
    pub log_out: Value,
    pub auth: &'static str,
    pub latency_ms: u64,
}

pub async fn drive(
    front: FrontName,
    profiles: &[Profile],
    tokens: Option<&EnvelopeTokenStore>,
    nonces: &NonceStore,
    sign: &SignWindow,
    latency: &LatencyConfig,
    req: &Incoming,
) -> PipeOut {
    let auth = Profile::auth_label(front);
    let path_hits: Vec<&Profile> = profiles
        .iter()
        .filter(|item| item.path == req.path)
        .collect();
    if path_hits.is_empty() {
        return plain(404, "not found", auth, Value::Null);
    }
    let Some(profile) = path_hits.into_iter().find(|item| item.allows(&req.method)) else {
        return plain(405, "method not allowed", auth, Value::Null);
    };
    let table = match slots::collect(profile, req) {
        Ok(table) => table,
        Err(SlotFail::TooLarge) => {
            return fail_http(profile, "bad_shape", 413, auth, Value::Object(Map::new()));
        }
        Err(SlotFail::Shape) => {
            return fail(profile, "bad_shape", false, auth, Value::Object(Map::new()));
        }
    };
    let log_in = biz_log(&table);
    let latency_ms = apply_latency(latency).await;
    match run_profile(profile, tokens, nonces, sign, table).await {
        Ok(done) => PipeOut {
            status: 200,
            content_type: "application/json",
            body: done.body,
            error: None,
            log_in,
            log_out: done.log_out,
            auth,
            latency_ms,
        },
        Err(blown) => {
            let mut out = fail(profile, blown.reason, blown.can_envelope, auth, log_in);
            out.latency_ms = latency_ms;
            out
        }
    }
}

struct Done {
    body: Vec<u8>,
    log_out: Value,
}

struct Blown {
    reason: &'static str,
    can_envelope: bool,
}

async fn run_profile(
    profile: &Profile,
    tokens: Option<&EnvelopeTokenStore>,
    nonces: &NonceStore,
    sign: &SignWindow,
    mut table: FieldTable,
) -> Result<Done, Blown> {
    let mut can_envelope = profile.fail_in == FailIn::Envelope && profile.wrap.is_none();
    if profile.kind == Kind::Issue {
        auth::check_credentials(profile, &table).map_err(|err| blown(err, false))?;
        let store = tokens.ok_or(blown(AuthFail::Token, false))?;
        let (token, expires) = store.issue();
        let field = profile
            .token_response_field
            .clone()
            .unwrap_or_else(|| "access_token".to_string());
        let value = SlotValue::Object(vec![
            (field, SlotValue::String(token)),
            (
                "expires_in".to_string(),
                SlotValue::Number(expires.to_string()),
            ),
        ]);
        return Ok(Done {
            log_out: value.to_serde(),
            body: value.compact().into_bytes(),
        });
    }

    if profile.order == Some(Order::EncryptThenSign) {
        decrypt_into(profile, &mut table).map_err(|err| {
            can_envelope = false;
            blown(err, false)
        })?;
        can_envelope = profile.fail_in == FailIn::Envelope;
    }
    if let Err(err) = authenticate(profile, &table, tokens, nonces, sign) {
        return Err(blown(err, can_envelope));
    }
    if profile.crypt_alg.is_some()
        && profile.order != Some(Order::EncryptThenSign)
        && let Err(err) = decrypt_into(profile, &mut table)
    {
        return Err(blown(err, false));
    }
    strip_control(profile, &mut table);
    let mock = MockBody::success(table.biz());
    let log_out = mock.to_value().to_serde();
    let body = seal(profile, &mock.to_value()).map_err(|_| blown(AuthFail::Decrypt, false))?;
    Ok(Done { body, log_out })
}

fn authenticate(
    profile: &Profile,
    table: &FieldTable,
    tokens: Option<&EnvelopeTokenStore>,
    nonces: &NonceStore,
    sign: &SignWindow,
) -> Result<(), AuthFail> {
    if profile.sign_alg.is_some() && profile.order != Some(Order::EncryptThenSign) {
        let plain = None;
        auth::check_sign(profile, table, nonces, sign, plain)?;
    }
    if table.first(crate::envelope::table::Role::Token).is_some() || profile.token_prefix.is_some()
    {
        let store = tokens.ok_or(AuthFail::Token)?;
        auth::check_token(profile, table, store)?;
    }
    if !profile.secrets.is_empty() {
        auth::check_credentials(profile, table)?;
    }
    if profile.sign_alg.is_some() && profile.order == Some(Order::EncryptThenSign) {
        let plain = table.raw_cipher.clone();
        auth::check_sign(profile, table, nonces, sign, plain.as_deref())?;
    }
    Ok(())
}

fn decrypt_into(profile: &Profile, table: &mut FieldTable) -> Result<(), AuthFail> {
    if profile.crypt_alg.is_none() {
        return Ok(());
    }
    let key = session_key(profile, table)?;
    let mut working = profile.clone();
    apply_fixed_iv(&mut working, table, &key)?;
    let mut plains = Vec::new();
    if profile.cipher_scope_body() {
        let raw = table.raw_cipher.clone().ok_or(AuthFail::Decrypt)?;
        plains.push(crypt::decrypt(&working, &key, &raw).map_err(map_crypt)?);
    } else {
        let names = cipher_names(profile);
        if names.is_empty() {
            return Err(AuthFail::Decrypt);
        }
        for name in names {
            let text = table
                .get(crate::envelope::table::Role::Cipher, &name)
                .map(SlotValue::text)
                .ok_or(AuthFail::Shape)?;
            let decoded = crypt::decode_crypt(profile, &text).map_err(map_crypt)?;
            let plain = crypt::decrypt(&working, &key, &decoded).map_err(map_crypt)?;
            plains.push(plain);
        }
    }
    if profile.order == Some(Order::EncryptThenSign) {
        table.raw_cipher = plains.first().cloned();
    }
    for plain in plains {
        merge_plain(table, &plain)?;
    }
    Ok(())
}

fn merge_plain(table: &mut FieldTable, plain: &[u8]) -> Result<(), AuthFail> {
    let text = std::str::from_utf8(plain).map_err(|_| AuthFail::Decrypt)?;
    let value = parse_json(text).map_err(|()| AuthFail::Decrypt)?;
    let SlotValue::Object(pairs) = value else {
        return Err(AuthFail::Decrypt);
    };
    for (key, value) in pairs {
        table.insert(crate::envelope::table::Role::Biz, key, value);
    }
    Ok(())
}

fn session_key(profile: &Profile, table: &FieldTable) -> Result<Vec<u8>, AuthFail> {
    if profile.wrap.is_some() {
        let text = table
            .first(crate::envelope::table::Role::WrapKey)
            .map(SlotValue::text)
            .ok_or(AuthFail::Shape)?;
        let wrapped = crypt::decode_crypt(profile, &text).map_err(|_| AuthFail::Decrypt)?;
        return crypt::unwrap_key(profile, &wrapped).map_err(|_| AuthFail::Decrypt);
    }
    if matches!(
        profile.crypt_alg,
        Some(
            crate::envelope::profile::CryptAlg::RsaPkcs1Private
                | crate::envelope::profile::CryptAlg::RsaPkcs1Public
                | crate::envelope::profile::CryptAlg::Sm2Encrypt
        )
    ) {
        return Ok(Vec::new());
    }
    profile.key.clone().ok_or(AuthFail::Decrypt)
}

fn apply_fixed_iv(profile: &mut Profile, table: &FieldTable, key: &[u8]) -> Result<(), AuthFail> {
    let Some(alg) = profile.crypt_alg else {
        return Ok(());
    };
    match profile.iv_from {
        Some(crate::envelope::profile::IvFrom::Key) => {
            let len = if alg == crate::envelope::profile::CryptAlg::DesCbcPkcs5 {
                8
            } else {
                16
            };
            if key.len() < len {
                return Err(AuthFail::Decrypt);
            }
            profile.fixed_iv = Some(key[..len].to_vec());
        }
        Some(crate::envelope::profile::IvFrom::Field) => {
            let text = table
                .get(crate::envelope::table::Role::Auth, "iv")
                .map(SlotValue::text)
                .ok_or(AuthFail::Shape)?;
            profile.fixed_iv =
                Some(crypt::decode_crypt(profile, &text).map_err(|_| AuthFail::Decrypt)?);
        }
        _ => {}
    }
    Ok(())
}

fn cipher_names(profile: &Profile) -> Vec<String> {
    if !profile.cipher_fields.is_empty() {
        return profile.cipher_fields.clone();
    }
    profile
        .fields
        .iter()
        .filter(|field| field.role == Role::Cipher)
        .map(|field| field.name.clone())
        .collect()
}

fn strip_control(profile: &Profile, table: &mut FieldTable) {
    let names: Vec<String> = profile
        .fields
        .iter()
        .filter(|field| field.role != Role::Biz)
        .map(|field| field.name.clone())
        .collect();
    table.strip_named(&names);
}

fn seal(profile: &Profile, result: &SlotValue) -> Result<Vec<u8>, CryptError> {
    let plain = match (profile.response_body_kind, profile.response_encoding) {
        (
            crate::envelope::profile::BodyKind::Json
            | crate::envelope::profile::BodyKind::Form
            | crate::envelope::profile::BodyKind::Raw
            | crate::envelope::profile::BodyKind::Empty,
            crate::envelope::profile::BodyEncoding::Identity
            | crate::envelope::profile::BodyEncoding::Base64,
        ) => result.compact().into_bytes(),
    };
    let crypt_alg = if profile.response_plain {
        None
    } else {
        profile.response_crypt.or(profile.crypt_alg)
    };
    if crypt_alg.is_none() {
        return Ok(sign_plain(profile, result));
    }
    let key = profile.key.clone().unwrap_or_default();
    let mut working = profile.clone();
    if let Some(alg) = crypt_alg {
        if needs_iv(alg) {
            working.fixed_iv = Some(crypt::prepare_iv(&working, alg)?);
        }
        working.crypt_alg = Some(alg);
    }
    let sealed = crypt::encrypt(&working, &key, &plain)?;
    let encoded = crypt::encode_crypt(profile, &sealed);
    let mut pairs = vec![(cipher_field(profile), SlotValue::String(encoded.clone()))];
    if profile.wrap.is_some() {
        let wrapped = crypt::wrap_key(profile, &key)?;
        pairs.push((
            "wrap_key".to_string(),
            SlotValue::String(crypt::encode_crypt(profile, &wrapped)),
        ));
    }
    if profile.sign_alg.is_some() {
        let material = if profile.order == Some(Order::EncryptThenSign) {
            plain
        } else {
            encoded.into_bytes()
        };
        let signature = crypt::sign(profile, &material)?;
        pairs.push((sign_field(profile), SlotValue::String(signature)));
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
    for (key, value) in pairs.iter() {
        scratch.insert(
            crate::envelope::table::Role::Biz,
            key.clone(),
            value.clone(),
        );
    }
    let bytes = if profile.canon == Some(crate::envelope::profile::CanonKind::Raw)
        && profile
            .canon_fields
            .first()
            .is_some_and(|name| scratch.biz().iter().all(|(key, _)| key != name))
    {
        result.compact().into_bytes()
    } else {
        crate::envelope::canon::material(profile, &scratch, None)
    };
    if let Ok(signature) = crypt::sign(profile, &bytes) {
        pairs.push((sign_field(profile), SlotValue::String(signature)));
    }
    signed.compact().into_bytes()
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
        .find(|field| field.role == Role::Sign)
        .map(|field| field.name.clone())
        .unwrap_or_else(|| "sign".to_string())
}

fn needs_iv(alg: crate::envelope::profile::CryptAlg) -> bool {
    matches!(
        alg,
        crate::envelope::profile::CryptAlg::AesCfb
            | crate::envelope::profile::CryptAlg::AesGcm
            | crate::envelope::profile::CryptAlg::DesCbcPkcs5
            | crate::envelope::profile::CryptAlg::Sm4CbcPkcs7
    )
}

fn blown(err: AuthFail, can_envelope: bool) -> Blown {
    Blown {
        reason: match err {
            AuthFail::Token => "bad_token",
            AuthFail::Sign => "bad_sign",
            AuthFail::Credential => "bad_credential",
            AuthFail::Decrypt => "bad_decrypt",
            AuthFail::Shape => "bad_shape",
        },
        can_envelope,
    }
}

fn map_crypt(err: CryptError) -> AuthFail {
    match err {
        CryptError::Mismatch => AuthFail::Sign,
        CryptError::Bad => AuthFail::Decrypt,
    }
}

fn biz_log(table: &FieldTable) -> Value {
    let mut map = Map::new();
    for (key, value) in table.biz() {
        map.insert(key.clone(), value.to_serde());
    }
    Value::Object(map)
}

fn fail(
    profile: &Profile,
    reason: &str,
    can_envelope: bool,
    auth: &'static str,
    log_in: Value,
) -> PipeOut {
    let mock = MockBody::failure(reason);
    let log_out = mock.to_value().to_serde();
    if can_envelope
        && profile.fail_in == FailIn::Envelope
        && let Ok(body) = seal(profile, &mock.to_value())
    {
        return PipeOut {
            status: 200,
            content_type: "application/json",
            body,
            error: Some(reason.to_string()),
            log_in,
            log_out,
            auth,
            latency_ms: 0,
        };
    }
    let status = if reason == "bad_token" || reason == "bad_sign" || reason == "bad_credential" {
        401
    } else {
        400
    };
    fail_http(profile, reason, status, auth, log_in)
}

fn fail_http(
    profile: &Profile,
    reason: &str,
    status: u16,
    auth: &'static str,
    log_in: Value,
) -> PipeOut {
    let _ = profile;
    let mock = MockBody::failure(reason);
    PipeOut {
        status,
        content_type: "application/json",
        body: mock.to_value().compact().into_bytes(),
        error: Some(reason.to_string()),
        log_in,
        log_out: mock.to_value().to_serde(),
        auth,
        latency_ms: 0,
    }
}

fn plain(status: u16, message: &str, auth: &'static str, log_out: Value) -> PipeOut {
    PipeOut {
        status,
        content_type: "text/plain",
        body: message.as_bytes().to_vec(),
        error: Some(message.to_string()),
        log_in: Value::Object(Map::new()),
        log_out,
        auth,
        latency_ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::drive;
    use crate::auth::nonce::NonceStore;
    use crate::config::{FrontName, LatencyConfig, SignWindow};
    use crate::envelope::crypt;
    use crate::envelope::mock::hex_sha256;
    use crate::envelope::profile::{
        BodyEncoding, BodyKind, CanonKind, CryptAlg, Encoding, FailIn, FieldSpec, HexCase, IvFrom,
        Kind, Profile, Role, SignAlg, Slot, WrapAlg,
    };
    use crate::envelope::slots::Incoming;
    use crate::envelope::table::parse_json;
    use crate::envelope::token_store::EnvelopeTokenStore;

    fn quiet() -> (LatencyConfig, SignWindow, NonceStore) {
        (
            LatencyConfig {
                min_ms: 0,
                max_ms: 0,
            },
            SignWindow {
                window_seconds: 300,
                nonce_ttl_seconds: 600,
            },
            NonceStore::new(600),
        )
    }

    fn req(
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        query: &str,
        body: &str,
    ) -> Incoming {
        Incoming {
            method: method.to_string(),
            path: path.to_string(),
            query: query.to_string(),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[tokio::test]
    async fn token_plain_success_and_failure() {
        let (latency, sign, nonces) = quiet();
        let store = EnvelopeTokenStore::new(30, 300);
        let mut issue = Profile::test_call(
            BodyKind::Form,
            BodyEncoding::Identity,
            vec![
                FieldSpec::new("username", Slot::Form, Role::Auth),
                FieldSpec::new("password", Slot::Form, Role::Auth),
            ],
            false,
        );
        issue.kind = Kind::Issue;
        issue.path = "/issue".to_string();
        issue
            .secrets
            .insert("username".to_string(), "alice".to_string());
        issue
            .secrets
            .insert("password".to_string(), "secret".to_string());
        issue.token_response_field = Some("access_token".to_string());
        let mut call = Profile::test_call(
            BodyKind::Json,
            BodyEncoding::Identity,
            vec![FieldSpec::new("authorization", Slot::Header, Role::Token)],
            true,
        );
        call.token_prefix = Some("Bearer ".to_string());
        let profiles = [issue, call];
        let bad = drive(
            FrontName::EnvelopeTokenPlain,
            &profiles,
            Some(&store),
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/issue",
                &[("content-type", "application/x-www-form-urlencoded")],
                "",
                "username=alice&password=no",
            ),
        )
        .await;
        assert_eq!(bad.status, 401);
        assert!(
            std::str::from_utf8(&bad.body)
                .unwrap()
                .contains("bad_credential")
        );

        let issued = drive(
            FrontName::EnvelopeTokenPlain,
            &profiles,
            Some(&store),
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/issue",
                &[("content-type", "application/x-www-form-urlencoded")],
                "",
                "username=alice&password=secret",
            ),
        )
        .await;
        assert_eq!(issued.status, 200);
        let text = std::str::from_utf8(&issued.body).unwrap();
        let value = parse_json(text).unwrap();
        let token = match value {
            crate::envelope::table::SlotValue::Object(pairs) => pairs
                .into_iter()
                .find(|(k, _)| k == "access_token")
                .unwrap()
                .1
                .text(),
            _ => panic!("issue body"),
        };
        let ok = drive(
            FrontName::EnvelopeTokenPlain,
            &profiles,
            Some(&store),
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/call",
                &[
                    ("content-type", "application/json"),
                    ("authorization", &format!("Bearer {token}")),
                ],
                "",
                r#"{"city":"hz"}"#,
            ),
        )
        .await;
        let body = std::str::from_utf8(&ok.body).unwrap();
        assert!(body.contains(r#""hit":true"#), "{body}");
        assert!(body.contains(&hex_sha256(br#"{"city":"hz"}"#)), "{body}");
        let other = EnvelopeTokenStore::new(30, 300);
        let crossed = drive(
            FrontName::EnvelopeTokenPlain,
            &profiles,
            Some(&other),
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/call",
                &[
                    ("content-type", "application/json"),
                    ("authorization", &format!("bearer {token}")),
                ],
                "",
                r#"{"city":"hz"}"#,
            ),
        )
        .await;
        assert_eq!(crossed.status, 401);
        assert!(
            std::str::from_utf8(&crossed.body)
                .unwrap()
                .contains("bad_token")
        );
    }

    #[tokio::test]
    async fn des_response_is_plain_json() {
        let (latency, sign, nonces) = quiet();
        let mut profile = Profile::test_call(
            BodyKind::Form,
            BodyEncoding::Identity,
            vec![
                FieldSpec::new("username", Slot::Form, Role::Auth),
                FieldSpec::new("params", Slot::Form, Role::Cipher),
            ],
            false,
        );
        profile
            .secrets
            .insert("username".to_string(), "alice".to_string());
        profile.crypt_alg = Some(CryptAlg::DesCbcPkcs5);
        profile.cipher_fields = vec!["params".to_string()];
        profile.iv_from = Some(IvFrom::Key);
        profile.key = Some(b"des-key!".to_vec());
        profile.response_plain = true;
        profile.response_crypt = None;
        profile.crypt_encoding = Encoding::Base64;
        let mut working = profile.clone();
        working.fixed_iv = Some(b"des-key!".to_vec());
        let cipher = crypt::encrypt_alg(
            &working,
            CryptAlg::DesCbcPkcs5,
            b"des-key!",
            br#"{"city":"hz"}"#,
        )
        .unwrap();
        let encoded = crypt::encode_crypt(&profile, &cipher);
        let body = format!("username=alice&params={}", urlencoding(&encoded));
        let out = drive(
            FrontName::EnvelopeCredCrypt,
            &[profile],
            None,
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/call",
                &[("content-type", "application/x-www-form-urlencoded")],
                "",
                &body,
            ),
        )
        .await;
        let text = std::str::from_utf8(&out.body).unwrap();
        assert_eq!(out.status, 200, "{text}");
        assert!(text.contains(r#""hit":true"#), "{text}");
    }

    fn urlencoding(raw: &str) -> String {
        url::form_urlencoded::byte_serialize(raw.as_bytes()).collect()
    }

    #[tokio::test]
    async fn missing_wrap_key_stays_http() {
        let (latency, sign, nonces) = quiet();
        let mut profile = Profile::test_call(
            BodyKind::Json,
            BodyEncoding::Identity,
            vec![FieldSpec::new("payload", Slot::Json, Role::Cipher)],
            false,
        );
        profile.fail_in = FailIn::Envelope;
        profile.wrap = Some(WrapAlg::RsaPkcs1Public);
        profile.crypt_alg = Some(CryptAlg::AesEcbPkcs5);
        profile.key = Some(vec![1u8; 16]);
        profile.cipher_fields = vec!["payload".to_string()];
        let out = drive(
            FrontName::EnvelopeSignCrypt,
            &[profile],
            None,
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/call",
                &[("content-type", "application/json")],
                "",
                r#"{"payload":"aaaa"}"#,
            ),
        )
        .await;
        assert_ne!(out.status, 200);
        assert!(std::str::from_utf8(&out.body).unwrap().contains("bad_"));
    }

    #[tokio::test]
    async fn sign_plain_md5_and_sm3() {
        let (latency, sign, nonces) = quiet();
        let mut md5p = Profile::test_call(
            BodyKind::Json,
            BodyEncoding::Identity,
            vec![FieldSpec::new("sign", Slot::Json, Role::Sign)],
            true,
        );
        md5p.sign_alg = Some(SignAlg::Md5);
        md5p.sign_encoding = Encoding::Hex;
        md5p.hex_case = HexCase::Lower;
        md5p.canon = Some(CanonKind::SortedKv);
        md5p.canon_skip = vec!["sign".to_string()];
        md5p.canon_sep = "&".to_string();
        md5p.canon_suffix_key = true;
        md5p.key = Some(b"secret".to_vec());
        let material = b"city=hz&secret";
        let sig = crypt::sign(&md5p, material).unwrap();
        let ok = drive(
            FrontName::EnvelopeSignPlain,
            &[md5p.clone()],
            None,
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/call",
                &[("content-type", "application/json")],
                "",
                &format!(r#"{{"city":"hz","sign":"{sig}"}}"#),
            ),
        )
        .await;
        assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
        let bad = drive(
            FrontName::EnvelopeSignPlain,
            &[md5p],
            None,
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/call",
                &[("content-type", "application/json")],
                "",
                r#"{"city":"hz","sign":"00"}"#,
            ),
        )
        .await;
        assert_eq!(bad.status, 401);

        let mut sm3 = Profile::test_call(
            BodyKind::Json,
            BodyEncoding::Identity,
            vec![
                FieldSpec::new("sign", Slot::Json, Role::Sign),
                FieldSpec::new("payload", Slot::Json, Role::Biz),
            ],
            false,
        );
        sm3.path = "/sm3".to_string();
        sm3.sign_alg = Some(SignAlg::Sm3);
        sm3.canon = Some(CanonKind::Raw);
        sm3.canon_fields = vec!["payload".to_string()];
        sm3.sign_encoding = Encoding::Hex;
        sm3.hex_case = HexCase::Lower;
        let sig = crypt::sign(&sm3, b"hello").unwrap();
        let ok = drive(
            FrontName::EnvelopeSignPlain,
            &[sm3],
            None,
            &nonces,
            &sign,
            &latency,
            &req(
                "POST",
                "/sm3",
                &[("content-type", "application/json")],
                "",
                &format!(r#"{{"payload":"hello","sign":"{sig}"}}"#),
            ),
        )
        .await;
        assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
    }

    #[test]
    fn oversized_maps_to_413() {
        let body = vec![b'a'; crate::envelope::slots::BODY_LIMIT + 1];
        let incoming = Incoming {
            method: "POST".to_string(),
            path: "/call".to_string(),
            query: String::new(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body,
        };
        let profile = Profile::test_call_empty();
        let (latency, sign, nonces) = quiet();
        let out = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(drive(
                FrontName::EnvelopeCredPlain,
                &[profile],
                None,
                &nonces,
                &sign,
                &latency,
                &incoming,
            ));
        assert_eq!(out.status, 413);
        assert!(
            std::str::from_utf8(&out.body)
                .unwrap()
                .contains("bad_shape")
        );
    }
}
