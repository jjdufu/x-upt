use sha2::{Digest, Sha256};

use crate::envelope::table::{SlotValue, biz_object};

#[derive(Clone, Debug, PartialEq)]
pub struct MockBody {
    pub ok: bool,
    pub reason: String,
    pub hit: bool,
    pub fields: Vec<(String, SlotValue)>,
    pub digest: String,
}

impl MockBody {
    #[must_use]
    pub fn success(biz: &[(String, SlotValue)]) -> Self {
        let hit = !biz.is_empty();
        let fields = if hit { biz.to_vec() } else { Vec::new() };
        Self {
            ok: true,
            reason: String::new(),
            hit,
            digest: digest_of(&fields),
            fields,
        }
    }

    #[must_use]
    pub fn failure(reason: &str) -> Self {
        Self {
            ok: false,
            reason: reason.to_string(),
            hit: false,
            fields: Vec::new(),
            digest: String::new(),
        }
    }

    #[must_use]
    pub fn to_value(&self) -> SlotValue {
        SlotValue::Object(vec![
            ("ok".to_string(), SlotValue::Bool(self.ok)),
            ("reason".to_string(), SlotValue::String(self.reason.clone())),
            ("hit".to_string(), SlotValue::Bool(self.hit)),
            ("fields".to_string(), biz_object(&self.fields)),
            ("digest".to_string(), SlotValue::String(self.digest.clone())),
        ])
    }
}

#[must_use]
pub fn digest_of(biz: &[(String, SlotValue)]) -> String {
    let text = biz_object(biz).canonical();
    hex_sha256(text.as_bytes())
}

#[must_use]
pub fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{MockBody, digest_of, hex_sha256};
    use crate::envelope::table::SlotValue;

    #[test]
    fn empty_biz_misses_and_hashes_empty_object() {
        let body = MockBody::success(&[]);
        assert!(!body.hit);
        assert!(body.fields.is_empty());
        assert_eq!(body.digest, hex_sha256(b"{}"));
        assert!(body.to_value().compact().contains(r#""hit":false"#));
    }

    #[test]
    fn same_biz_same_digest() {
        let biz = vec![
            ("b".to_string(), SlotValue::String("x".to_string())),
            ("a".to_string(), SlotValue::Number("1.50".to_string())),
        ];
        assert_eq!(digest_of(&biz), digest_of(&biz));
        let again = MockBody::success(&biz);
        assert!(again.hit);
        assert_eq!(again.digest, digest_of(&biz));
        assert_eq!(digest_of(&biz), hex_sha256(br#"{"a":1.50,"b":"x"}"#));
    }

    #[test]
    fn string_one_is_not_a_number() {
        let biz = vec![("n".to_string(), SlotValue::String("1".to_string()))];
        let body = MockBody::success(&biz);
        assert_eq!(body.fields[0].1, SlotValue::String("1".to_string()));
        assert!(body.to_value().compact().contains(r#""n":"1""#));
    }
}
