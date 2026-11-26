//! Envelope signing material canonicalization (`sorted_kv`, `concat`, `raw`).

use crate::profile::{CanonKind, Profile};
use crate::table::{FieldTable, Role};

/// Builds the byte sequence to sign or verify according to `profile.canon`.
#[must_use]
pub fn material(profile: &Profile, table: &FieldTable, plain_override: Option<&[u8]>) -> Vec<u8> {
    let Some(kind) = profile.canon else {
        return Vec::new();
    };
    let body = match kind {
        CanonKind::Raw => raw_material(profile, table, plain_override),
        CanonKind::Concat => concat_material(profile, table),
        CanonKind::SortedKv => sorted_kv(profile, table),
    };
    affix(profile, body)
}

fn raw_material(profile: &Profile, table: &FieldTable, plain_override: Option<&[u8]>) -> Vec<u8> {
    if let Some(plain) = plain_override {
        return plain.to_vec();
    }
    let name = profile.canon_fields.first().map_or("", String::as_str);
    field_text(table, name).into_bytes()
}

fn concat_material(profile: &Profile, table: &FieldTable) -> Vec<u8> {
    let mut parts = Vec::new();
    for name in &profile.canon_fields {
        if profile.canon_skip.iter().any(|skip| skip == name) {
            continue;
        }
        parts.push(field_text(table, name));
    }
    parts.join(&profile.canon_sep).into_bytes()
}

fn sorted_kv(profile: &Profile, table: &FieldTable) -> Vec<u8> {
    let mut pairs: Vec<(String, String)> = table
        .biz()
        .iter()
        .map(|(k, v)| (k.clone(), v.text()))
        .collect();
    for role in [
        Role::Auth,
        Role::Cipher,
        Role::Timestamp,
        Role::Nonce,
        Role::Token,
        Role::WrapKey,
    ] {
        for name in table.names(role) {
            if let Some(val) = table.get(role, &name) {
                pairs.push((name, val.text()));
            }
        }
    }
    pairs.retain(|(name, _)| !profile.canon_skip.iter().any(|skip| skip == name));
    pairs.sort_by(|l, r| l.0.as_bytes().cmp(r.0.as_bytes()));
    pairs.dedup_by(|r, l| r.0 == l.0);
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(&profile.canon_sep)
        .into_bytes()
}

fn field_text(table: &FieldTable, name: &str) -> String {
    if let Some((_, val)) = table.biz().iter().find(|(k, _)| k == name) {
        return val.text();
    }
    for role in [
        Role::Cipher,
        Role::Auth,
        Role::Sign,
        Role::Token,
        Role::Timestamp,
        Role::Nonce,
        Role::WrapKey,
        Role::Biz,
    ] {
        if let Some(val) = table.get(role, name) {
            return val.text();
        }
    }
    String::new()
}

fn affix(profile: &Profile, mut body: Vec<u8>) -> Vec<u8> {
    let key = profile.canon_key();
    if profile.canon_affix.prefix_key {
        let mut out = Vec::new();
        out.extend_from_slice(&key);
        if !profile.canon_sep.is_empty() {
            out.extend_from_slice(profile.canon_sep.as_bytes());
        }
        out.extend_from_slice(&body);
        body = out;
    }
    if profile.canon_affix.suffix_key {
        if !profile.canon_sep.is_empty() {
            body.extend_from_slice(profile.canon_sep.as_bytes());
        }
        body.extend_from_slice(&key);
    }
    body
}
