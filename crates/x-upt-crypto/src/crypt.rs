//! Envelope symmetric ciphers (AES, DES, SM4) and digests/MACs (MD5, HMAC-SHA256, SM3, HMAC-SM3).

use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use aes_gcm::aead::{Aead, Payload};
use base64::Engine;
use hmac::{Hmac, KeyInit as HmacKeyInit, Mac};
use md5::{Digest, Md5};
use sha2::Sha256;
use x_upt_core::domain::secrets_equal_bytes;

use crate::crypt_asym::{
    rsa_private_decrypt, rsa_private_encrypt, rsa_public_decrypt, rsa_public_encrypt, rsa_sign,
    rsa_verify, sm2_decrypt, sm2_encrypt, sm2_sign, sm2_verify,
};
use crate::profile::{CryptAlg, Encoding, HexCase, IvFrom, Profile, SignAlg};

type HmacSha256 = Hmac<Sha256>;

/// Cryptographic operation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptError {
    /// Invalid key, IV, padding, or ciphertext bytes.
    Bad,
    /// Signature verification mismatch.
    Mismatch,
}

impl std::fmt::Display for CryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bad => f.write_str("invalid cryptographic parameters or ciphertext"),
            Self::Mismatch => f.write_str("signature mismatch"),
        }
    }
}

impl std::error::Error for CryptError {}

/// Signs `material` using `profile.sign_alg` and formats according to `profile.sign_encoding`.
///
/// # Errors
/// Returns [`CryptError::Bad`] if signing algorithm or key material is missing/invalid.
pub fn sign(profile: &Profile, material: &[u8]) -> Result<String, CryptError> {
    let alg = profile.sign_alg.ok_or(CryptError::Bad)?;
    let raw = sign_raw(profile, alg, material)?;
    Ok(encode_bytes(
        raw.as_slice(),
        profile.sign_encoding,
        profile.hex_case,
    ))
}

/// Verifies `provided` signature against `material`.
///
/// # Errors
/// Returns [`CryptError`] on malformed signature encoding or missing algorithm/keys.
pub fn verify(profile: &Profile, material: &[u8], provided: &str) -> Result<bool, CryptError> {
    let alg = profile.sign_alg.ok_or(CryptError::Bad)?;
    if matches!(alg, SignAlg::RsaSha256 | SignAlg::Sm2Sign) {
        let raw = decode_sig(profile, provided).ok_or(CryptError::Mismatch)?;
        return if alg == SignAlg::RsaSha256 {
            rsa_verify(profile, material, &raw)
        } else {
            sm2_verify(profile, material, &raw)
        };
    }
    let raw = sign_raw(profile, alg, material)?;
    if profile.sign_encoding == Encoding::Hex {
        let expected = hex_encode(&raw, profile.hex_case);
        return Ok(secrets_equal_bytes(
            expected.as_bytes(),
            provided.as_bytes(),
        ));
    }
    let left = decode_sig(profile, provided).ok_or(CryptError::Mismatch)?;
    Ok(secrets_equal_bytes(&raw, &left))
}

/// Encrypts `plain` using `profile` response/request cipher and attaches `RandomPrefix` IV if configured.
///
/// # Errors
/// Returns [`CryptError::Bad`] on encryption failure.
pub fn encrypt(profile: &Profile, key: &[u8], plain: &[u8]) -> Result<Vec<u8>, CryptError> {
    let alg = profile
        .response_crypt
        .or(profile.crypt_alg)
        .ok_or(CryptError::Bad)?;
    let raw = encrypt_alg(profile, alg, key, plain)?;
    Ok(attach_iv(profile, alg, &raw))
}

/// Splits `RandomPrefix` IV if configured and decrypts `encoded_body`.
///
/// # Errors
/// Returns [`CryptError::Bad`] on malformed IV, padding, or authentication tag.
pub fn decrypt(profile: &Profile, key: &[u8], encoded_body: &[u8]) -> Result<Vec<u8>, CryptError> {
    let alg = profile.crypt_alg.ok_or(CryptError::Bad)?;
    let (iv, body) = split_iv(profile, alg, encoded_body)?;
    let mut owned = profile.clone();
    if let Some(iv) = iv {
        owned.fixed_iv = Some(iv);
    }
    decrypt_alg(&owned, alg, key, &body)
}

/// Encrypts `plain` with explicit `alg` (without outer IV prefix framing).
///
/// # Errors
/// Returns [`CryptError::Bad`] if key/IV/input constraints are violated.
pub fn encrypt_alg(
    profile: &Profile,
    alg: CryptAlg,
    key: &[u8],
    plain: &[u8],
) -> Result<Vec<u8>, CryptError> {
    match alg {
        CryptAlg::AesEcbPkcs5 => aes_ecb(key, &pkcs7_pad(plain, 16), true),
        CryptAlg::AesCfb => aes_cfb(profile, key, plain, true),
        CryptAlg::AesGcm => aes_gcm(profile, key, plain, true),
        CryptAlg::DesCbcPkcs5 => des_cbc(profile, key, &pkcs7_pad(plain, 8), true),
        CryptAlg::Sm4EcbPkcs7 => sm4_ecb(key, &pkcs7_pad(plain, 16), true),
        CryptAlg::Sm4CbcPkcs7 => sm4_cbc(profile, key, plain, true),
        CryptAlg::Sm4EcbNoPadding => {
            if !plain.len().is_multiple_of(16) {
                return Err(CryptError::Bad);
            }
            sm4_ecb(key, plain, true)
        }
        CryptAlg::RsaPkcs1Public => rsa_public_encrypt(profile, plain),
        CryptAlg::RsaPkcs1Private => rsa_private_encrypt(profile, plain),
        CryptAlg::Sm2Encrypt => sm2_encrypt(profile, plain),
    }
}

/// Decrypts `cipher` with explicit `alg`.
///
/// # Errors
/// Returns [`CryptError::Bad`] if decryption or unpadding fails.
pub fn decrypt_alg(
    profile: &Profile,
    alg: CryptAlg,
    key: &[u8],
    cipher: &[u8],
) -> Result<Vec<u8>, CryptError> {
    match alg {
        CryptAlg::AesEcbPkcs5 => pkcs7_unpad(&aes_ecb(key, cipher, false)?, 16),
        CryptAlg::AesCfb => aes_cfb(profile, key, cipher, false),
        CryptAlg::AesGcm => aes_gcm(profile, key, cipher, false),
        CryptAlg::DesCbcPkcs5 => pkcs7_unpad(&des_cbc(profile, key, cipher, false)?, 8),
        CryptAlg::Sm4EcbPkcs7 => pkcs7_unpad(&sm4_ecb(key, cipher, false)?, 16),
        CryptAlg::Sm4CbcPkcs7 => sm4_cbc(profile, key, cipher, false),
        CryptAlg::Sm4EcbNoPadding => {
            if !cipher.len().is_multiple_of(16) {
                return Err(CryptError::Bad);
            }
            sm4_ecb(key, cipher, false)
        }
        CryptAlg::RsaPkcs1Public => rsa_private_decrypt(profile, cipher),
        CryptAlg::RsaPkcs1Private => rsa_public_decrypt(profile, cipher),
        CryptAlg::Sm2Encrypt => sm2_decrypt(profile, cipher),
    }
}

/// Prepares outbound IV bytes according to `profile.iv_from`.
///
/// # Errors
/// Returns [`CryptError::Bad`] if IV configuration or key length is invalid.
pub fn prepare_iv(profile: &Profile, alg: CryptAlg) -> Result<Vec<u8>, CryptError> {
    let len = iv_len(profile, alg)?;
    match profile.iv_from {
        Some(IvFrom::Key) => {
            let key = profile.key.as_deref().ok_or(CryptError::Bad)?;
            Ok(key.get(..len).ok_or(CryptError::Bad)?.to_vec())
        }
        Some(IvFrom::RandomPrefix | IvFrom::Field) => {
            if let Some(fixed) = &profile.fixed_iv {
                if fixed.len() != len {
                    return Err(CryptError::Bad);
                }
                return Ok(fixed.clone());
            }
            let mut iv = vec![0u8; len];
            rand::fill(&mut iv[..]);
            Ok(iv)
        }
        None => Err(CryptError::Bad),
    }
}

/// Encodes ciphertext bytes into hex or Base64 according to `profile`.
#[must_use]
pub fn encode_crypt(profile: &Profile, raw: &[u8]) -> String {
    encode_bytes(raw, profile.crypt_encoding, profile.hex_case)
}

/// Decodes ciphertext text from hex (enforcing case) or Base64.
///
/// # Errors
/// Returns [`CryptError::Bad`] if `text` is invalid hex/Base64 or violates `hex_case`.
pub fn decode_crypt(profile: &Profile, text: &str) -> Result<Vec<u8>, CryptError> {
    match profile.crypt_encoding {
        Encoding::Hex => {
            let bytes = hex::decode(text).map_err(|_| CryptError::Bad)?;
            let canonical = hex_encode(&bytes, profile.hex_case);
            if !secrets_equal_bytes(canonical.as_bytes(), text.as_bytes()) {
                return Err(CryptError::Bad);
            }
            Ok(bytes)
        }
        Encoding::Base64 => base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|_| CryptError::Bad),
    }
}

fn attach_iv(profile: &Profile, alg: CryptAlg, cipher: &[u8]) -> Vec<u8> {
    if profile.iv_from != Some(IvFrom::RandomPrefix)
        || matches!(
            alg,
            CryptAlg::AesEcbPkcs5 | CryptAlg::Sm4EcbPkcs7 | CryptAlg::Sm4EcbNoPadding
        )
    {
        return cipher.to_vec();
    }
    let Some(iv) = profile.fixed_iv.as_deref() else {
        return cipher.to_vec();
    };
    let mut out = iv.to_vec();
    out.extend_from_slice(cipher);
    out
}

fn split_iv(
    profile: &Profile,
    alg: CryptAlg,
    data: &[u8],
) -> Result<(Option<Vec<u8>>, Vec<u8>), CryptError> {
    if profile.iv_from != Some(IvFrom::RandomPrefix)
        || matches!(
            alg,
            CryptAlg::AesEcbPkcs5 | CryptAlg::Sm4EcbPkcs7 | CryptAlg::Sm4EcbNoPadding
        )
    {
        return Ok((None, data.to_vec()));
    }
    let len = iv_len(profile, alg)?;
    let iv = data.get(..len).ok_or(CryptError::Bad)?.to_vec();
    let rest = data.get(len..).ok_or(CryptError::Bad)?.to_vec();
    Ok((Some(iv), rest))
}

fn iv_len(profile: &Profile, alg: CryptAlg) -> Result<usize, CryptError> {
    Ok(match alg {
        CryptAlg::AesGcm => profile.iv_len.ok_or(CryptError::Bad)?,
        CryptAlg::DesCbcPkcs5 => 8,
        CryptAlg::AesCfb | CryptAlg::Sm4CbcPkcs7 => 16,
        _ => 0,
    })
}

fn sign_raw(profile: &Profile, alg: SignAlg, material: &[u8]) -> Result<Vec<u8>, CryptError> {
    match alg {
        SignAlg::Md5 => Ok(Md5::digest(material).to_vec()),
        SignAlg::HmacSha256 => {
            let key = profile.sign_key.as_deref().ok_or(CryptError::Bad)?;
            let mut mac =
                <HmacSha256 as HmacKeyInit>::new_from_slice(key).map_err(|_| CryptError::Bad)?;
            mac.update(material);
            Ok(mac.finalize().into_bytes().to_vec())
        }
        SignAlg::Sm3 => Ok(sm3(material).to_vec()),
        SignAlg::HmacSm3 => {
            let key = profile.sign_key.as_deref().ok_or(CryptError::Bad)?;
            Ok(hmac_sm3(key, material).to_vec())
        }
        SignAlg::RsaSha256 => rsa_sign(profile, material),
        SignAlg::Sm2Sign => sm2_sign(profile, material),
    }
}

fn encode_bytes(raw: &[u8], enc: Encoding, case: HexCase) -> String {
    match enc {
        Encoding::Hex => hex_encode(raw, case),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD.encode(raw),
    }
}

fn decode_sig(profile: &Profile, text: &str) -> Option<Vec<u8>> {
    match profile.sign_encoding {
        Encoding::Hex => hex::decode(text).ok(),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD.decode(text).ok(),
    }
}

fn hex_encode(raw: &[u8], case: HexCase) -> String {
    let text = hex::encode(raw);
    match case {
        HexCase::Lower => text,
        HexCase::Upper => text.to_ascii_uppercase(),
    }
}

fn aes_ecb(key: &[u8], data: &[u8], encrypting: bool) -> Result<Vec<u8>, CryptError> {
    if !data.len().is_multiple_of(16) {
        return Err(CryptError::Bad);
    }
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);
        out.extend_from_slice(&aes_block(key, &block, encrypting)?);
    }
    Ok(out)
}

fn aes_block(key: &[u8], block: &[u8; 16], encrypting: bool) -> Result<[u8; 16], CryptError> {
    let mut out = *block;
    match key.len() {
        16 => apply_aes::<aes::Aes128>(key, &mut out, encrypting)?,
        24 => apply_aes::<aes::Aes192>(key, &mut out, encrypting)?,
        32 => apply_aes::<aes::Aes256>(key, &mut out, encrypting)?,
        _ => return Err(CryptError::Bad),
    }
    Ok(out)
}

fn apply_aes<C>(key: &[u8], block: &mut [u8; 16], encrypting: bool) -> Result<(), CryptError>
where
    C: KeyInit + BlockCipherEncrypt + BlockCipherDecrypt,
{
    let cipher = C::new_from_slice(key).map_err(|_| CryptError::Bad)?;
    let mut arr = cipher::Block::<C>::default();
    arr.copy_from_slice(block);
    if encrypting {
        cipher.encrypt_block(&mut arr);
    } else {
        cipher.decrypt_block(&mut arr);
    }
    block.copy_from_slice(&arr);
    Ok(())
}

fn aes_cfb(
    profile: &Profile,
    key: &[u8],
    data: &[u8],
    encrypting: bool,
) -> Result<Vec<u8>, CryptError> {
    let segment = profile.segment_bits.ok_or(CryptError::Bad)?;
    if segment == 0 || segment > 128 || segment % 8 != 0 {
        return Err(CryptError::Bad);
    }
    let padded = if encrypting && profile.cipher_flags.padding_pkcs5 {
        pkcs7_pad(data, 16)
    } else {
        data.to_vec()
    };
    let iv = profile.fixed_iv.as_deref().ok_or(CryptError::Bad)?;
    let mut iv_bytes: [u8; 16] = iv.try_into().map_err(|_| CryptError::Bad)?;
    let seg = segment / 8;
    let mut out = Vec::with_capacity(padded.len());
    let mut offset = 0usize;
    while offset < padded.len() {
        let stream = aes_block(key, &iv_bytes, true)?;
        let take = seg.min(padded.len() - offset);
        let chunk = padded.get(offset..offset + take).ok_or(CryptError::Bad)?;
        let produced: Vec<u8> = chunk
            .iter()
            .zip(stream.iter())
            .map(|(&p, &s)| p ^ s)
            .collect();
        out.extend_from_slice(&produced);
        let feedback = if encrypting {
            produced.as_slice()
        } else {
            chunk
        };
        shift_iv(&mut iv_bytes, feedback);
        offset += take;
    }
    if !encrypting && profile.cipher_flags.padding_pkcs5 {
        pkcs7_unpad(&out, 16)
    } else {
        Ok(out)
    }
}

fn shift_iv(iv: &mut [u8; 16], feedback: &[u8]) {
    if let Some(tail) = feedback
        .len()
        .checked_sub(16)
        .and_then(|s| feedback.get(s..))
    {
        iv.copy_from_slice(tail);
        return;
    }
    iv.copy_within(feedback.len().., 0);
    let start = 16 - feedback.len();
    if let Some(dst) = iv.get_mut(start..) {
        dst.copy_from_slice(feedback);
    }
}

fn aes_gcm(
    profile: &Profile,
    key: &[u8],
    data: &[u8],
    encrypting: bool,
) -> Result<Vec<u8>, CryptError> {
    let iv = profile.fixed_iv.as_deref().ok_or(CryptError::Bad)?;
    if profile.tag_bits != Some(128) {
        return Err(CryptError::Bad);
    }
    match (key.len(), iv.len()) {
        (16, 12) => gcm_typed::<aes_gcm::Aes128Gcm>(key, iv, data, encrypting),
        (24, 12) => gcm_typed::<aes_gcm::AesGcm<aes::Aes192, aes_gcm::aead::consts::U12>>(
            key, iv, data, encrypting,
        ),
        (32, 12) => gcm_typed::<aes_gcm::Aes256Gcm>(key, iv, data, encrypting),
        (16, 16) => gcm_typed::<aes_gcm::AesGcm<aes::Aes128, aes_gcm::aead::consts::U16>>(
            key, iv, data, encrypting,
        ),
        (24, 16) => gcm_typed::<aes_gcm::AesGcm<aes::Aes192, aes_gcm::aead::consts::U16>>(
            key, iv, data, encrypting,
        ),
        (32, 16) => gcm_typed::<aes_gcm::AesGcm<aes::Aes256, aes_gcm::aead::consts::U16>>(
            key, iv, data, encrypting,
        ),
        _ => Err(CryptError::Bad),
    }
}

fn gcm_typed<C: Aead + KeyInit>(
    key: &[u8],
    iv: &[u8],
    data: &[u8],
    encrypting: bool,
) -> Result<Vec<u8>, CryptError> {
    let cipher = C::new_from_slice(key).map_err(|_| CryptError::Bad)?;
    let mut nonce = aes_gcm::Nonce::<<C as aes_gcm::aead::AeadCore>::NonceSize>::default();
    nonce.copy_from_slice(iv);
    let payload = Payload {
        msg: data,
        aad: b"",
    };
    if encrypting {
        cipher.encrypt(&nonce, payload).map_err(|_| CryptError::Bad)
    } else {
        cipher.decrypt(&nonce, payload).map_err(|_| CryptError::Bad)
    }
}

fn des_cbc(
    profile: &Profile,
    key: &[u8],
    data: &[u8],
    encrypting: bool,
) -> Result<Vec<u8>, CryptError> {
    if key.len() != 8 || !data.len().is_multiple_of(8) {
        return Err(CryptError::Bad);
    }
    let iv = profile.fixed_iv.as_deref().ok_or(CryptError::Bad)?;
    let mut prev: [u8; 8] = iv.try_into().map_err(|_| CryptError::Bad)?;
    let cipher = des::Des::new_from_slice(key).map_err(|_| CryptError::Bad)?;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(8) {
        let mut block: [u8; 8] = chunk.try_into().map_err(|_| CryptError::Bad)?;
        if encrypting {
            for (b, p) in block.iter_mut().zip(prev.iter()) {
                *b ^= *p;
            }
            let mut arr = cipher::Block::<des::Des>::default();
            arr.copy_from_slice(&block);
            cipher.encrypt_block(&mut arr);
            block.copy_from_slice(&arr);
            prev = block;
            out.extend_from_slice(&block);
        } else {
            let mut arr = cipher::Block::<des::Des>::default();
            arr.copy_from_slice(&block);
            cipher.decrypt_block(&mut arr);
            let mut plain = [0u8; 8];
            plain.copy_from_slice(&arr);
            for (pl, pr) in plain.iter_mut().zip(prev.iter()) {
                *pl ^= *pr;
            }
            prev = block;
            out.extend_from_slice(&plain);
        }
    }
    Ok(out)
}

fn sm4_ecb(key: &[u8], data: &[u8], encrypting: bool) -> Result<Vec<u8>, CryptError> {
    if key.len() != 16 || !data.len().is_multiple_of(16) {
        return Err(CryptError::Bad);
    }
    let cipher = libsm::sm4::cipher::Sm4Cipher::new(key).map_err(|_| CryptError::Bad)?;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(16) {
        let block = if encrypting {
            cipher.encrypt(chunk).map_err(|_| CryptError::Bad)?
        } else {
            cipher.decrypt(chunk).map_err(|_| CryptError::Bad)?
        };
        out.extend_from_slice(&block);
    }
    Ok(out)
}

fn sm4_cbc(
    profile: &Profile,
    key: &[u8],
    data: &[u8],
    encrypting: bool,
) -> Result<Vec<u8>, CryptError> {
    let iv = profile.fixed_iv.as_deref().ok_or(CryptError::Bad)?;
    if key.len() != 16 || iv.len() != 16 {
        return Err(CryptError::Bad);
    }
    let mode =
        libsm::sm4::cipher_mode::Sm4CipherMode::new(key, libsm::sm4::cipher_mode::CipherMode::Cbc)
            .map_err(|_| CryptError::Bad)?;
    if encrypting {
        mode.encrypt(b"", data, iv).map_err(|_| CryptError::Bad)
    } else {
        mode.decrypt(b"", data, iv).map_err(|_| CryptError::Bad)
    }
}

fn sm3(data: &[u8]) -> [u8; 32] {
    let mut hasher = libsm::sm3::hash::Sm3Hash::new(data);
    hasher.get_hash()
}

fn hmac_sm3(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut keyed = if key.len() > BLOCK {
        sm3(key).to_vec()
    } else {
        key.to_vec()
    };
    keyed.resize(BLOCK, 0);
    let mut ipad: Vec<u8> = keyed.iter().map(|&k| k ^ 0x36).collect();
    let mut opad: Vec<u8> = keyed.iter().map(|&k| k ^ 0x5c).collect();
    ipad.extend_from_slice(data);
    let inner = sm3(&ipad);
    opad.extend_from_slice(&inner);
    sm3(&opad)
}

fn pkcs7_pad(data: &[u8], block: usize) -> Vec<u8> {
    let n = block - (data.len() % block);
    let mut out = data.to_vec();
    out.extend(std::iter::repeat_n(u8::try_from(n).unwrap_or(u8::MAX), n));
    out
}

fn pkcs7_unpad(data: &[u8], block: usize) -> Result<Vec<u8>, CryptError> {
    if data.is_empty() || !data.len().is_multiple_of(block) {
        return Err(CryptError::Bad);
    }
    let n = usize::from(*data.last().ok_or(CryptError::Bad)?);
    if n == 0 || n > block || data.len() < n {
        return Err(CryptError::Bad);
    }
    let pad = data.get(data.len() - n..).ok_or(CryptError::Bad)?;
    if pad.iter().any(|&b| usize::from(b) != n) {
        return Err(CryptError::Bad);
    }
    Ok(data.get(..data.len() - n).unwrap_or(&[]).to_vec())
}
