use crate::auth::nonce::NonceStore;
use crate::config::SignWindow;
use crate::envelope::canon;
use crate::envelope::crypt::{self, CryptError};
use crate::envelope::profile::Profile;
use crate::envelope::table::{FieldTable, Role, SlotValue};
use crate::envelope::token_store::EnvelopeTokenStore;
use crate::service::apps::secrets_equal_bytes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthFail {
    Token,
    Sign,
    Credential,
    Decrypt,
    Shape,
}

pub fn check_credentials(profile: &Profile, table: &FieldTable) -> Result<(), AuthFail> {
    if profile.secrets.is_empty() {
        return Err(AuthFail::Credential);
    }
    for (name, expect) in &profile.secrets {
        let got = table
            .get(Role::Auth, name)
            .map(SlotValue::text)
            .ok_or(AuthFail::Shape)?;
        if !secrets_equal_bytes(got.as_bytes(), expect.as_bytes()) {
            return Err(AuthFail::Credential);
        }
    }
    Ok(())
}

pub fn check_token(
    profile: &Profile,
    table: &FieldTable,
    store: &EnvelopeTokenStore,
) -> Result<(), AuthFail> {
    let raw = table
        .first(Role::Token)
        .map(SlotValue::text)
        .ok_or(AuthFail::Shape)?;
    let token = strip_prefix(profile, &raw)?;
    if store.contains(token) {
        Ok(())
    } else {
        Err(AuthFail::Token)
    }
}

fn strip_prefix<'a>(profile: &Profile, raw: &'a str) -> Result<&'a str, AuthFail> {
    let Some(prefix) = profile.token_prefix.as_deref() else {
        return Ok(raw);
    };
    raw.strip_prefix(prefix).ok_or(AuthFail::Token)
}

pub fn check_sign(
    profile: &Profile,
    table: &FieldTable,
    nonces: &NonceStore,
    window: &SignWindow,
    plain_override: Option<&[u8]>,
) -> Result<(), AuthFail> {
    check_timestamp(profile, table, window)?;
    check_nonce(profile, table, nonces)?;
    let provided = table
        .first(Role::Sign)
        .map(SlotValue::text)
        .ok_or(AuthFail::Shape)?;
    let bytes = canon::material(profile, table, plain_override);
    match crypt::verify(profile, &bytes, &provided) {
        Ok(true) => Ok(()),
        Ok(false) | Err(CryptError::Mismatch) => Err(AuthFail::Sign),
        Err(_) => Err(AuthFail::Sign),
    }
}

fn check_timestamp(
    profile: &Profile,
    table: &FieldTable,
    window: &SignWindow,
) -> Result<(), AuthFail> {
    let Some(value) = table.first(Role::Timestamp) else {
        return Ok(());
    };
    let ts = value.text().parse::<i64>().map_err(|_| AuthFail::Sign)?;
    let now = chrono::Utc::now().timestamp();
    let span = i64::try_from(window.window_seconds).unwrap_or(i64::MAX);
    if (now - ts).abs() > span {
        return Err(AuthFail::Sign);
    }
    let _ = profile;
    Ok(())
}

fn check_nonce(profile: &Profile, table: &FieldTable, nonces: &NonceStore) -> Result<(), AuthFail> {
    let Some(value) = table.first(Role::Nonce) else {
        let _ = profile;
        return Ok(());
    };
    let nonce = value.text();
    if nonce.is_empty() || !nonces.insert(&nonce) {
        return Err(AuthFail::Sign);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_credentials, check_token, strip_prefix};
    use crate::envelope::profile::Profile;
    use crate::envelope::table::{FieldTable, Role, SlotValue};
    use crate::envelope::token_store::EnvelopeTokenStore;

    #[test]
    fn prefixes_are_case_sensitive() {
        let mut lower = Profile::test_call_empty();
        lower.token_prefix = Some("bearer ".to_string());
        let mut upper = Profile::test_call_empty();
        upper.token_prefix = Some("Bearer ".to_string());
        assert!(strip_prefix(&upper, "Bearer abc").is_ok());
        assert!(strip_prefix(&upper, "bearer abc").is_err());
        assert!(strip_prefix(&lower, "bearer abc").is_ok());
        assert!(strip_prefix(&lower, "Bearer abc").is_err());
    }

    #[tokio::test]
    async fn credential_and_token() {
        let mut profile = Profile::test_call_empty();
        profile
            .secrets
            .insert("username".to_string(), "alice".to_string());
        let mut table = FieldTable::default();
        table.insert(
            Role::Auth,
            "username",
            SlotValue::String("alice".to_string()),
        );
        assert!(check_credentials(&profile, &table).is_ok());
        table.insert(Role::Auth, "username", SlotValue::String("bob".to_string()));
        assert!(check_credentials(&profile, &table).is_err());

        let store = EnvelopeTokenStore::new(30, 300);
        let other = EnvelopeTokenStore::new(30, 300);
        let (token, _) = store.issue();
        profile.token_prefix = Some("Bearer ".to_string());
        let mut tokens = FieldTable::default();
        tokens.insert(
            Role::Token,
            "authorization",
            SlotValue::String(format!("Bearer {token}")),
        );
        assert!(check_token(&profile, &tokens, &store).is_ok());
        assert!(check_token(&profile, &tokens, &other).is_err());
    }
}
