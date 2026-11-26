use std::collections::HashMap;

use base64::Engine;

use crate::envelope::profile::{BodyEncoding, BodyKind, Profile, Slot};
use crate::envelope::table::{FieldTable, SlotValue, parse_json};

pub const BODY_LIMIT: usize = 1_048_576;

#[derive(Clone, Debug)]
pub struct Incoming {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotFail {
    TooLarge,
    Shape,
}

pub fn collect(profile: &Profile, incoming: &Incoming) -> Result<FieldTable, SlotFail> {
    if incoming.body.len() > BODY_LIMIT {
        return Err(SlotFail::TooLarge);
    }
    let decoded = decode_body(profile.body_encoding, &incoming.body)?;
    let bags = split_slots(profile, incoming, &decoded)?;
    let mut table = FieldTable::default();
    if profile.cipher_scope_body() {
        table.raw_cipher = Some(decoded);
    }
    for field in &profile.fields {
        let value = bags
            .lookup(field.slot, &field.name)
            .ok_or(SlotFail::Shape)?;
        table.insert(field.role.into(), field.name.clone(), value);
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
            .any(|slot| {
                self.map(slot)
                    .keys()
                    .any(|name| !profile.names_field(name, slot))
            })
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
            table.insert(
                crate::envelope::table::Role::Biz,
                name.clone(),
                value.clone(),
            );
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
    let header = header_map(&incoming.headers);
    let query = query_map(&incoming.query)?;
    let content_type = header
        .get("content-type")
        .map(|value| value.text())
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
            (form_map(text)?, HashMap::new())
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
    let (media, charset) = split_content_type(content_type);
    if !media.eq_ignore_ascii_case(expected) {
        return Err(SlotFail::Shape);
    }
    if let Some(charset) = charset
        && !charset.eq_ignore_ascii_case("utf-8")
    {
        return Err(SlotFail::Shape);
    }
    Ok(())
}

fn split_content_type(raw: &str) -> (String, Option<String>) {
    let mut parts = raw.split(';');
    let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let mut charset = None;
    for part in parts {
        let part = part.trim();
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("charset") {
            charset = Some(value.trim().trim_matches('"').to_string());
        }
    }
    (media, charset)
}

fn header_map(headers: &[(String, String)]) -> HashMap<String, SlotValue> {
    let mut out = HashMap::new();
    for (name, value) in headers {
        let key = name.to_ascii_lowercase();
        out.entry(key)
            .or_insert_with(|| SlotValue::String(value.clone()));
    }
    out
}

fn query_map(raw: &str) -> Result<HashMap<String, SlotValue>, SlotFail> {
    form_map(raw)
}

fn form_map(raw: &str) -> Result<HashMap<String, SlotValue>, SlotFail> {
    let mut out = HashMap::new();
    if raw.is_empty() {
        return Ok(out);
    }
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        out.insert(key.into_owned(), SlotValue::String(value.into_owned()));
    }
    Ok(out)
}

fn json_map(raw: &str) -> Result<HashMap<String, SlotValue>, SlotFail> {
    if raw.trim().is_empty() {
        return Err(SlotFail::Shape);
    }
    let value = parse_json(raw).map_err(|()| SlotFail::Shape)?;
    let SlotValue::Object(pairs) = value else {
        return Err(SlotFail::Shape);
    };
    let mut out = HashMap::new();
    for (key, value) in pairs {
        out.insert(key, value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{BODY_LIMIT, Incoming, SlotFail, collect};
    use base64::Engine;

    use crate::envelope::profile::{BodyEncoding, BodyKind, FieldSpec, Profile, Role, Slot};
    use crate::envelope::table::Role as TableRole;

    fn incoming(body: &str, headers: Vec<(&str, &str)>, query: &str) -> Incoming {
        Incoming {
            method: "POST".to_string(),
            path: "/call".to_string(),
            query: query.to_string(),
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn named_fields_only_and_trace_dropped() {
        let profile = Profile::test_call(
            BodyKind::Json,
            BodyEncoding::Identity,
            vec![FieldSpec::new("city", Slot::Json, Role::Biz)],
            false,
        );
        let got = collect(
            &profile,
            &incoming(
                r#"{"city":"hz","extra":1}"#,
                vec![
                    ("content-type", "application/json"),
                    ("X-Trace-Id", "abc"),
                    ("X-Request-Id", "req-1"),
                ],
                "city=query",
            ),
        )
        .unwrap_err();
        assert_eq!(got, SlotFail::Shape);

        let profile = Profile::test_call(
            BodyKind::Json,
            BodyEncoding::Identity,
            vec![FieldSpec::new("authorization", Slot::Header, Role::Token)],
            true,
        );
        let table = collect(
            &profile,
            &incoming(
                r#"{"city":"hz"}"#,
                vec![
                    ("content-type", "application/json"),
                    ("Authorization", "Bearer t"),
                    ("X-Trace-Id", "abc"),
                ],
                "noise=1",
            ),
        )
        .unwrap();
        assert_eq!(
            table.get(TableRole::Token, "authorization").unwrap().text(),
            "Bearer t"
        );
        assert!(!table.contains_name("x-trace-id"));
    }

    #[test]
    fn header_query_json_only_named_enter() {
        let profile = Profile::test_call(
            BodyKind::Json,
            BodyEncoding::Identity,
            vec![
                FieldSpec::new("authorization", Slot::Header, Role::Token),
                FieldSpec::new("city", Slot::Json, Role::Biz),
            ],
            true,
        );
        let table = collect(
            &profile,
            &incoming(
                r#"{"city":"hz","n":1.50}"#,
                vec![
                    ("content-type", "application/json; charset=utf-8"),
                    ("Authorization", "Bearer abc"),
                    ("X-Trace-Id", "nope"),
                    ("X-Request-Id", "req"),
                ],
                "city=from-query&sign=zzz",
            ),
        )
        .unwrap();
        assert_eq!(
            table.get(TableRole::Token, "authorization").unwrap().text(),
            "Bearer abc"
        );
        assert!(!table.contains_name("X-Trace-Id"));
        assert!(!table.contains_name("x-trace-id"));
        assert!(!table.contains_name("x-request-id"));
        let biz = table.biz();
        assert!(biz.iter().any(|(k, v)| k == "city" && v.text() == "hz"));
        assert!(biz.iter().any(|(k, v)| k == "n" && v.text() == "1.50"));
        assert!(biz.iter().any(|(k, v)| k == "sign" && v.text() == "zzz"));
        assert!(!table.contains_name("x-trace-id"));
    }

    #[test]
    fn form_one_stays_string() {
        let profile = Profile::test_call(
            BodyKind::Form,
            BodyEncoding::Identity,
            vec![FieldSpec::new("n", Slot::Form, Role::Biz)],
            false,
        );
        let table = collect(
            &profile,
            &incoming(
                "n=1",
                vec![("content-type", "application/x-www-form-urlencoded")],
                "",
            ),
        )
        .unwrap();
        assert_eq!(
            table.get(TableRole::Biz, "n").unwrap(),
            &crate::envelope::table::SlotValue::String("1".to_string())
        );
    }

    #[test]
    fn base64_body_is_decoded_before_json() {
        let raw = base64::engine::general_purpose::STANDARD.encode(r#"{"city":"hz"}"#);
        let profile = Profile::test_call(
            BodyKind::Json,
            BodyEncoding::Base64,
            vec![FieldSpec::new("city", Slot::Json, Role::Biz)],
            false,
        );
        let table = collect(
            &profile,
            &incoming(&raw, vec![("content-type", "application/json")], ""),
        )
        .unwrap();
        assert_eq!(table.get(TableRole::Biz, "city").unwrap().text(), "hz");
    }

    #[test]
    fn oversized_body_is_too_large() {
        let profile = Profile::test_call(BodyKind::Raw, BodyEncoding::Identity, vec![], false);
        let incoming = Incoming {
            method: "POST".to_string(),
            path: "/call".to_string(),
            query: String::new(),
            headers: vec![],
            body: vec![0; BODY_LIMIT + 1],
        };
        assert_eq!(
            collect(&profile, &incoming).unwrap_err(),
            SlotFail::TooLarge
        );
    }

    #[test]
    fn bad_charset_is_bad_shape() {
        let profile = Profile::test_call(BodyKind::Json, BodyEncoding::Identity, vec![], true);
        let err = collect(
            &profile,
            &incoming(
                "{}",
                vec![("content-type", "application/json; charset=gbk")],
                "",
            ),
        )
        .unwrap_err();
        assert_eq!(err, SlotFail::Shape);
    }
}
