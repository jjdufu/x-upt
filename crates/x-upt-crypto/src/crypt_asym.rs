//! Asymmetric RSA and SM2 envelope encryption, signature, key-wrapping, and key generation helpers.

use base64::Engine;
use rsa::hazmat::{rsa_decrypt, rsa_encrypt};
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{BoxedUint, Pkcs1v15Encrypt, PrivateKey as RsaPrivateKey, PublicKey as RsaPublicKey};
use sha2::{Digest, Sha256};
use x_upt_core::domain::{EnvelopeSecretRecord, SecretKind};

use crate::crypt::CryptError;
use crate::profile::{Profile, Sm2Mode, WrapAlg};

/// Encrypts `plain` with RSA PKCS#1 v1.5 public key.
///
/// # Errors
/// Returns [`CryptError::Bad`] if key is missing or encryption fails.
pub fn rsa_public_encrypt(profile: &Profile, plain: &[u8]) -> Result<Vec<u8>, CryptError> {
    let key = profile.rsa_public.as_ref().ok_or(CryptError::Bad)?;
    key.encrypt(&mut rand::rngs::SysRng, Pkcs1v15Encrypt, plain)
        .map_err(|_| CryptError::Bad)
}

/// Decrypts `cipher` with RSA PKCS#1 v1.5 private key.
///
/// # Errors
/// Returns [`CryptError::Bad`] if key is missing or decryption fails.
pub fn rsa_private_decrypt(profile: &Profile, cipher: &[u8]) -> Result<Vec<u8>, CryptError> {
    let key = profile.rsa_private.as_ref().ok_or(CryptError::Bad)?;
    key.decrypt(Pkcs1v15Encrypt, cipher)
        .map_err(|_| CryptError::Bad)
}

/// Encrypts `plain` using RSA PKCS#1 type-1 private key block.
///
/// # Errors
/// Returns [`CryptError::Bad`] if key is missing or message exceeds modulus capacity.
pub fn rsa_private_encrypt(profile: &Profile, plain: &[u8]) -> Result<Vec<u8>, CryptError> {
    let key = profile.rsa_private.as_ref().ok_or(CryptError::Bad)?;
    let k = key.size();
    let padded = pkcs1_type1(plain, k)?;
    let m = uint_below_modulus(&padded, key.n().as_ref())?;
    let c = rsa_decrypt(Some(&mut rand::rngs::SysRng), key, &m).map_err(|_| CryptError::Bad)?;
    let full = c.to_be_bytes();
    let start = full.iter().position(|&b| b != 0).unwrap_or(full.len());
    Ok(left_pad(full.get(start..).unwrap_or(&[]), k))
}

/// Decrypts `cipher` using RSA PKCS#1 type-1 public key block.
///
/// # Errors
/// Returns [`CryptError::Bad`] if block length or PKCS#1 type-1 padding is invalid.
pub fn rsa_public_decrypt(profile: &Profile, cipher: &[u8]) -> Result<Vec<u8>, CryptError> {
    let key = profile.rsa_public.as_ref().ok_or(CryptError::Bad)?;
    let k = key.size();
    if cipher.len() != k {
        return Err(CryptError::Bad);
    }
    let c = uint_below_modulus(cipher, key.n().as_ref())?;
    let m = rsa_encrypt(key, &c).map_err(|_| CryptError::Bad)?;
    let full = m.to_be_bytes();
    let start = full.iter().position(|&b| b != 0).unwrap_or(full.len());
    let opened = left_pad(full.get(start..).unwrap_or(&[]), k);
    pkcs1_type1_unpad(&opened)
}

fn uint_below_modulus(bytes: &[u8], n: &BoxedUint) -> Result<BoxedUint, CryptError> {
    let value = BoxedUint::from_be_slice(bytes, n.bits_precision()).map_err(|_| CryptError::Bad)?;
    if value >= *n {
        return Err(CryptError::Bad);
    }
    Ok(value)
}

fn left_pad(raw: &[u8], len: usize) -> Vec<u8> {
    if raw.len() >= len {
        return raw.get(raw.len() - len..).unwrap_or(&[]).to_vec();
    }
    let mut out = vec![0u8; len - raw.len()];
    out.extend_from_slice(raw);
    out
}

fn pkcs1_type1(msg: &[u8], k: usize) -> Result<Vec<u8>, CryptError> {
    if k < 11 || msg.len() + 11 > k {
        return Err(CryptError::Bad);
    }
    let mut out = vec![0xffu8; k];
    if let Some(b0) = out.get_mut(0) {
        *b0 = 0x00;
    }
    if let Some(b1) = out.get_mut(1) {
        *b1 = 0x01;
    }
    let sep = k - msg.len() - 1;
    if let Some(bs) = out.get_mut(sep) {
        *bs = 0x00;
    }
    if let Some(tail) = out.get_mut(sep + 1..) {
        tail.copy_from_slice(msg);
    }
    Ok(out)
}

fn pkcs1_type1_unpad(block: &[u8]) -> Result<Vec<u8>, CryptError> {
    if block.len() < 11 || block.first() != Some(&0x00) || block.get(1) != Some(&0x01) {
        return Err(CryptError::Bad);
    }
    let mut i = 2usize;
    while block.get(i) == Some(&0xff) {
        i += 1;
    }
    if i < 10 || block.get(i) != Some(&0x00) {
        return Err(CryptError::Bad);
    }
    Ok(block.get(i + 1..).unwrap_or(&[]).to_vec())
}

/// Signs `material` with PKCS#1 v1.5 `SHA256WithRSA`.
///
/// # Errors
/// Returns [`CryptError::Bad`] if RSA private key is absent.
pub fn rsa_sign(profile: &Profile, material: &[u8]) -> Result<Vec<u8>, CryptError> {
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    let key = profile.rsa_private.as_ref().ok_or(CryptError::Bad)?;
    let signer = SigningKey::<Sha256>::new(key.clone());
    Ok(Signer::sign(&signer, material).to_bytes().into_vec())
}

/// Verifies PKCS#1 v1.5 `SHA256WithRSA` signature.
///
/// # Errors
/// Returns [`CryptError::Bad`] if RSA public key or signature format is invalid.
pub fn rsa_verify(profile: &Profile, material: &[u8], sig: &[u8]) -> Result<bool, CryptError> {
    use rsa::pkcs1v15::VerifyingKey;
    use rsa::signature::Verifier;
    let key = profile.rsa_public.as_ref().ok_or(CryptError::Bad)?;
    let verifier = VerifyingKey::<Sha256>::new(key.clone());
    let signature = rsa::pkcs1v15::Signature::try_from(sig).map_err(|_| CryptError::Bad)?;
    Ok(verifier.verify(material, &signature).is_ok())
}

/// Signs `material` using SM2 with `profile.user_id`.
///
/// # Errors
/// Returns [`CryptError::Bad`] if SM2 keys or user ID are missing/invalid.
pub fn sm2_sign(profile: &Profile, material: &[u8]) -> Result<Vec<u8>, CryptError> {
    let ctx = libsm::sm2::signature::SigCtx::new();
    let sk = ctx
        .load_seckey(profile.sm2_private.as_deref().ok_or(CryptError::Bad)?)
        .map_err(|_| CryptError::Bad)?;
    let pk = ctx
        .load_pubkey(profile.sm2_public.as_deref().ok_or(CryptError::Bad)?)
        .map_err(|_| CryptError::Bad)?;
    let user = profile.user_id.as_deref().ok_or(CryptError::Bad)?;
    let digest = ctx.hash(user, &pk, material).map_err(|_| CryptError::Bad)?;
    let sig = ctx.sign_raw(&digest, &sk).map_err(|_| CryptError::Bad)?;
    Ok(sig.der_encode())
}

/// Verifies an SM2 signature over `material`.
///
/// # Errors
/// Returns [`CryptError::Bad`] if SM2 public key or DER signature cannot be decoded.
pub fn sm2_verify(profile: &Profile, material: &[u8], sig: &[u8]) -> Result<bool, CryptError> {
    let ctx = libsm::sm2::signature::SigCtx::new();
    let pk = ctx
        .load_pubkey(profile.sm2_public.as_deref().ok_or(CryptError::Bad)?)
        .map_err(|_| CryptError::Bad)?;
    let parsed = libsm::sm2::signature::Signature::der_decode(sig).map_err(|_| CryptError::Bad)?;
    let user = profile.user_id.as_deref().ok_or(CryptError::Bad)?;
    let digest = ctx.hash(user, &pk, material).map_err(|_| CryptError::Bad)?;
    ctx.verify_raw(&digest, &pk, &parsed)
        .map_err(|_| CryptError::Bad)
}

/// Encrypts `plain` with SM2 public key in configured `C1C3C2` / `C1C2C3` layout.
///
/// # Errors
/// Returns [`CryptError::Bad`] if SM2 public key is missing or encryption fails.
pub fn sm2_encrypt(profile: &Profile, plain: &[u8]) -> Result<Vec<u8>, CryptError> {
    let ctx = libsm::sm2::signature::SigCtx::new();
    let pk = ctx
        .load_pubkey(profile.sm2_public.as_deref().ok_or(CryptError::Bad)?)
        .map_err(|_| CryptError::Bad)?;
    let enc = libsm::sm2::encrypt::EncryptCtx::new(plain.len(), pk);
    let raw = enc.encrypt(plain).map_err(|_| CryptError::Bad)?;
    repack_sm2(&raw, plain.len(), profile.sm2_mode, true)
}

/// Decrypts SM2 `cipher` bytes.
///
/// # Errors
/// Returns [`CryptError::Bad`] if ciphertext is shorter than 97 bytes or decryption fails.
pub fn sm2_decrypt(profile: &Profile, cipher: &[u8]) -> Result<Vec<u8>, CryptError> {
    if cipher.len() < 97 {
        return Err(CryptError::Bad);
    }
    let klen = cipher.len() - 97;
    let raw = repack_sm2(cipher, klen, profile.sm2_mode, false)?;
    let ctx = libsm::sm2::signature::SigCtx::new();
    let sk = ctx
        .load_seckey(profile.sm2_private.as_deref().ok_or(CryptError::Bad)?)
        .map_err(|_| CryptError::Bad)?;
    let dec = libsm::sm2::encrypt::DecryptCtx::new(klen, sk);
    dec.decrypt(&raw).map_err(|_| CryptError::Bad)
}

fn repack_sm2(
    data: &[u8],
    klen: usize,
    mode: Sm2Mode,
    to_profile: bool,
) -> Result<Vec<u8>, CryptError> {
    let c1 = data.get(..65).ok_or(CryptError::Bad)?;
    let rest = data.get(65..).ok_or(CryptError::Bad)?;
    match mode {
        Sm2Mode::C1C2C3 => Ok(data.to_vec()),
        Sm2Mode::C1C3C2 if to_profile => {
            let c2 = rest.get(..klen).ok_or(CryptError::Bad)?;
            let c3 = rest.get(klen..).ok_or(CryptError::Bad)?;
            let mut out = c1.to_vec();
            out.extend_from_slice(c3);
            out.extend_from_slice(c2);
            Ok(out)
        }
        Sm2Mode::C1C3C2 => {
            let c3 = rest.get(..32).ok_or(CryptError::Bad)?;
            let c2 = rest.get(32..).ok_or(CryptError::Bad)?;
            let mut out = c1.to_vec();
            out.extend_from_slice(c2);
            out.extend_from_slice(c3);
            Ok(out)
        }
    }
}

/// Unwraps a session symmetric key using `profile.wrap`.
///
/// # Errors
/// Returns [`CryptError::Bad`] if unwrapping fails.
pub fn unwrap_key(profile: &Profile, wrapped: &[u8]) -> Result<Vec<u8>, CryptError> {
    match profile.wrap.ok_or(CryptError::Bad)? {
        WrapAlg::RsaPkcs1Public => rsa_private_decrypt(profile, wrapped),
        WrapAlg::Sm2Encrypt => sm2_decrypt(profile, wrapped),
    }
}

/// Wraps a session symmetric key using `profile.wrap`.
///
/// # Errors
/// Returns [`CryptError::Bad`] if wrapping fails.
pub fn wrap_key(profile: &Profile, key: &[u8]) -> Result<Vec<u8>, CryptError> {
    match profile.wrap.ok_or(CryptError::Bad)? {
        WrapAlg::RsaPkcs1Public => rsa_public_encrypt(profile, key),
        WrapAlg::Sm2Encrypt => sm2_encrypt(profile, key),
    }
}

/// Generates compliant random secret record(s) for WebUI one-click key creation.
///
/// Supported modes: `"bytes_8"`, `"bytes_16"`, `"bytes_24"`, `"bytes_32"`, `"text_24"`, `"rsa_2048"`, `"sm2"`.
///
/// # Errors
/// Returns an error string when `mode` is unknown or key generation fails.
pub fn generate_secrets(
    name: &str,
    mode: &str,
    now_rfc3339: &str,
) -> Result<Vec<EnvelopeSecretRecord>, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err("密钥名称长度须在 1 到 64 字符之间".to_string());
    }
    match mode {
        "bytes_8" => Ok(vec![random_bytes_record(name, 8, now_rfc3339)]),
        "bytes_16" => Ok(vec![random_bytes_record(name, 16, now_rfc3339)]),
        "bytes_24" => Ok(vec![random_bytes_record(name, 24, now_rfc3339)]),
        "bytes_32" => Ok(vec![random_bytes_record(name, 32, now_rfc3339)]),
        "text_24" => {
            let mut raw = [0u8; 18];
            rand::fill(&mut raw);
            let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
            Ok(vec![make_record(
                name.to_string(),
                SecretKind::Text,
                text.into_bytes(),
                None,
                now_rfc3339,
            )])
        }
        "rsa_2048" => {
            let sk = RsaPrivateKey::new(&mut rand::rngs::SysRng, 2048)
                .map_err(|e| format!("RSA 密钥生成失败: {e}"))?;
            let pk = RsaPublicKey::from(&sk);
            let sk_pem = sk
                .to_pkcs8_pem(LineEnding::LF)
                .map_err(|e| format!("RSA 私钥编码失败: {e}"))?
                .to_string();
            let pk_pem = pk
                .to_public_key_pem(LineEnding::LF)
                .map_err(|e| format!("RSA 公钥编码失败: {e}"))?;
            Ok(vec![
                make_record(
                    format!("{name}_private"),
                    SecretKind::Pem,
                    sk_pem.into_bytes(),
                    Some(pk_pem.clone()),
                    now_rfc3339,
                ),
                make_record(
                    format!("{name}_public"),
                    SecretKind::Pem,
                    pk_pem.clone().into_bytes(),
                    Some(pk_pem),
                    now_rfc3339,
                ),
            ])
        }
        "sm2" => {
            let ctx = libsm::sm2::signature::SigCtx::new();
            let (pk, sk) = ctx
                .new_keypair()
                .map_err(|_| "SM2 密钥生成失败".to_string())?;
            let sk_bytes = ctx
                .serialize_seckey(&sk)
                .map_err(|_| "SM2 私钥序列化失败".to_string())?;
            let pk_bytes = ctx
                .serialize_pubkey(&pk, false)
                .map_err(|_| "SM2 公钥序列化失败".to_string())?;
            let pk_hex = hex::encode(&pk_bytes);
            Ok(vec![
                make_record(
                    format!("{name}_private"),
                    SecretKind::BytesHex,
                    sk_bytes,
                    Some(pk_hex.clone()),
                    now_rfc3339,
                ),
                make_record(
                    format!("{name}_public"),
                    SecretKind::BytesHex,
                    pk_bytes,
                    Some(pk_hex),
                    now_rfc3339,
                ),
            ])
        }
        other => Err(format!("不支持的密钥生成模式: {other}")),
    }
}

fn random_bytes_record(name: &str, len: usize, now_rfc3339: &str) -> EnvelopeSecretRecord {
    let mut buf = vec![0u8; len];
    rand::fill(&mut buf[..]);
    make_record(
        name.to_string(),
        SecretKind::BytesHex,
        buf,
        None,
        now_rfc3339,
    )
}

/// Builds a fingerprinted [`EnvelopeSecretRecord`].
#[must_use]
pub fn make_record(
    name: String,
    kind: SecretKind,
    raw_bytes: Vec<u8>,
    public_material: Option<String>,
    now_rfc3339: &str,
) -> EnvelopeSecretRecord {
    let digest = hex::encode(Sha256::digest(&raw_bytes));
    let fingerprint = digest.get(..16).unwrap_or(&digest).to_string();
    EnvelopeSecretRecord {
        name,
        kind,
        raw_bytes,
        fingerprint,
        public_material,
        updated_at: now_rfc3339.to_string(),
    }
}
