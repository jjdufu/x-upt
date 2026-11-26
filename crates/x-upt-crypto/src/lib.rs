#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]
//! Stateless HTTP authentication verifiers and envelope cryptographic primitives for `x-upt`.

pub mod canon;
pub mod crypt;
pub mod crypt_asym;
pub mod hawk_aws;
pub mod http_auth;
pub mod jwt_digest;
pub mod nonce;
pub mod pipeline;
pub mod profile;
pub mod slots;
pub mod table;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use x_upt_core::domain::{EnvelopeFieldDto, EnvelopeProfileSpec, FrontName};

    use crate::crypt::{decrypt, encrypt, sign, verify};
    use crate::crypt_asym::generate_secrets;
    use crate::nonce::NonceStore;
    use crate::pipeline::{DriveCtx, EnvelopeTokenStore, drive};
    use crate::profile::compile_profiles;
    use crate::slots::Incoming;
    use crate::table::parse_json;

    #[test]
    fn envelope_token_and_sm4_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let mut secrets = HashMap::new();
        for rec in generate_secrets("sm4_key", "bytes_16", "2026-01-01T00:00:00Z")? {
            secrets.insert(rec.name.clone(), rec);
        }
        for rec in generate_secrets("issue_user", "text_24", "2026-01-01T00:00:00Z")? {
            secrets.insert(rec.name.clone(), rec);
        }
        for rec in generate_secrets("issue_pass", "text_24", "2026-01-01T00:00:00Z")? {
            secrets.insert(rec.name.clone(), rec);
        }

        let user_txt = String::from_utf8(
            secrets
                .get("issue_user")
                .ok_or("missing user")?
                .raw_bytes
                .clone(),
        )?;
        let pass_txt = String::from_utf8(
            secrets
                .get("issue_pass")
                .ok_or("missing pass")?
                .raw_bytes
                .clone(),
        )?;

        let specs = vec![
            EnvelopeProfileSpec {
                name: "issue".into(),
                kind: "issue".into(),
                methods: vec!["POST".into()],
                path: "/issue".into(),
                body_kind: "json".into(),
                body_encoding: "identity".into(),
                slot_priority: None,
                fields: vec![
                    EnvelopeFieldDto {
                        name: "id".into(),
                        slot: "json".into(),
                        role: "auth".into(),
                    },
                    EnvelopeFieldDto {
                        name: "secret".into(),
                        slot: "json".into(),
                        role: "auth".into(),
                    },
                ],
                biz_rest: Some(false),
                token_prefix: None,
                token_response_field: Some("access_token".into()),
                secrets: HashMap::from([
                    ("id".into(), "issue_user".into()),
                    ("secret".into(), "issue_pass".into()),
                ]),
                canon: None,
                canon_skip: None,
                canon_sep: None,
                canon_prefix_key: None,
                canon_suffix_key: None,
                canon_fields: None,
                sign_alg: None,
                hex_case: None,
                sign_encoding: None,
                crypt_alg: None,
                cipher_scope: None,
                cipher_fields: None,
                crypt_encoding: None,
                padding: None,
                segment_bits: None,
                iv_len: None,
                iv_from: None,
                tag_bits: None,
                user_id: None,
                sm2_mode: None,
                key: None,
                sign_key: None,
                private_key: None,
                public_key: None,
                wrap: None,
                order: None,
                fail_in: "http".into(),
                response_body_kind: None,
                response_encoding: None,
                response_crypt_alg: None,
            },
            EnvelopeProfileSpec {
                name: "call".into(),
                kind: "call".into(),
                methods: vec!["POST".into()],
                path: "/call".into(),
                body_kind: "json".into(),
                body_encoding: "identity".into(),
                slot_priority: None,
                fields: vec![
                    EnvelopeFieldDto {
                        name: "authorization".into(),
                        slot: "header".into(),
                        role: "token".into(),
                    },
                    EnvelopeFieldDto {
                        name: "data".into(),
                        slot: "json".into(),
                        role: "cipher".into(),
                    },
                ],
                biz_rest: Some(false),
                token_prefix: Some("Bearer ".into()),
                token_response_field: None,
                secrets: HashMap::new(),
                canon: None,
                canon_skip: None,
                canon_sep: None,
                canon_prefix_key: None,
                canon_suffix_key: None,
                canon_fields: None,
                sign_alg: None,
                hex_case: None,
                sign_encoding: None,
                crypt_alg: Some("sm4_ecb_pkcs7".into()),
                cipher_scope: Some("fields".into()),
                cipher_fields: Some(vec!["data".into()]),
                crypt_encoding: Some("base64".into()),
                padding: None,
                segment_bits: None,
                iv_len: None,
                iv_from: None,
                tag_bits: None,
                user_id: None,
                sm2_mode: None,
                key: Some("sm4_key".into()),
                sign_key: None,
                private_key: None,
                public_key: None,
                wrap: None,
                order: None,
                fail_in: "http".into(),
                response_body_kind: None,
                response_encoding: None,
                response_crypt_alg: None,
            },
        ];

        let compiled = compile_profiles(FrontName::EnvelopeTokenCrypt, &specs, &secrets)?;
        let token_store = EnvelopeTokenStore::new(30);
        let nonces = NonceStore::new(600);
        let ctx = DriveCtx {
            front: FrontName::EnvelopeTokenCrypt,
            profiles: &compiled,
            tokens: Some(&token_store),
            nonces: &nonces,
            sign_window_seconds: 300,
        };

        let issue_out = drive(
            &ctx,
            &Incoming {
                method: "POST".into(),
                path: "/issue".into(),
                query: String::new(),
                headers: vec![("content-type".into(), "application/json".into())],
                body: serde_json::json!({ "id": user_txt, "secret": pass_txt })
                    .to_string()
                    .into_bytes(),
            },
        );
        assert_eq!(issue_out.status, 200);
        let issue_val: serde_json::Value = serde_json::from_slice(&issue_out.body)?;
        let tok = issue_val
            .get("access_token")
            .and_then(serde_json::Value::as_str)
            .ok_or("no token")?;

        let call_prof = compiled.get(1).ok_or("no call profile")?;
        let sm4_bytes = &secrets.get("sm4_key").ok_or("no sm4")?.raw_bytes;
        let sealed = encrypt(call_prof, sm4_bytes, br#"{"city":"hz","n":1.50}"#)?;
        let b64 = crate::crypt::encode_crypt(call_prof, &sealed);
        let call_out = drive(
            &ctx,
            &Incoming {
                method: "POST".into(),
                path: "/call".into(),
                query: String::new(),
                headers: vec![
                    ("content-type".into(), "application/json".into()),
                    ("authorization".into(), format!("Bearer {tok}")),
                ],
                body: serde_json::json!({ "data": b64 }).to_string().into_bytes(),
            },
        );
        assert_eq!(call_out.status, 200);
        let resp_json: serde_json::Value = serde_json::from_slice(&call_out.body)?;
        let resp_cipher = resp_json
            .get("data")
            .and_then(serde_json::Value::as_str)
            .ok_or("no resp cipher")?;
        let resp_raw = crate::crypt::decode_crypt(call_prof, resp_cipher)?;
        let plain_resp = decrypt(call_prof, sm4_bytes, &resp_raw)?;
        let parsed = parse_json(std::str::from_utf8(&plain_resp)?)?;
        assert!(parsed.canonical().contains(r#""hit":true"#));
        Ok(())
    }

    #[test]
    fn rsa_and_sm2_generation_and_sign() -> Result<(), Box<dyn std::error::Error>> {
        let mut secrets = HashMap::new();
        for rec in generate_secrets("rsa_pair", "rsa_2048", "2026-01-01T00:00:00Z")? {
            secrets.insert(rec.name.clone(), rec);
        }
        for rec in generate_secrets("sm2_pair", "sm2", "2026-01-01T00:00:00Z")? {
            secrets.insert(rec.name.clone(), rec);
        }

        let spec = EnvelopeProfileSpec {
            name: "rsa_sig".into(),
            kind: "call".into(),
            methods: vec!["POST".into()],
            path: "/rsa".into(),
            body_kind: "json".into(),
            body_encoding: "identity".into(),
            slot_priority: None,
            fields: vec![EnvelopeFieldDto {
                name: "sign".into(),
                slot: "json".into(),
                role: "sign".into(),
            }],
            biz_rest: Some(true),
            token_prefix: None,
            token_response_field: None,
            secrets: HashMap::new(),
            canon: Some("sorted_kv".into()),
            canon_skip: Some(vec!["sign".into()]),
            canon_sep: Some("&".into()),
            canon_prefix_key: None,
            canon_suffix_key: None,
            canon_fields: None,
            sign_alg: Some("rsa_sha256".into()),
            hex_case: None,
            sign_encoding: Some("base64".into()),
            crypt_alg: None,
            cipher_scope: None,
            cipher_fields: None,
            crypt_encoding: None,
            padding: None,
            segment_bits: None,
            iv_len: None,
            iv_from: None,
            tag_bits: None,
            user_id: None,
            sm2_mode: None,
            key: None,
            sign_key: None,
            private_key: Some("rsa_pair_private".into()),
            public_key: Some("rsa_pair_public".into()),
            wrap: None,
            order: None,
            fail_in: "http".into(),
            response_body_kind: None,
            response_encoding: None,
            response_crypt_alg: None,
        };
        let compiled = compile_profiles(FrontName::EnvelopeSignPlain, &[spec], &secrets)?;
        let prof = compiled.first().ok_or("empty")?;
        let sig = sign(prof, b"a=1")?;
        assert!(verify(prof, b"a=1", &sig)?);
        assert!(!verify(prof, b"a=2", &sig)?);
        Ok(())
    }
}
