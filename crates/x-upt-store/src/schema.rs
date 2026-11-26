//! Idempotent append-only `SQLite` schema migrations and zero-preset default switch seeding.

use rusqlite::{Connection, params};
use x_upt_core::domain::{BusinessMethod, FrontName, FrontRecord, GlobalSettings};
use x_upt_core::error::AppError;

const MIGRATION_V1_SQL: &str = r"
CREATE TABLE IF NOT EXISTS global_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    latency_min_ms INTEGER NOT NULL,
    latency_max_ms INTEGER NOT NULL,
    token_expire_minutes INTEGER NOT NULL,
    token_cleanup_seconds INTEGER NOT NULL,
    token_max_per_client INTEGER NOT NULL,
    sign_window_seconds INTEGER NOT NULL,
    sign_nonce_ttl_seconds INTEGER NOT NULL,
    job_expire_seconds INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS business_methods (
    method TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS fronts (
    name TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 0,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    http2 INTEGER NOT NULL,
    h2c INTEGER NOT NULL,
    expire_minutes INTEGER NOT NULL,
    auth_json TEXT NOT NULL,
    tls_enabled INTEGER NOT NULL DEFAULT 0,
    tls_cert_pem TEXT,
    tls_key_pem TEXT,
    tls_client_ca_pem TEXT,
    ws_max_conn INTEGER NOT NULL,
    ws_ping_secs INTEGER NOT NULL,
    job_workers INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS images (
    id TEXT PRIMARY KEY,
    filename TEXT NOT NULL UNIQUE,
    scene TEXT NOT NULL,
    mime TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    size INTEGER NOT NULL,
    ahash TEXT NOT NULL,
    rel_path TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS posts (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS apps (
    client_id TEXT PRIMARY KEY,
    client_secret TEXT NOT NULL,
    name TEXT NOT NULL,
    rsa_public_pem TEXT,
    methods_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS envelope_secrets (
    name TEXT PRIMARY KEY,
    secret_kind TEXT NOT NULL,
    raw_bytes BLOB NOT NULL,
    fingerprint TEXT NOT NULL,
    public_material TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS envelope_profiles (
    front TEXT NOT NULL,
    ord INTEGER NOT NULL,
    spec_json TEXT NOT NULL,
    PRIMARY KEY (front, ord)
);
";

/// Applies connection PRAGMAs, runs pending migrations, and ensures default disabled switch rows exist.
///
/// # Errors
/// Returns [`AppError::Internal`] if `SQLite` DDL or seeding fails.
pub fn init_database(conn: &mut Connection, now_rfc3339: &str) -> Result<(), AppError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at TEXT NOT NULL
         );",
    )
    .map_err(db_err)?;

    let applied: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 1)",
            [],
            |row| row.get(0),
        )
        .map_err(db_err)?;

    if !applied {
        let tx = conn.transaction().map_err(db_err)?;
        tx.execute_batch(MIGRATION_V1_SQL).map_err(db_err)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (1, ?1, ?2)",
            params!["initial_stateful_schema", now_rfc3339],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
    }

    seed_default_switches(conn)
}

fn seed_default_switches(conn: &mut Connection) -> Result<(), AppError> {
    let tx = conn.transaction().map_err(db_err)?;
    let def = GlobalSettings::default();
    tx.execute(
        "INSERT OR IGNORE INTO global_settings (
            id, latency_min_ms, latency_max_ms, token_expire_minutes,
            token_cleanup_seconds, token_max_per_client, sign_window_seconds,
            sign_nonce_ttl_seconds, job_expire_seconds
        ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            i64::try_from(def.latency_min_ms).unwrap_or(20),
            i64::try_from(def.latency_max_ms).unwrap_or(100),
            i64::try_from(def.token_expire_minutes).unwrap_or(30),
            i64::try_from(def.token_cleanup_seconds).unwrap_or(300),
            i64::try_from(def.token_max_per_client).unwrap_or(0),
            i64::try_from(def.sign_window_seconds).unwrap_or(300),
            i64::try_from(def.sign_nonce_ttl_seconds).unwrap_or(600),
            i64::try_from(def.job_expire_seconds).unwrap_or(600),
        ],
    )
    .map_err(db_err)?;

    for method in BusinessMethod::all() {
        tx.execute(
            "INSERT OR IGNORE INTO business_methods (method, enabled) VALUES (?1, 0)",
            params![method.as_str()],
        )
        .map_err(db_err)?;
    }

    for front_name in FrontName::all() {
        let rec = FrontRecord::default_for(front_name);
        let auth_json = serde_json::to_string(&rec.auth).map_err(json_err)?;
        tx.execute(
            "INSERT OR IGNORE INTO fronts (
                name, enabled, host, port, http2, h2c, expire_minutes,
                auth_json, tls_enabled, tls_cert_pem, tls_key_pem, tls_client_ca_pem,
                ws_max_conn, ws_ping_secs, job_workers
            ) VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, 0, NULL, NULL, NULL, ?8, ?9, ?10)",
            params![
                rec.name.as_str(),
                rec.host,
                i64::from(rec.port),
                i64::from(rec.proto.http2),
                i64::from(rec.proto.h2c),
                i64::try_from(rec.expire_minutes).unwrap_or(30),
                auth_json,
                i64::try_from(rec.ws_max_conn).unwrap_or(1024),
                i64::try_from(rec.ws_ping_secs).unwrap_or(30),
                i64::try_from(rec.job_workers).unwrap_or(4),
            ],
        )
        .map_err(db_err)?;
    }

    tx.commit().map_err(db_err)
}

pub(crate) fn db_err(err: rusqlite::Error) -> AppError {
    AppError::Internal(format!("sqlite error: {err}"))
}

pub(crate) fn json_err(err: serde_json::Error) -> AppError {
    AppError::Internal(format!("json serialization error: {err}"))
}
