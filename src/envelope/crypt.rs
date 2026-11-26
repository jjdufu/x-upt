use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use aes_gcm::aead::{Aead, Payload};
use base64::Engine;
use hmac::{Hmac, KeyInit as HmacKeyInit, Mac};
use md5::{Digest, Md5};
use rsa::hazmat::{rsa_decrypt, rsa_encrypt};
use rsa::pkcs8::DecodePrivateKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::traits::PublicKeyParts;
use rsa::{BoxedUint, Pkcs1v15Encrypt, PrivateKey as RsaPrivateKey, PublicKey as RsaPublicKey};
use sha2::Sha256;

use crate::envelope::profile::{
    CryptAlg, Encoding, HexCase, IvFrom, Profile, SignAlg, Sm2Mode, WrapAlg,
};
use crate::service::apps::secrets_equal_bytes;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub enum CryptError {
    Bad,
    Mismatch,
}

pub fn sign(profile: &Profile, material: &[u8]) -> Result<String, CryptError> {
    let alg = profile.sign_alg.ok_or(CryptError::Bad)?;
    let raw = sign_raw(profile, alg, material)?;
    Ok(encode_sign(profile, &raw))
}

pub fn verify(profile: &Profile, material: &[u8], provided: &str) -> Result<bool, CryptError> {
    let alg = profile.sign_alg.ok_or(CryptError::Bad)?;
    if matches!(alg, SignAlg::RsaSha256 | SignAlg::Sm2Sign) {
        let raw = decode_sign(profile, provided).ok_or(CryptError::Mismatch)?;
        return if alg == SignAlg::RsaSha256 {
            rsa_verify(profile, material, &raw)
        } else {
            sm2_verify(profile, material, &raw)
        };
    }
    let raw = sign_raw(profile, alg, material)?;
    let expected = encode_sign(profile, &raw);
    if profile.sign_encoding == Encoding::Hex {
        return Ok(secrets_equal_bytes(
            expected.as_bytes(),
            provided.as_bytes(),
        ));
    }
    let left = decode_sign(profile, provided).ok_or(CryptError::Mismatch)?;
    Ok(secrets_equal_bytes(&raw, &left))
}

pub fn encrypt(profile: &Profile, key: &[u8], plain: &[u8]) -> Result<Vec<u8>, CryptError> {
    seal_symmetric(profile, key, plain, profile.fixed_iv.as_deref())
}

pub fn decrypt(profile: &Profile, key: &[u8], cipher: &[u8]) -> Result<Vec<u8>, CryptError> {
    open_symmetric(profile, key, cipher)
}

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

pub fn decrypt_alg(
    profile: &Profile,
    alg: CryptAlg,
    key: &[u8],
    cipher: &[u8],
) -> Result<Vec<u8>, CryptError> {
    match alg {
        CryptAlg::AesEcbPkcs5 => {
            let raw = aes_ecb(key, cipher, false)?;
            pkcs7_unpad(&raw, 16)
        }
        CryptAlg::AesCfb => aes_cfb(profile, key, cipher, false),
        CryptAlg::AesGcm => aes_gcm(profile, key, cipher, false),
        CryptAlg::DesCbcPkcs5 => {
            let raw = des_cbc(profile, key, cipher, false)?;
            pkcs7_unpad(&raw, 8)
        }
        CryptAlg::Sm4EcbPkcs7 => {
            let raw = sm4_ecb(key, cipher, false)?;
            pkcs7_unpad(&raw, 16)
        }
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

pub fn seal_symmetric(
    profile: &Profile,
    key: &[u8],
    plain: &[u8],
    iv: Option<&[u8]>,
) -> Result<Vec<u8>, CryptError> {
    let alg = profile
        .response_crypt
        .or(profile.crypt_alg)
        .ok_or(CryptError::Bad)?;
    let mut owned = profile.clone();
    if let Some(iv) = iv {
        owned.fixed_iv = Some(iv.to_vec());
    }
    let raw = encrypt_alg(&owned, alg, key, plain)?;
    Ok(attach_iv(&owned, alg, &raw))
}

pub fn open_symmetric(
    profile: &Profile,
    key: &[u8],
    encoded_body: &[u8],
) -> Result<Vec<u8>, CryptError> {
    let alg = profile.crypt_alg.ok_or(CryptError::Bad)?;
    let (iv, body) = split_iv(profile, alg, encoded_body)?;
    let mut owned = profile.clone();
    if let Some(iv) = iv {
        owned.fixed_iv = Some(iv);
    }
    decrypt_alg(&owned, alg, key, &body)
}

fn attach_iv(profile: &Profile, alg: CryptAlg, cipher: &[u8]) -> Vec<u8> {
    if profile.iv_from != Some(IvFrom::RandomPrefix) {
        return cipher.to_vec();
    }
    let Some(iv) = profile.fixed_iv.as_deref() else {
        return cipher.to_vec();
    };
    if matches!(
        alg,
        CryptAlg::AesEcbPkcs5 | CryptAlg::Sm4EcbPkcs7 | CryptAlg::Sm4EcbNoPadding
    ) {
        return cipher.to_vec();
    }
    let mut out = iv.to_vec();
    out.extend_from_slice(cipher);
    out
}

fn split_iv(
    profile: &Profile,
    alg: CryptAlg,
    data: &[u8],
) -> Result<(Option<Vec<u8>>, Vec<u8>), CryptError> {
    if profile.iv_from != Some(IvFrom::RandomPrefix) {
        return Ok((None, data.to_vec()));
    }
    if matches!(
        alg,
        CryptAlg::AesEcbPkcs5 | CryptAlg::Sm4EcbPkcs7 | CryptAlg::Sm4EcbNoPadding
    ) {
        return Ok((None, data.to_vec()));
    }
    let len = iv_len(profile, alg)?;
    if data.len() < len {
        return Err(CryptError::Bad);
    }
    Ok((Some(data[..len].to_vec()), data[len..].to_vec()))
}

pub fn prepare_iv(profile: &Profile, alg: CryptAlg) -> Result<Vec<u8>, CryptError> {
    let len = iv_len(profile, alg)?;
    match profile.iv_from {
        Some(IvFrom::Key) => {
            let key = profile.key.as_deref().ok_or(CryptError::Bad)?;
            if key.len() < len {
                return Err(CryptError::Bad);
            }
            Ok(key[..len].to_vec())
        }
        Some(IvFrom::RandomPrefix) | Some(IvFrom::Field) => {
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

fn encode_sign(profile: &Profile, raw: &[u8]) -> String {
    match profile.sign_encoding {
        Encoding::Hex => hex_encode(raw, profile.hex_case),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD.encode(raw),
    }
}

fn decode_sign(profile: &Profile, text: &str) -> Option<Vec<u8>> {
    match profile.sign_encoding {
        Encoding::Hex => hex::decode(text).ok(),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD.decode(text).ok(),
    }
}

pub fn encode_crypt(profile: &Profile, raw: &[u8]) -> String {
    match profile.crypt_encoding {
        Encoding::Hex => hex_encode(raw, profile.hex_case),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD.encode(raw),
    }
}

pub fn decode_crypt(profile: &Profile, text: &str) -> Result<Vec<u8>, CryptError> {
    match profile.crypt_encoding {
        Encoding::Hex => {
            let encoded = hex_encode(
                &hex::decode(text).map_err(|_| CryptError::Bad)?,
                profile.hex_case,
            );
            if !secrets_equal_bytes(encoded.as_bytes(), text.as_bytes()) {
                return Err(CryptError::Bad);
            }
            hex::decode(text).map_err(|_| CryptError::Bad)
        }
        Encoding::Base64 => base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(|_| CryptError::Bad),
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
        let next = aes_block(key, &block, encrypting)?;
        out.extend_from_slice(&next);
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
    let mut array = cipher::Block::<C>::default();
    array.copy_from_slice(block);
    if encrypting {
        cipher.encrypt_block(&mut array);
    } else {
        cipher.decrypt_block(&mut array);
    }
    block.copy_from_slice(&array);
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
    let padded = if encrypting && profile.padding_pkcs5 {
        pkcs7_pad(data, 16)
    } else {
        data.to_vec()
    };
    let iv = profile.fixed_iv.clone().ok_or(CryptError::Bad)?;
    if iv.len() != 16 {
        return Err(CryptError::Bad);
    }
    let mut iv_bytes = [0u8; 16];
    iv_bytes.copy_from_slice(&iv);
    let seg = segment / 8;
    let mut out = Vec::with_capacity(padded.len());
    let mut offset = 0;
    while offset < padded.len() {
        let stream = aes_block(key, &iv_bytes, true)?;
        let take = seg.min(padded.len() - offset);
        let mut produced = vec![0u8; take];
        for i in 0..take {
            produced[i] = padded[offset + i] ^ stream[i];
        }
        out.extend_from_slice(&produced);
        let feedback = if encrypting {
            &produced
        } else {
            &padded[offset..offset + take]
        };
        shift_iv(&mut iv_bytes, feedback);
        offset += take;
    }
    if encrypting {
        Ok(out)
    } else if profile.padding_pkcs5 {
        pkcs7_unpad(&out, 16)
    } else {
        Ok(out)
    }
}

fn shift_iv(iv: &mut [u8; 16], feedback: &[u8]) {
    if feedback.len() >= 16 {
        iv.copy_from_slice(&feedback[feedback.len() - 16..]);
        return;
    }
    iv.copy_within(feedback.len().., 0);
    let start = 16 - feedback.len();
    iv[start..].copy_from_slice(feedback);
}

fn aes_gcm(
    profile: &Profile,
    key: &[u8],
    data: &[u8],
    encrypting: bool,
) -> Result<Vec<u8>, CryptError> {
    let iv = profile.fixed_iv.as_deref().ok_or(CryptError::Bad)?;
    let tag_bits = profile.tag_bits.ok_or(CryptError::Bad)?;
    if tag_bits != 128 || iv.len() != 12 && iv.len() != 16 {
        return Err(CryptError::Bad);
    }
    gcm_crypt(key, iv, data, encrypting)
}

fn gcm_crypt(key: &[u8], iv: &[u8], data: &[u8], encrypting: bool) -> Result<Vec<u8>, CryptError> {
    match (key.len(), iv.len()) {
        (16, 12) => gcm_crypt_typed::<aes_gcm::Aes128Gcm>(key, iv, data, encrypting),
        (24, 12) => gcm_crypt_typed::<aes_gcm::AesGcm<aes::Aes192, aes_gcm::aead::consts::U12>>(
            key, iv, data, encrypting,
        ),
        (32, 12) => gcm_crypt_typed::<aes_gcm::Aes256Gcm>(key, iv, data, encrypting),
        (16, 16) => gcm_crypt_typed::<aes_gcm::AesGcm<aes::Aes128, aes_gcm::aead::consts::U16>>(
            key, iv, data, encrypting,
        ),
        (24, 16) => gcm_crypt_typed::<aes_gcm::AesGcm<aes::Aes192, aes_gcm::aead::consts::U16>>(
            key, iv, data, encrypting,
        ),
        (32, 16) => gcm_crypt_typed::<aes_gcm::AesGcm<aes::Aes256, aes_gcm::aead::consts::U16>>(
            key, iv, data, encrypting,
        ),
        _ => Err(CryptError::Bad),
    }
}

fn gcm_crypt_typed<C>(
    key: &[u8],
    iv: &[u8],
    data: &[u8],
    encrypting: bool,
) -> Result<Vec<u8>, CryptError>
where
    C: Aead + KeyInit,
{
    let cipher = C::new_from_slice(key).map_err(|_| CryptError::Bad)?;
    let mut nonce = aes_gcm::Nonce::<<C as aes_gcm::aead::AeadCore>::NonceSize>::default();
    nonce.copy_from_slice(iv);
    if encrypting {
        cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: data,
                    aad: b"",
                },
            )
            .map_err(|_| CryptError::Bad)
    } else {
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: data,
                    aad: b"",
                },
            )
            .map_err(|_| CryptError::Bad)
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
    if iv.len() != 8 {
        return Err(CryptError::Bad);
    }
    let cipher = des::Des::new_from_slice(key).map_err(|_| CryptError::Bad)?;
    let mut prev = [0u8; 8];
    prev.copy_from_slice(iv);
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(8) {
        let mut block = [0u8; 8];
        block.copy_from_slice(chunk);
        if encrypting {
            for i in 0..8 {
                block[i] ^= prev[i];
            }
            let mut array = cipher::Block::<des::Des>::default();
            array.copy_from_slice(&block);
            cipher.encrypt_block(&mut array);
            block.copy_from_slice(&array);
            prev = block;
            out.extend_from_slice(&block);
        } else {
            let mut plain = block;
            let mut array = cipher::Block::<des::Des>::default();
            array.copy_from_slice(&plain);
            cipher.decrypt_block(&mut array);
            plain.copy_from_slice(&array);
            for i in 0..8 {
                plain[i] ^= prev[i];
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
    if key.len() != 16 {
        return Err(CryptError::Bad);
    }
    let iv = profile.fixed_iv.as_deref().ok_or(CryptError::Bad)?;
    if iv.len() != 16 {
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

fn rsa_public_encrypt(profile: &Profile, plain: &[u8]) -> Result<Vec<u8>, CryptError> {
    let key = profile.rsa_public.as_ref().ok_or(CryptError::Bad)?;
    key.encrypt(&mut rand::rngs::SysRng, Pkcs1v15Encrypt, plain)
        .map_err(|_| CryptError::Bad)
}

fn rsa_private_decrypt(profile: &Profile, cipher: &[u8]) -> Result<Vec<u8>, CryptError> {
    let key = profile.rsa_private.as_ref().ok_or(CryptError::Bad)?;
    key.decrypt(Pkcs1v15Encrypt, cipher)
        .map_err(|_| CryptError::Bad)
}

fn rsa_private_encrypt(profile: &Profile, plain: &[u8]) -> Result<Vec<u8>, CryptError> {
    let key = profile.rsa_private.as_ref().ok_or(CryptError::Bad)?;
    let k = key.size();
    let padded = pkcs1_type1(plain, k)?;
    rsa_private_raw(key, &padded, k)
}

fn rsa_public_decrypt(profile: &Profile, cipher: &[u8]) -> Result<Vec<u8>, CryptError> {
    let key = profile.rsa_public.as_ref().ok_or(CryptError::Bad)?;
    let k = key.size();
    if cipher.len() != k {
        return Err(CryptError::Bad);
    }
    let opened = rsa_public_raw(key, cipher, k)?;
    pkcs1_type1_unpad(&opened)
}

fn uint_below_modulus(bytes: &[u8], n: &BoxedUint) -> Result<BoxedUint, CryptError> {
    let value = BoxedUint::from_be_slice(bytes, n.bits_precision()).map_err(|_| CryptError::Bad)?;
    if value >= *n {
        return Err(CryptError::Bad);
    }
    Ok(value)
}

fn rsa_private_raw(key: &RsaPrivateKey, block: &[u8], k: usize) -> Result<Vec<u8>, CryptError> {
    let m = uint_below_modulus(block, key.n().as_ref())?;
    let c = rsa_decrypt(Some(&mut rand::rngs::SysRng), key, &m).map_err(|_| CryptError::Bad)?;
    let full = c.to_be_bytes();
    let start = full.iter().position(|b| *b != 0).unwrap_or(full.len());
    Ok(left_pad(&full[start..], k))
}

fn rsa_public_raw(key: &RsaPublicKey, block: &[u8], k: usize) -> Result<Vec<u8>, CryptError> {
    let c = uint_below_modulus(block, key.n().as_ref())?;
    let m = rsa_encrypt(key, &c).map_err(|_| CryptError::Bad)?;
    let full = m.to_be_bytes();
    let start = full.iter().position(|b| *b != 0).unwrap_or(full.len());
    Ok(left_pad(&full[start..], k))
}

fn left_pad(raw: &[u8], len: usize) -> Vec<u8> {
    if raw.len() >= len {
        return raw[raw.len() - len..].to_vec();
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
    out[0] = 0x00;
    out[1] = 0x01;
    let sep = k - msg.len() - 1;
    out[sep] = 0x00;
    out[sep + 1..].copy_from_slice(msg);
    Ok(out)
}

fn pkcs1_type1_unpad(block: &[u8]) -> Result<Vec<u8>, CryptError> {
    if block.len() < 11 || block[0] != 0x00 || block[1] != 0x01 {
        return Err(CryptError::Bad);
    }
    let mut i = 2;
    while i < block.len() && block[i] == 0xff {
        i += 1;
    }
    if i < 10 || i >= block.len() || block[i] != 0x00 {
        return Err(CryptError::Bad);
    }
    Ok(block[i + 1..].to_vec())
}

fn rsa_sign(profile: &Profile, material: &[u8]) -> Result<Vec<u8>, CryptError> {
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    let key = profile.rsa_private.as_ref().ok_or(CryptError::Bad)?;
    let signer = SigningKey::<Sha256>::new(key.clone());
    let sig = Signer::sign(&signer, material);
    Ok(sig.to_bytes().into_vec())
}

pub fn rsa_verify(profile: &Profile, material: &[u8], sig: &[u8]) -> Result<bool, CryptError> {
    use rsa::pkcs1v15::VerifyingKey;
    use rsa::signature::Verifier;
    let key = profile.rsa_public.as_ref().ok_or(CryptError::Bad)?;
    let verifier = VerifyingKey::<Sha256>::new(key.clone());
    let signature = rsa::pkcs1v15::Signature::try_from(sig).map_err(|_| CryptError::Bad)?;
    Ok(verifier.verify(material, &signature).is_ok())
}

fn sm2_sign(profile: &Profile, material: &[u8]) -> Result<Vec<u8>, CryptError> {
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

fn sm2_verify(profile: &Profile, material: &[u8], sig: &[u8]) -> Result<bool, CryptError> {
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

fn sm2_encrypt(profile: &Profile, plain: &[u8]) -> Result<Vec<u8>, CryptError> {
    let ctx = libsm::sm2::signature::SigCtx::new();
    let pk = ctx
        .load_pubkey(profile.sm2_public.as_deref().ok_or(CryptError::Bad)?)
        .map_err(|_| CryptError::Bad)?;
    let enc = libsm::sm2::encrypt::EncryptCtx::new(plain.len(), pk);
    let raw = enc.encrypt(plain).map_err(|_| CryptError::Bad)?;
    Ok(repack_sm2(&raw, plain.len(), profile.sm2_mode, true))
}

fn sm2_decrypt(profile: &Profile, cipher: &[u8]) -> Result<Vec<u8>, CryptError> {
    if cipher.len() < 97 {
        return Err(CryptError::Bad);
    }
    let klen = cipher.len() - 97;
    let raw = repack_sm2(cipher, klen, profile.sm2_mode, false);
    let ctx = libsm::sm2::signature::SigCtx::new();
    let sk = ctx
        .load_seckey(profile.sm2_private.as_deref().ok_or(CryptError::Bad)?)
        .map_err(|_| CryptError::Bad)?;
    let dec = libsm::sm2::encrypt::DecryptCtx::new(klen, sk);
    dec.decrypt(&raw).map_err(|_| CryptError::Bad)
}

fn repack_sm2(data: &[u8], klen: usize, mode: Sm2Mode, to_profile: bool) -> Vec<u8> {
    let c1 = &data[..65];
    let rest = &data[65..];
    match mode {
        Sm2Mode::C1C2C3 => data.to_vec(),
        Sm2Mode::C1C3C2 if to_profile => {
            let c2 = &rest[..klen];
            let c3 = &rest[klen..];
            let mut out = c1.to_vec();
            out.extend_from_slice(c3);
            out.extend_from_slice(c2);
            out
        }
        Sm2Mode::C1C3C2 => {
            let c3 = &rest[..32];
            let c2 = &rest[32..];
            let mut out = c1.to_vec();
            out.extend_from_slice(c2);
            out.extend_from_slice(c3);
            out
        }
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
    let mut ipad = vec![0x36u8; BLOCK];
    let mut opad = vec![0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= keyed[i];
        opad[i] ^= keyed[i];
    }
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
    if data[data.len() - n..]
        .iter()
        .any(|byte| usize::from(*byte) != n)
    {
        return Err(CryptError::Bad);
    }
    Ok(data[..data.len() - n].to_vec())
}

pub fn load_rsa_private(pem: &str) -> Result<RsaPrivateKey, String> {
    RsaPrivateKey::from_pkcs8_pem(pem).map_err(|e| format!("rsa private key: {e}"))
}

pub fn load_rsa_public(pem: &str) -> Result<RsaPublicKey, String> {
    RsaPublicKey::from_public_key_pem(pem).map_err(|e| format!("rsa public key: {e}"))
}

pub fn unwrap_key(profile: &Profile, wrapped: &[u8]) -> Result<Vec<u8>, CryptError> {
    match profile.wrap.ok_or(CryptError::Bad)? {
        WrapAlg::RsaPkcs1Public => rsa_private_decrypt(profile, wrapped),
        WrapAlg::Sm2Encrypt => sm2_decrypt(profile, wrapped),
    }
}

pub fn wrap_key(profile: &Profile, key: &[u8]) -> Result<Vec<u8>, CryptError> {
    match profile.wrap.ok_or(CryptError::Bad)? {
        WrapAlg::RsaPkcs1Public => rsa_public_encrypt(profile, key),
        WrapAlg::Sm2Encrypt => sm2_encrypt(profile, key),
    }
}

#[cfg(test)]
mod tests {
    use super::{decrypt, decrypt_alg, encrypt, encrypt_alg, sign, unwrap_key, verify, wrap_key};
    use crate::envelope::profile::{
        CryptAlg, Encoding, HexCase, IvFrom, Profile, SignAlg, Sm2Mode,
    };

    fn flip(data: &[u8]) -> Vec<u8> {
        let mut out = data.to_vec();
        let last = out.len() - 1;
        out[last] ^= 0x01;
        out
    }

    fn round_sym(alg: CryptAlg, key: &[u8], mut profile: Profile) {
        profile.crypt_alg = Some(alg);
        profile.key = Some(key.to_vec());
        if matches!(
            alg,
            CryptAlg::AesCfb | CryptAlg::AesGcm | CryptAlg::DesCbcPkcs5 | CryptAlg::Sm4CbcPkcs7
        ) {
            let len = if alg == CryptAlg::DesCbcPkcs5 {
                8
            } else if alg == CryptAlg::AesGcm {
                profile.iv_len.unwrap_or(12)
            } else {
                16
            };
            profile.fixed_iv = Some(vec![7u8; len]);
            profile.iv_from = Some(IvFrom::Key);
        }
        let plain = b"envelope-roundtrip";
        let cipher = encrypt(&profile, key, plain).unwrap();
        let back = decrypt(&profile, key, &cipher).unwrap();
        assert_eq!(back, plain);
        let bad = decrypt(&profile, key, &flip(&cipher));
        assert!(bad.is_err() || bad.unwrap() != plain);
    }

    #[test]
    fn symmetric_roundtrips() {
        round_sym(
            CryptAlg::AesEcbPkcs5,
            &[1u8; 16],
            Profile::test_call_empty(),
        );
        round_sym(
            CryptAlg::AesEcbPkcs5,
            &[2u8; 24],
            Profile::test_call_empty(),
        );
        round_sym(
            CryptAlg::AesEcbPkcs5,
            &[3u8; 32],
            Profile::test_call_empty(),
        );
        let mut cfb = Profile::test_call_empty();
        cfb.segment_bits = Some(128);
        cfb.padding_pkcs5 = true;
        round_sym(CryptAlg::AesCfb, &[4u8; 16], cfb);
        let mut cfb8 = Profile::test_call_empty();
        cfb8.segment_bits = Some(8);
        cfb8.padding_pkcs5 = false;
        round_sym(CryptAlg::AesCfb, &[5u8; 16], cfb8);
        let mut gcm = Profile::test_call_empty();
        gcm.iv_len = Some(12);
        gcm.tag_bits = Some(128);
        round_sym(CryptAlg::AesGcm, &[6u8; 16], gcm);
        let mut gcm16 = Profile::test_call_empty();
        gcm16.iv_len = Some(16);
        gcm16.tag_bits = Some(128);
        round_sym(CryptAlg::AesGcm, &[6u8; 32], gcm16);
        round_sym(CryptAlg::DesCbcPkcs5, &[7u8; 8], Profile::test_call_empty());
        round_sym(
            CryptAlg::Sm4EcbPkcs7,
            &[8u8; 16],
            Profile::test_call_empty(),
        );
        round_sym(
            CryptAlg::Sm4CbcPkcs7,
            &[9u8; 16],
            Profile::test_call_empty(),
        );
        let mut none = Profile::test_call_empty();
        none.fixed_iv = None;
        let key = [10u8; 16];
        let plain = [11u8; 32];
        let cipher = encrypt_alg(&none, CryptAlg::Sm4EcbNoPadding, &key, &plain).unwrap();
        assert_eq!(
            decrypt_alg(&none, CryptAlg::Sm4EcbNoPadding, &key, &cipher).unwrap(),
            plain
        );
        assert!(decrypt_alg(&none, CryptAlg::Sm4EcbNoPadding, &key, &[1u8; 15]).is_err());
    }

    #[test]
    fn digest_and_hmac_case() {
        let mut md5p = Profile::test_call_empty();
        md5p.sign_alg = Some(SignAlg::Md5);
        md5p.sign_encoding = Encoding::Hex;
        md5p.hex_case = HexCase::Lower;
        let sig = sign(&md5p, b"abc").unwrap();
        assert_eq!(sig, "900150983cd24fb0d6963f7d28e17f72");
        assert!(verify(&md5p, b"abc", &sig).unwrap());
        assert!(!verify(&md5p, b"abc", &sig.to_ascii_uppercase()).unwrap());
        md5p.hex_case = HexCase::Upper;
        let upper = sign(&md5p, b"abc").unwrap();
        assert!(upper.chars().all(|ch| !ch.is_ascii_lowercase()));
        assert!(!verify(&md5p, b"abd", &upper).unwrap());

        let mut mac = Profile::test_call_empty();
        mac.sign_alg = Some(SignAlg::HmacSha256);
        mac.sign_key = Some(b"key".to_vec());
        mac.sign_encoding = Encoding::Base64;
        let sig = sign(&mac, b"msg").unwrap();
        assert!(verify(&mac, b"msg", &sig).unwrap());
        assert!(!verify(&mac, b"msg2", &sig).unwrap());

        let mut sm = Profile::test_call_empty();
        sm.sign_alg = Some(SignAlg::Sm3);
        sm.sign_encoding = Encoding::Hex;
        sm.hex_case = HexCase::Lower;
        let sig = sign(&sm, b"abc").unwrap();
        assert_eq!(
            sig,
            "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0"
        );
        assert!(!verify(&sm, b"abC", &sig).unwrap());

        let mut hsm = Profile::test_call_empty();
        hsm.sign_alg = Some(SignAlg::HmacSm3);
        hsm.sign_key = Some(b"sm3-key".to_vec());
        hsm.sign_encoding = Encoding::Hex;
        hsm.hex_case = HexCase::Lower;
        let sig = sign(&hsm, b"payload").unwrap();
        assert!(verify(&hsm, b"payload", &sig).unwrap());
        assert!(!verify(&hsm, b"payloaD", &sig).unwrap());
    }

    #[test]
    fn rsa_and_sm2_roundtrip() {
        let private = super::RsaPrivateKey::new(&mut rand::rngs::SysRng, 2048).unwrap();
        let public = super::RsaPublicKey::from(&private);
        let mut profile = Profile::test_call_empty();
        profile.rsa_private = Some(private);
        profile.rsa_public = Some(public);
        profile.sign_alg = Some(SignAlg::RsaSha256);
        profile.sign_encoding = Encoding::Base64;
        let sig = sign(&profile, b"rsa-body").unwrap();
        assert!(verify(&profile, b"rsa-body", &sig).unwrap());
        assert!(!verify(&profile, b"rsa-body!", &sig).unwrap());

        profile.wrap = Some(crate::envelope::profile::WrapAlg::RsaPkcs1Public);
        let wrapped = wrap_key(&profile, b"session-key").unwrap();
        assert_eq!(unwrap_key(&profile, &wrapped).unwrap(), b"session-key");
        let wrapped = encrypt_alg(&profile, CryptAlg::RsaPkcs1Public, &[], b"session-key").unwrap();
        let opened = decrypt_alg(&profile, CryptAlg::RsaPkcs1Public, &[], &wrapped).unwrap();
        assert_eq!(opened, b"session-key");
        // rsa 0.10 Pkcs1v15Encrypt 采用隐式拒绝（implicit rejection）抵抗 Marvin/Bleichenbacher 预言机：
        // 等长篡改密文返回伪随机合成明文而非直接报错，截断密文则直接返回错误。
        let tampered = decrypt_alg(&profile, CryptAlg::RsaPkcs1Public, &[], &flip(&wrapped));
        assert!(tampered.is_err() || tampered.unwrap() != b"session-key");
        assert!(
            decrypt_alg(
                &profile,
                CryptAlg::RsaPkcs1Public,
                &[],
                &wrapped[..wrapped.len() - 1],
            )
            .is_err()
        );

        let hidden = encrypt_alg(&profile, CryptAlg::RsaPkcs1Private, &[], b"biz-body").unwrap();
        let shown = decrypt_alg(&profile, CryptAlg::RsaPkcs1Private, &[], &hidden).unwrap();
        assert_eq!(shown, b"biz-body");
        assert!(decrypt_alg(&profile, CryptAlg::RsaPkcs1Private, &[], &flip(&hidden)).is_err());

        let ctx = libsm::sm2::signature::SigCtx::new();
        let (pk, sk) = ctx.new_keypair().unwrap();
        profile.sm2_private = Some(ctx.serialize_seckey(&sk).unwrap());
        profile.sm2_public = Some(ctx.serialize_pubkey(&pk, false).unwrap());
        profile.user_id = Some("1234567812345678".to_string());
        profile.sm2_mode = Sm2Mode::C1C3C2;
        profile.sign_alg = Some(SignAlg::Sm2Sign);
        profile.sign_encoding = Encoding::Base64;
        let sig = sign(&profile, b"sm2").unwrap();
        assert!(verify(&profile, b"sm2", &sig).unwrap());
        assert!(!verify(&profile, b"sm2!", &sig).unwrap());
        let enc = encrypt_alg(&profile, CryptAlg::Sm2Encrypt, &[], b"sm2-plain").unwrap();
        assert_eq!(
            decrypt_alg(&profile, CryptAlg::Sm2Encrypt, &[], &enc).unwrap(),
            b"sm2-plain"
        );
    }
}
