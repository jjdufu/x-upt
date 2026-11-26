use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{Map, Number, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Auth,
    Cipher,
    Biz,
    Sign,
    Token,
    WrapKey,
    Timestamp,
    Nonce,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SlotValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<SlotValue>),
    Object(Vec<(String, SlotValue)>),
}

impl SlotValue {
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
                    let _ = write!(out, "{}:{}", json_string(key), value.canonical());
                }
                out.push('}');
                out
            }
        }
    }

    #[must_use]
    pub fn compact(&self) -> String {
        match self {
            Self::Object(pairs) => {
                let mut out = String::from("{");
                for (i, (key, value)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    let _ = write!(out, "{}:{}", json_string(key), value.compact());
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

    #[must_use]
    pub fn to_serde(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(v) => Value::Bool(*v),
            Self::Number(raw) => raw
                .parse::<Number>()
                .map(Value::Number)
                .unwrap_or_else(|_| Value::String(raw.clone())),
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

#[must_use]
pub fn json_string(raw: &str) -> String {
    serde_json::to_string(raw).unwrap_or_else(|_| format!("\"{raw}\""))
}

#[must_use]
pub fn biz_object(pairs: &[(String, SlotValue)]) -> SlotValue {
    SlotValue::Object(pairs.to_vec())
}

pub fn parse_json(input: &str) -> Result<SlotValue, ()> {
    let mut parser = Parser {
        bytes: input.as_bytes(),
        index: 0,
    };
    let value = parser.value()?;
    parser.skip_ws();
    if parser.index != parser.bytes.len() {
        return Err(());
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
        for byte in expected {
            if self.bump()? != *byte {
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
            Some(b'1'..=b'9') => {
                self.index += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.index += 1;
                }
            }
            _ => return Err(()),
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(());
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(());
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.index += 1;
            }
        }
        let raw = std::str::from_utf8(&self.bytes[start..self.index]).map_err(|_| ())?;
        Ok(SlotValue::Number(raw.to_string()))
    }

    fn string(&mut self) -> Result<String, ()> {
        if self.bump()? != b'"' {
            return Err(());
        }
        let mut out = String::new();
        loop {
            match self.bump()? {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.bump()?;
                    match esc {
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
                            let ch = char::from_u32(code).ok_or(())?;
                            out.push(ch);
                        }
                        _ => return Err(()),
                    }
                }
                b if b < 0x20 => return Err(()),
                _ => {
                    self.index -= 1;
                    let rest = std::str::from_utf8(&self.bytes[self.index..]).map_err(|_| ())?;
                    let ch = rest.chars().next().ok_or(())?;
                    out.push(ch);
                    self.index += ch.len_utf8();
                }
            }
        }
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
                b',' => continue,
                _ => return Err(()),
            }
        }
    }

    fn object(&mut self) -> Result<SlotValue, ()> {
        if self.bump()? != b'{' {
            return Err(());
        }
        self.skip_ws();
        let mut pairs = Vec::new();
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
            if let Some(existing) = pairs.iter_mut().find(|(name, _)| name == &key) {
                existing.1 = value;
            } else {
                pairs.push((key, value));
            }
            self.skip_ws();
            match self.bump()? {
                b'}' => return Ok(SlotValue::Object(pairs)),
                b',' => continue,
                _ => return Err(()),
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldTable {
    entries: Vec<(Role, String, SlotValue)>,
    biz: Vec<(String, SlotValue)>,
    pub raw_cipher: Option<Vec<u8>>,
}

impl FieldTable {
    #[must_use]
    pub fn biz(&self) -> &[(String, SlotValue)] {
        &self.biz
    }

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

    #[must_use]
    pub fn first(&self, role: Role) -> Option<&SlotValue> {
        self.entries
            .iter()
            .find(|(got, _, _)| *got == role)
            .map(|(_, _, value)| value)
    }

    #[must_use]
    pub fn names(&self, role: Role) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(got, _, _)| *got == role)
            .map(|(_, name, _)| name.clone())
            .collect()
    }

    pub fn strip_named(&mut self, names: &[String]) {
        self.biz
            .retain(|(key, _)| !names.iter().any(|name| name == key));
    }

    #[cfg(test)]
    #[must_use]
    pub fn contains_name(&self, name: &str) -> bool {
        self.biz.iter().any(|(key, _)| key == name)
            || self.entries.iter().any(|(_, key, _)| key == name)
    }
}

fn upsert(pairs: &mut Vec<(String, SlotValue)>, name: String, value: SlotValue) {
    if let Some(existing) = pairs.iter_mut().find(|(key, _)| key == &name) {
        existing.1 = value;
    } else {
        pairs.push((name, value));
    }
}

#[cfg(test)]
mod tests {
    use super::{SlotValue, parse_json};

    #[test]
    fn number_keeps_lexical_form() {
        let value = parse_json(r#"{"n":1.50,"z":0,"s":"1"}"#).unwrap();
        assert_eq!(value.canonical(), r#"{"n":1.50,"s":"1","z":0}"#);
        match value {
            SlotValue::Object(pairs) => {
                assert_eq!(pairs[0].1, SlotValue::Number("1.50".to_string()));
                assert_eq!(pairs[2].1, SlotValue::String("1".to_string()));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn null_bool_array_stay() {
        let value = parse_json(r#"{"a":null,"b":true,"c":[2,1]}"#).unwrap();
        assert_eq!(value.canonical(), r#"{"a":null,"b":true,"c":[2,1]}"#);
    }
}
