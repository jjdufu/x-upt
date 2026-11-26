//! Precondition-guarded switch transitions, front configuration updates, and status evaluation.

use std::collections::HashSet;

use rusqlite::{Connection, params};
use x_upt_core::domain::{
    AuthMethod, BusinessMethod, FrontName, FrontProtocolFlags, FrontRecord, FrontTlsConfig,
    GlobalSettings,
};
use x_upt_core::error::AppError;
use x_upt_core::imgs::ImageMeta;
use x_upt_core::preconditions::{
    FrontEvalInput, MethodStatus, SceneCounts, evaluate_methods, guard_disable_method,
    missing_for_front, missing_for_method,
};
use x_upt_crypto::profile::compile_profiles;

use crate::dto::{FrontStatusView, FrontUpdatePayload, SystemCountsView, SystemStatusView};
use crate::ops_envelopes::secrets_by_name;
use crate::queries::{
    count_posts, load_apps, load_fronts, load_images_with_paths, load_methods_state,
    load_profiles_map, load_settings, save_front, save_settings,
};
use crate::schema::db_err;

/// Updates global simulation parameters after range checks.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] on invalid ranges.
pub fn update_global_settings(
    conn: &mut Connection,
    next: GlobalSettings,
) -> Result<GlobalSettings, AppError> {
    if next.latency_min_ms > next.latency_max_ms || next.latency_max_ms > 60_000 {
        return Err(AppError::InvalidInput(
            "模拟延迟区间须满足 0 <= latency_min_ms <= latency_max_ms <= 60000".to_string(),
        ));
    }
    if next.token_expire_minutes == 0
        || next.token_cleanup_seconds == 0
        || next.sign_window_seconds == 0
        || next.sign_nonce_ttl_seconds == 0
        || next.job_expire_seconds == 0
    {
        return Err(AppError::InvalidInput(
            "过期时间与签名时间窗参数均须大于 0".to_string(),
        ));
    }
    save_settings(conn, &next)?;
    Ok(next)
}

/// Enables or disables a business method inside a `SQLite` transaction with full precondition checks.
///
/// # Errors
/// Returns [`AppError::Conflict`] (`409`) when preconditions are missing or active core fronts depend on it.
pub fn set_business_method(
    conn: &mut Connection,
    method: BusinessMethod,
    enable: bool,
) -> Result<MethodStatus, AppError> {
    let tx = conn.transaction().map_err(db_err)?;
    let mut state = load_methods_state(&tx)?;
    let images: Vec<ImageMeta> = load_images_with_paths(&tx)?
        .into_iter()
        .map(|(m, _)| m)
        .collect();
    let scenes = SceneCounts::from_images(&images);
    let posts_cnt = count_posts(&tx)?;

    if enable {
        let missing = missing_for_method(method, scenes, posts_cnt);
        if !missing.is_empty() {
            return Err(AppError::precondition(
                format!("业务子接口 {} 未满足开启前置条件", method.as_str()),
                missing,
            ));
        }
    } else if state.get(method) {
        let fronts = load_fronts(&tx)?;
        let apps = load_apps(&tx)?;
        guard_disable_method(method, state, &fronts, &apps)?;
    }

    state.set(method, enable);
    tx.execute(
        "UPDATE business_methods SET enabled = ?1 WHERE method = ?2",
        params![i64::from(enable), method.as_str()],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;

    evaluate_methods(state, scenes, posts_cnt)
        .into_iter()
        .find(|m| m.method == method.as_str())
        .ok_or_else(|| AppError::Internal("method status lookup failed".to_string()))
}

/// Updates listener, TLS, authentication, and worker parameters for `front_name`.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] for bad fields or [`AppError::Conflict`] if an active front becomes invalid.
pub fn update_front_record(
    conn: &mut Connection,
    admin_port: u16,
    front_name: FrontName,
    payload: FrontUpdatePayload,
) -> Result<FrontRecord, AppError> {
    let host = payload.host.trim().to_string();
    if host.is_empty() || payload.port == 0 {
        return Err(AppError::InvalidInput(
            "监听地址不能为空且端口须在 1–65535 之间".to_string(),
        ));
    }
    if payload.expire_minutes == 0
        || payload.ws_max_conn == 0
        || payload.ws_ping_secs == 0
        || payload.job_workers == 0
    {
        return Err(AppError::InvalidInput(
            "过期时间、连接上限、心跳秒数与异步工作协程数均须大于 0".to_string(),
        ));
    }

    let mut parsed_auth = Vec::new();
    for raw in &payload.auth {
        let m = AuthMethod::parse(raw).map_err(AppError::InvalidInput)?;
        if !front_name.supported_auth().contains(&m) {
            return Err(AppError::InvalidInput(format!(
                "接入面 {} 不支持鉴权方式 {}",
                front_name.as_str(),
                m.as_str()
            )));
        }
        if !parsed_auth.contains(&m) {
            parsed_auth.push(m);
        }
    }

    let tx = conn.transaction().map_err(db_err)?;
    let mut fronts = load_fronts(&tx)?;
    let idx = fronts
        .iter()
        .position(|f| f.name == front_name)
        .ok_or_else(|| AppError::NotFound(format!("未找到接入面: {}", front_name.as_str())))?;
    let current = fronts
        .get(idx)
        .cloned()
        .ok_or_else(|| AppError::Internal("front index out of bounds".to_string()))?;

    let updated = FrontRecord {
        name: front_name,
        enabled: current.enabled,
        host,
        port: payload.port,
        proto: FrontProtocolFlags {
            http2: payload.http2,
            h2c: payload.h2c,
        },
        expire_minutes: payload.expire_minutes,
        auth: parsed_auth,
        tls: FrontTlsConfig {
            enabled: payload.tls_enabled,
            cert_pem: merge_optional_pem(payload.tls_cert_pem, current.tls.cert_pem),
            key_pem: merge_optional_pem(payload.tls_key_pem, current.tls.key_pem),
            client_ca_pem: merge_optional_pem(payload.tls_client_ca_pem, current.tls.client_ca_pem),
        },
        ws_max_conn: payload.ws_max_conn,
        ws_ping_secs: payload.ws_ping_secs,
        job_workers: payload.job_workers,
    };

    if let Some(slot) = fronts.get_mut(idx) {
        *slot = updated.clone();
    }

    if updated.enabled {
        let missing = eval_front_missing(&tx, admin_port, &updated, &fronts)?;
        if !missing.is_empty() {
            return Err(AppError::precondition(
                format!(
                    "更新后的配置不满足已启用接入面 {} 的运行条件",
                    front_name.as_str()
                ),
                missing,
            ));
        }
    }

    save_front(&tx, &updated)?;
    tx.commit().map_err(db_err)?;
    Ok(updated)
}

fn merge_optional_pem(incoming: Option<String>, existing: Option<String>) -> Option<String> {
    match incoming {
        None => existing,
        Some(s) if s.trim().is_empty() => None,
        Some(s) => Some(s),
    }
}

/// Verifies all preconditions for enabling `front_name` and returns its [`FrontRecord`] ready to bind.
///
/// # Errors
/// Returns [`AppError::Conflict`] (`409`) when any precondition is unmet.
pub fn check_front_can_enable(
    conn: &Connection,
    admin_port: u16,
    front_name: FrontName,
) -> Result<FrontRecord, AppError> {
    let fronts = load_fronts(conn)?;
    let target = fronts
        .iter()
        .find(|f| f.name == front_name)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("未找到接入面: {}", front_name.as_str())))?;
    let missing = eval_front_missing(conn, admin_port, &target, &fronts)?;
    if !missing.is_empty() {
        return Err(AppError::precondition(
            format!("接入面 {} 未满足开启前置条件", front_name.as_str()),
            missing,
        ));
    }
    Ok(target)
}

/// Persists `enabled` flag for `front_name` after supervisor listener bind/stop succeeds.
///
/// # Errors
/// Returns [`AppError::Internal`] on `SQLite` write failure.
pub fn persist_front_enabled(
    conn: &Connection,
    front_name: FrontName,
    enabled: bool,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE fronts SET enabled = ?1 WHERE name = ?2",
        params![i64::from(enabled), front_name.as_str()],
    )
    .map_err(db_err)?;
    Ok(())
}

fn eval_front_missing(
    conn: &Connection,
    admin_port: u16,
    target: &FrontRecord,
    all_fronts: &[FrontRecord],
) -> Result<Vec<String>, AppError> {
    let methods = load_methods_state(conn)?;
    let apps = load_apps(conn)?;
    let profiles_map = load_profiles_map(conn)?;
    let secrets = secrets_by_name(conn)?;
    let specs = profiles_map
        .get(&target.name)
        .map_or(&[][..], Vec::as_slice);
    let mut env_errs = Vec::new();
    if target.name.is_envelope()
        && !specs.is_empty()
        && let Err(e) = compile_profiles(target.name, specs, &secrets)
    {
        env_errs.push(e);
    }
    let ctx = FrontEvalInput {
        admin_port,
        all_fronts,
        methods,
        apps: &apps,
        profiles: specs,
        envelope_errors: &env_errs,
    };
    Ok(missing_for_front(target, &ctx))
}

/// Computes full [`SystemStatusView`] across methods, fronts, and stored assets.
///
/// # Errors
/// Returns [`AppError::Internal`] on `SQLite` query failure.
pub fn compute_system_status(
    conn: &Connection,
    version: &str,
    admin_listen: &str,
    admin_port: u16,
    running: &HashSet<FrontName>,
) -> Result<SystemStatusView, AppError> {
    let settings = load_settings(conn)?;
    let methods_state = load_methods_state(conn)?;
    let images: Vec<ImageMeta> = load_images_with_paths(conn)?
        .into_iter()
        .map(|(m, _)| m)
        .collect();
    let scenes = SceneCounts::from_images(&images);
    let posts_cnt = count_posts(conn)?;
    let apps = load_apps(conn)?;
    let secrets = secrets_by_name(conn)?;
    let profiles_map = load_profiles_map(conn)?;
    let fronts = load_fronts(conn)?;

    let methods = evaluate_methods(methods_state, scenes, posts_cnt);
    let mut front_views = Vec::with_capacity(fronts.len());
    let mut total_profiles = 0usize;
    for p in profiles_map.values() {
        total_profiles += p.len();
    }

    for f in &fronts {
        let specs = profiles_map.get(&f.name).map_or(&[][..], Vec::as_slice);
        let mut env_errs = Vec::new();
        if f.name.is_envelope()
            && !specs.is_empty()
            && let Err(e) = compile_profiles(f.name, specs, &secrets)
        {
            env_errs.push(e);
        }
        let ctx = FrontEvalInput {
            admin_port,
            all_fronts: &fronts,
            methods: methods_state,
            apps: &apps,
            profiles: specs,
            envelope_errors: &env_errs,
        };
        let missing = missing_for_front(f, &ctx);
        front_views.push(to_front_view(
            f,
            running.contains(&f.name),
            specs.len(),
            missing,
        ));
    }

    Ok(SystemStatusView {
        version: version.to_string(),
        admin_listen: admin_listen.to_string(),
        counts: SystemCountsView {
            images: images.len(),
            images_by_scene: scenes,
            posts: posts_cnt,
            apps: apps.len(),
            envelope_secrets: secrets.len(),
            envelope_profiles: total_profiles,
        },
        settings,
        methods,
        fronts: front_views,
    })
}

pub(crate) fn to_front_view(
    f: &FrontRecord,
    is_running: bool,
    profile_count: usize,
    missing: Vec<String>,
) -> FrontStatusView {
    FrontStatusView {
        name: f.name.as_str().to_string(),
        category: f.name.category(),
        protocol: f.name.protocol_label(),
        enabled: f.enabled,
        running: is_running,
        host: f.host.clone(),
        port: f.port,
        http2: f.proto.http2,
        h2c: f.proto.h2c,
        expire_minutes: f.expire_minutes,
        auth: f.auth.iter().map(|a| a.as_str().to_string()).collect(),
        supported_auth: f
            .name
            .supported_auth()
            .iter()
            .map(|a| a.as_str().to_string())
            .collect(),
        tls_enabled: f.tls.enabled,
        has_tls_cert: f
            .tls
            .cert_pem
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty()),
        has_tls_key: f
            .tls
            .key_pem
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty()),
        has_tls_client_ca: f
            .tls
            .client_ca_pem
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty()),
        ws_max_conn: f.ws_max_conn,
        ws_ping_secs: f.ws_ping_secs,
        job_workers: f.job_workers,
        profile_count,
        can_enable: missing.is_empty(),
        missing_preconditions: missing,
    }
}
