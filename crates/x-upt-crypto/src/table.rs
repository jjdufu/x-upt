//! Envelope lexical JSON parser, `SlotValue` canonicalizer, and `FieldTable`.

use std::collections::BTreeMap;

use serde_json::{Map, Number, Value};

/// Semantic role of an extracted envelope parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Static credential field.
    Auth,
    /// Ciphertext field.
    Cipher,
    /// Business payload field.
    Biz,
    /// Signature field.
    Sign,
    /// Session/bearer token field.
    Token,
    /// Asymmetric-wrapped symmetric key field.
    WrapKey,
    /// Request timestamp field.
    Timestamp,
    /// Request replay nonce field.
    Nonce,
}

/// Lexical JSON/form value preserving exact number formatting for deterministic canonical digests.
#[derive(Clone, Debug, PartialEq)]
pub enum SlotValue {
    /// JSON `null`.
    Null,
    /// Boolean literal.
    Bool(bool),
    /// Exact lexical JSON number string (e.g. `"1.50"`).
    Number(String),
    /// String value.
    String(String),
    /// Ordered JSON array.
    Array(Vec<SlotValue>),
    /// JSON object entries.
    Object(Vec<(String, SlotValue)>),
}

impl SlotValue {
    /// Unquoted scalar text or canonical JSON for nested containers.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(true) => "true".to_string(),
            Self::Bool(false) => "false".to_string(),
            Self::Number(raw) | Self::String(raw) => raw.clone(),
            other => other.canonical(),
        }
    }

    /// Canonical UTF-8 key-sorted JSON text used for `digest` calculation.
    #[must_use]
    pub fn canonical(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Bool(true) => "true".to_string(),
            Self::Bool(false) => "false".to_string(),
            Self::Number(raw) => raw.clone(),
            Self::String(raw) => json_string(raw),
            Self::Array(items) => {
                let inner = items
                    .iter()
                    .map(Self::canonical)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("[{inner}]")
            }
            Self::Object(pairs) => {
                let mut ordered: BTreeMap<&str, &SlotValue> = BTreeMap::new();
                for (key, value) in pairs {
                    ordered.insert(key.as_str(), value);
                }
                let mut out = String::from("{");
                for (i, (key, value)) in ordered.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&json_string(key));
                    out.push(':');
                    out.push_str(&value.canonical());
                }
                out.push('}');
                out
            }
        }
    }

    /// Compact JSON preserving pair insertion order for outbound response bodies.
    #[must_use]
    pub fn compact(&self) -> String {
        match self {
            Self::Object(pairs) => {
                let mut out = String::from("{");
                for (i, (key, value)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&json_string(key));
                    out.push(':');
                    out.push_str(&value.compact());
                }
                out.push('}');
                out
            }
            Self::Array(items) => {
                let inner = items
                    .iter()
                    .map(Self::compact)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("[{inner}]")
            }
            other => other.canonical(),
        }
    }

    /// Converts into `serde_json::Value` for structured access logging.
    #[must_use]
    pub fn to_serde(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(v) => Value::Bool(*v),
            Self::Number(raw) => raw
                .parse::<Number>()
                .map_or_else(|_| Value::String(raw.clone()), Value::Number),
            Self::String(raw) => Value::String(raw.clone()),
            Self::Array(items) => Value::Array(items.iter().map(Self::to_serde).collect()),
            Self::Object(pairs) => {
                let mut map = Map::new();
                for (key, value) in pairs {
                    map.insert(key.clone(), value.to_serde());
                }
                Value::Object(map)
            }
        }
    }
}

/// Escapes `raw` as a JSON string literal.
#[must_use]
pub fn json_string(raw: &str) -> String {
    serde_json::to_string(raw).unwrap_or_else(|_| format!("\"{raw}\""))
}

/// Wraps business key-value pairs as a [`SlotValue::Object`].
#[must_use]
pub fn biz_object(pairs: &[(String, SlotValue)]) -> SlotValue {
    SlotValue::Object(pairs.to_vec())
}

/// Error returned when lexical JSON parsing fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsonParseError;

impl std::fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid JSON syntax")
    }
}

impl std::error::Error for JsonParseError {}

/// Parses JSON text while preserving exact number lexical representations.
///
/// # Errors
/// Returns [`JsonParseError`] when `input` is not valid RFC 8259 JSON.
pub fn parse_json(input: &str) -> Result<SlotValue, JsonParseError> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        index: 0,
    };
    let value = parser.value().map_err(|()| JsonParseError)?;
    parser.skip_ws();
    if parser.index != parser.bytes.len() {
        return Err(JsonParseError);
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.index += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) -> Result<u8, ()> {
        let b = self.peek().ok_or(())?;
        self.index += 1;
        Ok(b)
    }

    fn value(&mut self) -> Result<SlotValue, ()> {
        self.skip_ws();
        match self.peek().ok_or(())? {
            b'n' => self.literal(b"null", SlotValue::Null),
            b't' => self.literal(b"true", SlotValue::Bool(true)),
            b'f' => self.literal(b"false", SlotValue::Bool(false)),
            b'"' => Ok(SlotValue::String(self.string()?)),
            b'[' => self.array(),
            b'{' => self.object(),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(()),
        }
    }

    fn literal(&mut self, expected: &[u8], value: SlotValue) -> Result<SlotValue, ()> {
        for &byte in expected {
            if self.bump()? != byte {
                return Err(());
            }
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<SlotValue, ()> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        match self.peek() {
            Some(b'0') => self.index += 1,
            Some(b'1'..=b'9') => self.consume_digits(),
            _ => return Err(()),
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(());
            }
            self.consume_digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(());
            }
            self.consume_digits();
        }
        let slice = self.bytes.get(start..self.index).ok_or(())?;
        let raw = std::str::from_utf8(slice).map_err(|_| ())?;
        Ok(SlotValue::Number(raw.to_string()))
    }

    fn consume_digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.index += 1;
        }
    }

    fn string(&mut self) -> Result<String, ()> {
        if self.bump()? != b'"' {
            return Err(());
        }
        let mut out = String::new();
        loop {
            match self.bump()? {
                b'"' => return Ok(out),
                b'\\' => self.read_escape(&mut out)?,
                b if b < 0x20 => return Err(()),
                _ => {
                    self.index -= 1;
                    let rest = std::str::from_utf8(self.bytes.get(self.index..).ok_or(())?)
                        .map_err(|_| ())?;
                    let ch = rest.chars().next().ok_or(())?;
                    out.push(ch);
                    self.index += ch.len_utf8();
                }
            }
        }
    }

    fn read_escape(&mut self, out: &mut String) -> Result<(), ()> {
        match self.bump()? {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{0008}'),
            b'f' => out.push('\u{000c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let mut hex = [0u8; 4];
                for slot in &mut hex {
                    *slot = self.bump()?;
                }
                let text = std::str::from_utf8(&hex).map_err(|_| ())?;
                let code = u32::from_str_radix(text, 16).map_err(|_| ())?;
                out.push(char::from_u32(code).ok_or(())?);
            }
            _ => return Err(()),
        }
        Ok(())
    }

    fn array(&mut self) -> Result<SlotValue, ()> {
        if self.bump()? != b'[' {
            return Err(());
        }
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.index += 1;
            return Ok(SlotValue::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.bump()? {
                b']' => return Ok(SlotValue::Array(items)),
                b',' => {}
                _ => return Err(()),
            }
        }
    }

    fn object(&mut self) -> Result<SlotValue, ()> {
        if self.bump()? != b'{' {
            return Err(());
        }
        self.skip_ws();
        let mut pairs: Vec<(String, SlotValue)> = Vec::new();
        if self.peek() == Some(b'}') {
            self.index += 1;
            return Ok(SlotValue::Object(pairs));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(());
            }
            let key = self.string()?;
            self.skip_ws();
            if self.bump()? != b':' {
                return Err(());
            }
            let value = self.value()?;
            upsert(&mut pairs, key, value);
            self.skip_ws();
            match self.bump()? {
                b'}' => return Ok(SlotValue::Object(pairs)),
                b',' => {}
                _ => return Err(()),
            }
        }
    }
}

/// Extracted multi-slot field table partitioned by [`Role`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldTable {
    entries: Vec<(Role, String, SlotValue)>,
    biz: Vec<(String, SlotValue)>,
    /// Whole-body ciphertext bytes when `cipher_scope == "body"`.
    pub raw_cipher: Option<Vec<u8>>,
}

impl FieldTable {
    /// Returns extracted business fields.
    #[must_use]
    pub fn biz(&self) -> &[(String, SlotValue)] {
        &self.biz
    }

    /// Inserts or updates a field in the table.
    pub fn insert(&mut self, role: Role, name: impl Into<String>, value: SlotValue) {
        let name = name.into();
        if role == Role::Biz {
            upsert(&mut self.biz, name, value);
            return;
        }
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|(got, key, _)| *got == role && key == &name)
        {
            existing.2 = value;
        } else {
            self.entries.push((role, name, value));
        }
    }

    /// Retrieves a field by role and name.
    #[must_use]
    pub fn get(&self, role: Role, name: &str) -> Option<&SlotValue> {
        if role == Role::Biz {
            return self
                .biz
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value);
        }
        self.entries
            .iter()
            .find(|(got, key, _)| *got == role && key == name)
            .map(|(_, _, value)| value)
    }

    /// Retrieves the first field matching `role`.
    #[must_use]
    pub fn first(&self, role: Role) -> Option<&SlotValue> {
        self.entries
            .iter()
            .find(|(got, _, _)| *got == role)
            .map(|(_, _, value)| value)
    }

    /// Lists field names recorded under `role`.
    #[must_use]
    pub fn names(&self, role: Role) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(got, _, _)| *got == role)
            .map(|(_, name, _)| name.clone())
            .collect()
    }

    /// Removes control/auth/sign/cipher fields from `biz`.
    pub fn strip_named(&mut self, names: &[String]) {
        self.biz
            .retain(|(key, _)| !names.iter().any(|name| name == key));
    }
}

fn upsert(pairs: &mut Vec<(String, SlotValue)>, name: String, value: SlotValue) {
    if let Some(existing) = pairs.iter_mut().find(|(key, _)| key == &name) {
        existing.1 = value;
    } else {
        pairs.push((name, value));
    }
}
