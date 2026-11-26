use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

use crate::types::gateway::{is_known_method, known_methods};
use crate::types::token::AuthedApp;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppToml {
    pub client_id: String,
    pub client_secret: String,
    pub name: Option<String>,
    pub methods: Vec<String>,
    #[serde(default)]
    pub rsa_public_pem: String,
}

#[derive(Debug, Clone)]
pub struct App {
    pub client_id: String,
    pub client_secret: String,
    pub methods: HashSet<String>,
    pub rsa_public_pem: Option<Vec<u8>>,
}

impl From<&App> for AuthedApp {
    fn from(app: &App) -> Self {
        Self {
            client_id: app.client_id.clone(),
            methods: app.methods.clone(),
        }
    }
}

pub struct AppRegistry {
    by_id: HashMap<String, App>,
}

impl AppRegistry {
    pub fn from_toml(entries: Vec<AppToml>, bin_dir: &Path) -> Result<Self, String> {
        if entries.is_empty() {
            return Err("config [[apps]] must contain at least one application".to_string());
        }

        let mut by_id = HashMap::new();
        for entry in entries {
            let AppToml {
                client_id: raw_id,
                client_secret,
                name,
                methods: method_list,
                rsa_public_pem: rsa_path,
            } = entry;
            let client_id = raw_id.trim().to_string();
            let id_len = client_id.chars().count();
            if !(1..=64).contains(&id_len) {
                return Err("app client_id must be 1-64 characters".to_string());
            }
            if let Some(ref name) = name {
                let nlen = name.chars().count();
                if nlen > 64 {
                    return Err(format!(
                        "app {client_id} name must be at most 64 characters"
                    ));
                }
            }
            if !(8..=128).contains(&client_secret.len()) {
                return Err(format!("app {client_id} client_secret must be 8-128 bytes"));
            }
            if method_list.is_empty() {
                return Err(format!("app {client_id} methods must not be empty"));
            }

            let mut methods = HashSet::new();
            for method in method_list {
                if !is_known_method(&method) {
                    return Err(format!(
                        "app {client_id} has unknown method {method}; known: {}",
                        known_methods().join(", ")
                    ));
                }
                methods.insert(method);
            }

            if by_id.contains_key(&client_id) {
                return Err(format!("duplicate app client_id {client_id}"));
            }

            let rsa_public_pem =
                load_optional_pem(bin_dir, &rsa_path, &client_id, "rsa_public_pem")?;

            by_id.insert(
                client_id.clone(),
                App {
                    client_id,
                    client_secret,
                    methods,
                    rsa_public_pem,
                },
            );
        }

        Ok(Self { by_id })
    }

    #[must_use]
    pub fn get(&self, client_id: &str) -> Option<&App> {
        self.by_id.get(client_id)
    }

    #[must_use]
    pub fn find_by_secret(&self, secret: &str) -> Option<&App> {
        self.by_id
            .values()
            .find(|app| secrets_equal(&app.client_secret, secret))
    }

    pub fn iter(&self) -> impl Iterator<Item = &App> {
        self.by_id.values()
    }
}

fn load_optional_pem(
    bin_dir: &Path,
    raw: &str,
    client_id: &str,
    field: &str,
) -> Result<Option<Vec<u8>>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let mut candidates = Vec::new();
    if Path::new(raw).is_absolute() {
        candidates.push(Path::new(raw).to_path_buf());
    } else {
        candidates.push(bin_dir.join(raw));
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join(raw));
        }
    }
    let mut last_err = String::new();
    for path in candidates {
        match std::fs::read(&path) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) => last_err = format!("app {client_id} {field} file {}: {e}", path.display()),
        }
    }
    Err(last_err)
}

#[must_use]
pub fn secrets_equal(left: &str, right: &str) -> bool {
    secrets_equal_bytes(left.as_bytes(), right.as_bytes())
}

#[must_use]
pub fn secrets_equal_bytes(left: &[u8], right: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if left.len() != right.len() {
        return false;
    }
    left.ct_eq(right).into()
}
