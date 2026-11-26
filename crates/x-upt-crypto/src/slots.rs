//! Envelope 4-slot parameter extractor (`header`, `query`, `form`, `json`) and 1 MiB body limit.

use std::collections::HashMap;

use base64::Engine;

use crate::profile::{BodyEncoding, BodyKind, Profile, Slot};
use crate::table::{FieldTable, Role, SlotValue, parse_json};

/// Hard maximum envelope request body size (`1 MiB`).
pub const BODY_LIMIT: usize = 1_048_576;

/// Raw inbound HTTP request representation for envelope processing.
#[derive(Clone, Debug)]
pub struct Incoming {
    /// HTTP method (`GET` or `POST`).
    pub method: String,
    /// Request path.
    pub path: String,
    /// Raw query string.
    pub query: String,
    /// Request headers `(name, value)`.
    pub headers: Vec<(String, String)>,
    /// Raw request body bytes.
    pub body: Vec<u8>,
}

/// Slot extraction failure category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotFail {
    /// Body exceeds [`BODY_LIMIT`] (`413`).
    TooLarge,
    /// Content-Type, encoding, or required field shape error (`bad_shape`).
    Shape,
}

/// Extracts parameters from `incoming` into a [`FieldTable`] according to `profile`.
///
/// # Errors
/// Returns [`SlotFail::TooLarge`] if body exceeds 1 MiB or [`SlotFail::Shape`] on malformed input.
pub fn collect(profile: &Profile, incoming: &Incoming) -> Result<FieldTable, SlotFail> {
    if incoming.body.len() > BODY_LIMIT {
        return Err(SlotFail::TooLarge);
    }
    let decoded = decode_body(profile.body_encoding, &incoming.body)?;
    let bags = split_slots(profile, incoming, &decoded)?;
    let mut table = FieldTable::default();
    if profile.cipher_flags.scope_body {
        table.raw_cipher = Some(decoded);
    }
    for field in &profile.fields {
        let value = bags
            .lookup(field.slot, &field.name)
            .ok_or(SlotFail::Shape)?;
        table.insert(field.role, field.name.clone(), value);
    }
    if profile.biz_rest {
        fill_rest(profile, &bags, &mut table);
    } else if bags.has_unnamed(profile) {
        return Err(SlotFail::Shape);
    }
    Ok(table)
}

struct Bags {
    header: HashMap<String, SlotValue>,
    query: HashMap<String, SlotValue>,
    form: HashMap<String, SlotValue>,
    json: HashMap<String, SlotValue>,
}

impl Bags {
    fn lookup(&self, slot: Slot, name: &str) -> Option<SlotValue> {
        let key = if slot == Slot::Header {
            name.to_ascii_lowercase()
        } else {
            name.to_string()
        };
        self.map(slot).get(&key).cloned()
    }

    fn map(&self, slot: Slot) -> &HashMap<String, SlotValue> {
        match slot {
            Slot::Header => &self.header,
            Slot::Query => &self.query,
            Slot::Form => &self.form,
            Slot::Json => &self.json,
        }
    }

    fn has_unnamed(&self, profile: &Profile) -> bool {
        [Slot::Json, Slot::Form, Slot::Query]
            .into_iter()
            .any(|slot| self.map(slot).keys().any(|n| !profile.names_field(n, slot)))
    }
}

fn fill_rest(profile: &Profile, bags: &Bags, table: &mut FieldTable) {
    for slot in profile.slot_priority.iter().rev().copied() {
        if slot == Slot::Header {
            continue;
        }
        for (name, value) in bags.map(slot) {
            if profile.names_field(name, slot) || profile.named_anywhere(name) {
                continue;
            }
            table.insert(Role::Biz, name.clone(), value.clone());
        }
    }
}

fn decode_body(encoding: BodyEncoding, body: &[u8]) -> Result<Vec<u8>, SlotFail> {
    match encoding {
        BodyEncoding::Identity => Ok(body.to_vec()),
        BodyEncoding::Base64 => {
            let text = std::str::from_utf8(body).map_err(|_| SlotFail::Shape)?;
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(Vec::new());
            }
            base64::engine::general_purpose::STANDARD
                .decode(trimmed)
                .map_err(|_| SlotFail::Shape)
        }
    }
}

fn split_slots(profile: &Profile, incoming: &Incoming, body: &[u8]) -> Result<Bags, SlotFail> {
    let mut header = HashMap::new();
    for (name, value) in &incoming.headers {
        header
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| SlotValue::String(value.clone()));
    }
    let query = form_map(&incoming.query);
    let content_type = header
        .get("content-type")
        .map(SlotValue::text)
        .unwrap_or_default();
    let (form, json) = match profile.body_kind {
        BodyKind::Empty => {
            if !body.is_empty() {
                return Err(SlotFail::Shape);
            }
            (HashMap::new(), HashMap::new())
        }
        BodyKind::Raw => (HashMap::new(), HashMap::new()),
        BodyKind::Form => {
            expect_type(&content_type, "application/x-www-form-urlencoded")?;
            let text = std::str::from_utf8(body).map_err(|_| SlotFail::Shape)?;
            (form_map(text), HashMap::new())
        }
        BodyKind::Json => {
            expect_type(&content_type, "application/json")?;
            let text = std::str::from_utf8(body).map_err(|_| SlotFail::Shape)?;
            (HashMap::new(), json_map(text)?)
        }
    };
    Ok(Bags {
        header,
        query,
        form,
        json,
    })
}

fn expect_type(content_type: &str, expected: &str) -> Result<(), SlotFail> {
    let mut parts = content_type.split(';');
    let media = parts.next().unwrap_or("").trim();
    if !media.eq_ignore_ascii_case(expected) {
        return Err(SlotFail::Shape);
    }
    for part in parts {
        if let Some((k, v)) = part.trim().split_once('=')
            && k.trim().eq_ignore_ascii_case("charset")
            && !v.trim().trim_matches('"').eq_ignore_ascii_case("utf-8")
        {
            return Err(SlotFail::Shape);
        }
    }
    Ok(())
}

fn form_map(raw: &str) -> HashMap<String, SlotValue> {
    let mut out = HashMap::new();
    if raw.is_empty() {
        return out;
    }
    for (k, v) in url::form_urlencoded::parse(raw.as_bytes()) {
        out.insert(k.into_owned(), SlotValue::String(v.into_owned()));
    }
    out
}

fn json_map(raw: &str) -> Result<HashMap<String, SlotValue>, SlotFail> {
    if raw.trim().is_empty() {
        return Err(SlotFail::Shape);
    }
    let SlotValue::Object(pairs) = parse_json(raw).map_err(|_| SlotFail::Shape)? else {
        return Err(SlotFail::Shape);
    };
    Ok(pairs.into_iter().collect())
}
