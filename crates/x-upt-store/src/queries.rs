//! Low-level `SQLite` query helpers for loading and persisting domain records.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use x_upt_core::domain::{
    AppRecord, AuthMethod, BusinessMethod, EnvelopeProfileSpec, EnvelopeSecretRecord, FrontName,
    FrontProtocolFlags, FrontRecord, FrontTlsConfig, GlobalSettings, MethodsState, SecretKind,
};
use x_upt_core::error::AppError;
use x_upt_core::imgs::{ImageMeta, PhotoScene};
use x_upt_core::services::Post;

use crate::schema::{db_err, json_err};

pub(crate) fn load_settings(conn: &Connection) -> Result<GlobalSettings, AppError> {
    conn.query_row(
        "SELECT latency_min_ms, latency_max_ms, token_expire_minutes, token_cleanup_seconds,
                token_max_per_client, sign_window_seconds, sign_nonce_ttl_seconds, job_expire_seconds
         FROM global_settings WHERE id = 1",
        [],
        |row| {
            Ok(GlobalSettings {
                latency_min_ms: i64_to_u64(row.get(0)?),
                latency_max_ms: i64_to_u64(row.get(1)?),
                token_expire_minutes: i64_to_u64(row.get(2)?),
                token_cleanup_seconds: i64_to_u64(row.get(3)?),
                token_max_per_client: i64_to_u64(row.get(4)?),
                sign_window_seconds: i64_to_u64(row.get(5)?),
                sign_nonce_ttl_seconds: i64_to_u64(row.get(6)?),
                job_expire_seconds: i64_to_u64(row.get(7)?),
            })
        },
    )
    .map_err(db_err)
}

pub(crate) fn save_settings(conn: &Connection, s: &GlobalSettings) -> Result<(), AppError> {
    conn.execute(
        "UPDATE global_settings SET
            latency_min_ms = ?1, latency_max_ms = ?2, token_expire_minutes = ?3,
            token_cleanup_seconds = ?4, token_max_per_client = ?5, sign_window_seconds = ?6,
            sign_nonce_ttl_seconds = ?7, job_expire_seconds = ?8
         WHERE id = 1",
        params![
            u64_to_i64(s.latency_min_ms),
            u64_to_i64(s.latency_max_ms),
            u64_to_i64(s.token_expire_minutes),
            u64_to_i64(s.token_cleanup_seconds),
            u64_to_i64(s.token_max_per_client),
            u64_to_i64(s.sign_window_seconds),
            u64_to_i64(s.sign_nonce_ttl_seconds),
            u64_to_i64(s.job_expire_seconds),
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

pub(crate) fn load_methods_state(conn: &Connection) -> Result<MethodsState, AppError> {
    let mut stmt = conn
        .prepare("SELECT method, enabled FROM business_methods")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            let m: String = row.get(0)?;
            let en: i64 = row.get(1)?;
            Ok((m, en != 0))
        })
        .map_err(db_err)?;
    let mut state = MethodsState::default();
    for item in rows {
        let (name, enabled) = item.map_err(db_err)?;
        if let Some(bm) = BusinessMethod::parse(&name) {
            state.set(bm, enabled);
        }
    }
    Ok(state)
}

pub(crate) fn load_fronts(conn: &Connection) -> Result<Vec<FrontRecord>, AppError> {
    let mut map = HashMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT name, enabled, host, port, http2, h2c, expire_minutes, auth_json,
                    tls_enabled, tls_cert_pem, tls_key_pem, tls_client_ca_pem,
                    ws_max_conn, ws_ping_secs, job_workers
             FROM fronts",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            let raw_name: String = row.get(0)?;
            let auth_str: String = row.get(7)?;
            Ok((
                raw_name,
                row.get::<_, i64>(1)? != 0,
                row.get::<_, String>(2)?,
                u16::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                row.get::<_, i64>(4)? != 0,
                row.get::<_, i64>(5)? != 0,
                i64_to_u64(row.get(6)?),
                auth_str,
                row.get::<_, i64>(8)? != 0,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                usize::try_from(row.get::<_, i64>(12)?).unwrap_or(1024),
                i64_to_u64(row.get(13)?),
                usize::try_from(row.get::<_, i64>(14)?).unwrap_or(4),
            ))
        })
        .map_err(db_err)?;
    for item in rows {
        let tup = item.map_err(db_err)?;
        let Some(name) = FrontName::parse(&tup.0) else {
            continue;
        };
        let auth: Vec<AuthMethod> = serde_json::from_str(&tup.7).unwrap_or_default();
        map.insert(
            name,
            FrontRecord {
                name,
                enabled: tup.1,
                host: tup.2,
                port: tup.3,
                proto: FrontProtocolFlags {
                    http2: tup.4,
                    h2c: tup.5,
                },
                expire_minutes: tup.6,
                auth,
                tls: FrontTlsConfig {
                    enabled: tup.8,
                    cert_pem: tup.9,
                    key_pem: tup.10,
                    client_ca_pem: tup.11,
                },
                ws_max_conn: tup.12,
                ws_ping_secs: tup.13,
                job_workers: tup.14,
            },
        );
    }
    Ok(FrontName::all()
        .into_iter()
        .map(|n| {
            map.remove(&n)
                .unwrap_or_else(|| FrontRecord::default_for(n))
        })
        .collect())
}

pub(crate) fn save_front(conn: &Connection, f: &FrontRecord) -> Result<(), AppError> {
    let auth_json = serde_json::to_string(&f.auth).map_err(json_err)?;
    conn.execute(
        "UPDATE fronts SET
            enabled = ?1, host = ?2, port = ?3, http2 = ?4, h2c = ?5,
            expire_minutes = ?6, auth_json = ?7, tls_enabled = ?8,
            tls_cert_pem = ?9, tls_key_pem = ?10, tls_client_ca_pem = ?11,
            ws_max_conn = ?12, ws_ping_secs = ?13, job_workers = ?14
         WHERE name = ?15",
        params![
            i64::from(f.enabled),
            f.host,
            i64::from(f.port),
            i64::from(f.proto.http2),
            i64::from(f.proto.h2c),
            u64_to_i64(f.expire_minutes),
            auth_json,
            i64::from(f.tls.enabled),
            f.tls.cert_pem,
            f.tls.key_pem,
            f.tls.client_ca_pem,
            usize_to_i64(f.ws_max_conn),
            u64_to_i64(f.ws_ping_secs),
            usize_to_i64(f.job_workers),
            f.name.as_str(),
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

pub(crate) fn load_images_with_paths(
    conn: &Connection,
) -> Result<Vec<(ImageMeta, String)>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, filename, scene, mime, width, height, size, ahash, rel_path, created_at
             FROM images ORDER BY filename ASC, id ASC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            let scene_str: String = row.get(2)?;
            let scene = PhotoScene::parse(&scene_str).unwrap_or(PhotoScene::ArticleCover);
            Ok((
                ImageMeta {
                    id: row.get(0)?,
                    filename: row.get(1)?,
                    scene,
                    mime: row.get(3)?,
                    width: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                    height: u32::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                    size: i64_to_u64(row.get(6)?),
                    ahash: row.get(7)?,
                    created_at: row.get(9)?,
                },
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

pub(crate) fn find_image_by_id(
    conn: &Connection,
    id: &str,
) -> Result<Option<(ImageMeta, String)>, AppError> {
    conn.query_row(
        "SELECT id, filename, scene, mime, width, height, size, ahash, rel_path, created_at
         FROM images WHERE id = ?1",
        params![id],
        |row| {
            let scene_str: String = row.get(2)?;
            let scene = PhotoScene::parse(&scene_str).unwrap_or(PhotoScene::ArticleCover);
            Ok((
                ImageMeta {
                    id: row.get(0)?,
                    filename: row.get(1)?,
                    scene,
                    mime: row.get(3)?,
                    width: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                    height: u32::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
                    size: i64_to_u64(row.get(6)?),
                    ahash: row.get(7)?,
                    created_at: row.get(9)?,
                },
                row.get::<_, String>(8)?,
            ))
        },
    )
    .optional()
    .map_err(db_err)
}

pub(crate) fn load_posts(conn: &Connection) -> Result<Vec<Post>, AppError> {
    let mut stmt = conn
        .prepare("SELECT id, user_id, title, body FROM posts ORDER BY id ASC")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Post {
                id: i64_to_u64(row.get(0)?),
                user_id: i64_to_u64(row.get(1)?),
                title: row.get(2)?,
                body: row.get(3)?,
            })
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

pub(crate) fn count_posts(conn: &Connection) -> Result<usize, AppError> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))
        .map_err(db_err)?;
    Ok(usize::try_from(n).unwrap_or(0))
}

pub(crate) fn load_apps(conn: &Connection) -> Result<Vec<AppRecord>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT client_id, client_secret, name, rsa_public_pem, methods_json, created_at
             FROM apps ORDER BY client_id ASC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            let methods_str: String = row.get(4)?;
            let methods: Vec<String> = serde_json::from_str(&methods_str).unwrap_or_default();
            Ok(AppRecord {
                client_id: row.get(0)?,
                client_secret: row.get(1)?,
                name: row.get(2)?,
                rsa_public_pem: row.get(3)?,
                methods,
                created_at: row.get(5)?,
            })
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

pub(crate) fn load_secrets(conn: &Connection) -> Result<Vec<EnvelopeSecretRecord>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT name, secret_kind, raw_bytes, fingerprint, public_material, updated_at
             FROM envelope_secrets ORDER BY name ASC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            let kind_str: String = row.get(1)?;
            let kind = SecretKind::parse(&kind_str).unwrap_or(SecretKind::Text);
            Ok(EnvelopeSecretRecord {
                name: row.get(0)?,
                kind,
                raw_bytes: row.get(2)?,
                fingerprint: row.get(3)?,
                public_material: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

pub(crate) fn load_profiles_map(
    conn: &Connection,
) -> Result<HashMap<FrontName, Vec<EnvelopeProfileSpec>>, AppError> {
    let mut out: HashMap<FrontName, Vec<EnvelopeProfileSpec>> = HashMap::new();
    let mut stmt = conn
        .prepare("SELECT front, spec_json FROM envelope_profiles ORDER BY front ASC, ord ASC")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_err)?;
    for item in rows {
        let (front_str, spec_json) = item.map_err(db_err)?;
        let Some(front) = FrontName::parse(&front_str) else {
            continue;
        };
        let spec: EnvelopeProfileSpec = serde_json::from_str(&spec_json).map_err(json_err)?;
        out.entry(front).or_default().push(spec);
    }
    Ok(out)
}

pub(crate) fn i64_to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

pub(crate) fn u64_to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

pub(crate) fn usize_to_i64(v: usize) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}
