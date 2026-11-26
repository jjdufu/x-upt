use crate::envelope::profile::{CanonKind, Profile};
use crate::envelope::table::{FieldTable, Role, SlotValue};

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
    let name = profile
        .canon_fields
        .first()
        .map(String::as_str)
        .unwrap_or("");
    field_bytes(table, name)
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
    let mut pairs: Vec<(String, String)> = Vec::new();
    push_pairs(&mut pairs, table.biz());
    for role in [
        Role::Auth,
        Role::Cipher,
        Role::Timestamp,
        Role::Nonce,
        Role::Token,
        Role::WrapKey,
    ] {
        for name in table.names(role) {
            if let Some(value) = table.get(role, &name) {
                pairs.push((name, value.text()));
            }
        }
    }
    pairs.retain(|(name, _)| !profile.canon_skip.iter().any(|skip| skip == name));
    pairs.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    pairs.dedup_by(|right, left| right.0 == left.0);
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(&profile.canon_sep)
        .into_bytes()
}

fn push_pairs(out: &mut Vec<(String, String)>, pairs: &[(String, SlotValue)]) {
    for (key, value) in pairs {
        out.push((key.clone(), value.text()));
    }
}

fn field_text(table: &FieldTable, name: &str) -> String {
    if let Some((_, value)) = table.biz().iter().find(|(key, _)| key == name) {
        return value.text();
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
        if let Some(value) = table.get(role, name) {
            return value.text();
        }
    }
    String::new()
}

fn field_bytes(table: &FieldTable, name: &str) -> Vec<u8> {
    field_text(table, name).into_bytes()
}

fn affix(profile: &Profile, mut body: Vec<u8>) -> Vec<u8> {
    let key = profile.canon_key();
    if profile.canon_prefix_key {
        let mut out = Vec::new();
        out.extend_from_slice(&key);
        if !profile.canon_sep.is_empty() {
            out.extend_from_slice(profile.canon_sep.as_bytes());
        }
        out.extend_from_slice(&body);
        body = out;
    }
    if profile.canon_suffix_key {
        if !profile.canon_sep.is_empty() {
            body.extend_from_slice(profile.canon_sep.as_bytes());
        }
        body.extend_from_slice(&key);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::material;
    use crate::envelope::profile::{CanonKind, HexCase, Profile};
    use crate::envelope::table::{FieldTable, Role, SlotValue};

    fn table() -> FieldTable {
        let mut table = FieldTable::default();
        table.insert(Role::Biz, "b", SlotValue::String("2".to_string()));
        table.insert(Role::Biz, "a", SlotValue::String("1".to_string()));
        table.insert(Role::Sign, "sign", SlotValue::String("dead".to_string()));
        table
    }

    #[test]
    fn sorted_kv_skips_sign_and_appends_key() {
        let mut profile = Profile::test_sign(CanonKind::SortedKv);
        profile.canon_skip = vec!["sign".to_string()];
        profile.canon_sep = "&".to_string();
        profile.canon_suffix_key = true;
        profile.key = Some(b"secret".to_vec());
        profile.hex_case = HexCase::Lower;
        let got = material(&profile, &table(), None);
        assert_eq!(got, b"a=1&b=2&secret");
    }

    #[test]
    fn concat_and_raw() {
        let mut profile = Profile::test_sign(CanonKind::Concat);
        profile.canon_fields = vec!["b".to_string(), "a".to_string()];
        profile.canon_sep = String::new();
        profile.canon_prefix_key = true;
        profile.key = Some(b"K".to_vec());
        assert_eq!(material(&profile, &table(), None), b"K21");

        let mut raw = Profile::test_sign(CanonKind::Raw);
        raw.canon_fields = vec!["a".to_string()];
        raw.hex_case = HexCase::Upper;
        assert_eq!(material(&raw, &table(), None), b"1");
    }
}
