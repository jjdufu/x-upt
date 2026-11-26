//! Envelope secret and profile persistence with cryptographic validation and dependency locks.

use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{Connection, params};
use x_upt_core::domain::{EnvelopeProfileSpec, EnvelopeSecretRecord, FrontName, SecretKind};
use x_upt_core::error::AppError;
use x_upt_crypto::crypt_asym::{generate_secrets, make_record};
use x_upt_crypto::profile::compile_profiles;

use crate::dto::{EnvelopeProfilesView, SecretMetaView, SecretUpsertPayload};
use crate::queries::{load_fronts, load_profiles_map, load_secrets, usize_to_i64};
use crate::schema::{db_err, json_err};

/// Converts an [`EnvelopeSecretRecord`] into redacted [`SecretMetaView`].
#[must_use]
pub fn to_secret_meta(rec: &EnvelopeSecretRecord) -> SecretMetaView {
    SecretMetaView {
        name: rec.name.clone(),
        secret_kind: rec.kind.as_str().to_string(),
        byte_len: rec.raw_bytes.len(),
        fingerprint: rec.fingerprint.clone(),
        public_material: rec.public_material.clone(),
        updated_at: rec.updated_at.clone(),
    }
}

/// Lists all envelope secrets with secret values redacted.
///
/// # Errors
/// Returns [`AppError::Internal`] on `SQLite` query failure.
pub fn list_secret_metas(conn: &Connection) -> Result<Vec<SecretMetaView>, AppError> {
    Ok(load_secrets(conn)?.iter().map(to_secret_meta).collect())
}

/// Uploads or generates one or more envelope secrets and validates any active envelope fronts.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] on invalid key format, or [`AppError::Conflict`] if active fronts break.
pub fn upsert_envelope_secret(
    conn: &mut Connection,
    payload: SecretUpsertPayload,
) -> Result<Vec<SecretMetaView>, AppError> {
    let now = Utc::now().to_rfc3339();
    let records = if let Some(mode) = payload
        .generate
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        generate_secrets(&payload.name, mode, &now).map_err(AppError::InvalidInput)?
    } else {
        vec![parse_manual_secret(&payload, &now)?]
    };

    let tx = conn.transaction().map_err(db_err)?;
    for rec in &records {
        tx.execute(
            "INSERT INTO envelope_secrets (name, secret_kind, raw_bytes, fingerprint, public_material, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET
                secret_kind = excluded.secret_kind,
                raw_bytes = excluded.raw_bytes,
                fingerprint = excluded.fingerprint,
                public_material = excluded.public_material,
                updated_at = excluded.updated_at",
            params![
                rec.name,
                rec.kind.as_str(),
                rec.raw_bytes,
                rec.fingerprint,
                rec.public_material,
                rec.updated_at,
            ],
        )
        .map_err(db_err)?;
    }

    verify_active_envelopes(&tx)?;
    tx.commit().map_err(db_err)?;
    Ok(records.iter().map(to_secret_meta).collect())
}

fn parse_manual_secret(
    payload: &SecretUpsertPayload,
    now: &str,
) -> Result<EnvelopeSecretRecord, AppError> {
    let name = payload.name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::InvalidInput(
            "密钥名称长度须在 1 到 64 字符之间".to_string(),
        ));
    }
    let kind_str = payload.secret_kind.as_deref().unwrap_or("text");
    let kind = SecretKind::parse(kind_str).map_err(AppError::InvalidInput)?;
    let raw_value = payload
        .value
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput("上传密钥时须提供 value".to_string()))?;
    if raw_value.trim().is_empty() {
        return Err(AppError::InvalidInput("密钥内容不能为空".to_string()));
    }

    let (raw_bytes, pub_mat) = match kind {
        SecretKind::Text => (raw_value.as_bytes().to_vec(), None),
        SecretKind::BytesHex => {
            let clean = raw_value.trim();
            let decoded = hex::decode(clean)
                .map_err(|e| AppError::InvalidInput(format!("十六进制密钥解析失败: {e}")))?;
            if decoded.is_empty() {
                return Err(AppError::InvalidInput(
                    "解码后的密钥字节不能为空".to_string(),
                ));
            }
            (decoded, None)
        }
        SecretKind::Pem => {
            let trimmed = raw_value.trim();
            let pub_only = if trimmed.contains("PUBLIC KEY") && !trimmed.contains("PRIVATE KEY") {
                Some(trimmed.to_string())
            } else {
                None
            };
            (trimmed.as_bytes().to_vec(), pub_only)
        }
    };

    Ok(make_record(name.to_string(), kind, raw_bytes, pub_mat, now))
}

/// Deletes an envelope secret if not referenced by any enabled envelope front.
///
/// # Errors
/// Returns [`AppError::NotFound`] if missing, or [`AppError::Conflict`] if referenced by an active front.
pub fn delete_envelope_secret(conn: &mut Connection, name: &str) -> Result<(), AppError> {
    let tx = conn.transaction().map_err(db_err)?;
    let fronts = load_fronts(&tx)?;
    let profiles_map = load_profiles_map(&tx)?;

    let mut blocking = Vec::new();
    for f in fronts.iter().filter(|f| f.enabled && f.name.is_envelope()) {
        if let Some(specs) = profiles_map.get(&f.name)
            && specs.iter().any(|s| spec_references_secret(s, name))
        {
            blocking.push(f.name.as_str().to_string());
        }
    }
    if !blocking.is_empty() {
        return Err(AppError::in_use(
            format!("密钥 {name} 正在被已启用的信封接入面引用，请先关闭对应接入面"),
            blocking,
        ));
    }

    let deleted = tx
        .execute(
            "DELETE FROM envelope_secrets WHERE name = ?1",
            params![name],
        )
        .map_err(db_err)?;
    if deleted == 0 {
        return Err(AppError::NotFound(format!("未找到密钥: {name}")));
    }
    verify_active_envelopes(&tx)?;
    tx.commit().map_err(db_err)
}

fn spec_references_secret(spec: &EnvelopeProfileSpec, target: &str) -> bool {
    spec.secrets.values().any(|v| v == target)
        || spec.key.as_deref() == Some(target)
        || spec.sign_key.as_deref() == Some(target)
        || spec.private_key.as_deref() == Some(target)
        || spec.public_key.as_deref() == Some(target)
}

/// Returns the stored envelope profiles for `front`.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] if `front` is not one of the 6 envelope fronts.
pub fn get_front_profiles(
    conn: &Connection,
    front: FrontName,
) -> Result<EnvelopeProfilesView, AppError> {
    if !front.is_envelope() {
        return Err(AppError::InvalidInput(format!(
            "{} 不是信封接入面",
            front.as_str()
        )));
    }
    let mut map = load_profiles_map(conn)?;
    let profiles = map.remove(&front).unwrap_or_default();
    Ok(EnvelopeProfilesView {
        front: front.as_str().to_string(),
        profiles,
    })
}

/// Replaces all envelope profiles for `front`, validating compilation if Secrets are present or if `front` is enabled.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] or [`AppError::Conflict`] on invalid profile/key configuration.
pub fn replace_front_profiles(
    conn: &mut Connection,
    front: FrontName,
    profiles: Vec<EnvelopeProfileSpec>,
) -> Result<EnvelopeProfilesView, AppError> {
    if !front.is_envelope() {
        return Err(AppError::InvalidInput(format!(
            "{} 不是信封接入面",
            front.as_str()
        )));
    }

    let tx = conn.transaction().map_err(db_err)?;
    let secrets = secrets_by_name(&tx)?;

    if !profiles.is_empty() {
        compile_profiles(front, &profiles, &secrets).map_err(AppError::InvalidInput)?;
    }

    tx.execute(
        "DELETE FROM envelope_profiles WHERE front = ?1",
        params![front.as_str()],
    )
    .map_err(db_err)?;
    for (idx, spec) in profiles.iter().enumerate() {
        let spec_json = serde_json::to_string(spec).map_err(json_err)?;
        tx.execute(
            "INSERT INTO envelope_profiles (front, ord, spec_json) VALUES (?1, ?2, ?3)",
            params![front.as_str(), usize_to_i64(idx), spec_json],
        )
        .map_err(db_err)?;
    }

    verify_active_envelopes(&tx)?;
    tx.commit().map_err(db_err)?;
    Ok(EnvelopeProfilesView {
        front: front.as_str().to_string(),
        profiles,
    })
}

pub(crate) fn secrets_by_name(
    conn: &Connection,
) -> Result<HashMap<String, EnvelopeSecretRecord>, AppError> {
    Ok(load_secrets(conn)?
        .into_iter()
        .map(|s| (s.name.clone(), s))
        .collect())
}

fn verify_active_envelopes(conn: &Connection) -> Result<(), AppError> {
    let fronts = load_fronts(conn)?;
    let profiles_map = load_profiles_map(conn)?;
    let secrets = secrets_by_name(conn)?;
    for f in fronts
        .into_iter()
        .filter(|f| f.enabled && f.name.is_envelope())
    {
        let specs = profiles_map.get(&f.name).cloned().unwrap_or_default();
        if specs.is_empty() {
            return Err(AppError::in_use(
                format!("已启用信封面 {} 不能清空全部画像", f.name.as_str()),
                vec![f.name.as_str().to_string()],
            ));
        }
        if f.name.is_token_envelope() {
            let has_issue = specs.iter().any(|p| p.kind == "issue");
            let has_call = specs.iter().any(|p| p.kind == "call");
            if !has_issue || !has_call {
                return Err(AppError::in_use(
                    format!(
                        "已启用换票信封面 {} 须同时保留 issue 与 call 画像",
                        f.name.as_str()
                    ),
                    vec![f.name.as_str().to_string()],
                ));
            }
        }
        compile_profiles(f.name, &specs, &secrets).map_err(|e| {
            AppError::in_use(
                format!("已启用信封面 {} 校验未通过: {e}", f.name.as_str()),
                vec![f.name.as_str().to_string()],
            )
        })?;
    }
    Ok(())
}
