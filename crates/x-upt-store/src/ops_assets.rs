//! Asset and credential CRUD operations (`images`, `posts`, `apps`) with active-dependency locks.

use std::fs;
use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;
use x_upt_core::domain::{
    AppRecord, AuthMethod, FrontName, FrontRecord, MethodsState, is_known_method,
};
use x_upt_core::error::AppError;
use x_upt_core::imgs::{ImageMeta, PhotoScene, average_hash, detect_mime, load_image_bytes};
use x_upt_core::preconditions::{guard_delete_image, guard_delete_post};
use x_upt_core::services::Post;

use crate::dto::{AppMetaView, AppUpsertPayload, PostInput, PostsPageView};
use crate::queries::{
    count_posts, find_image_by_id, i64_to_u64, load_apps, load_fronts, load_images_with_paths,
    load_methods_state, u64_to_i64,
};
use crate::schema::{db_err, json_err};

/// Validates, decodes, hashes, writes to `<data_dir>/images/<id>.bin`, and records an image.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] for invalid filename/bytes, or [`AppError::Internal`] on IO/SQLite errors.
pub fn import_image(
    conn: &mut Connection,
    data_dir: &Path,
    scene: PhotoScene,
    raw_filename: &str,
    bytes: &[u8],
) -> Result<ImageMeta, AppError> {
    let filename = sanitize_filename(raw_filename)?;
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return Err(AppError::InvalidInput(
            "单张基准图片大小须在 1 字节到 8 MiB 之间".to_string(),
        ));
    }
    let decoded = load_image_bytes(bytes).map_err(AppError::InvalidInput)?;
    let hash_u64 = average_hash(&decoded)
        .ok_or_else(|| AppError::InvalidInput("无法计算图片感知哈希".to_string()))?;
    let ahash = format!("{hash_u64:016x}");
    let mime = detect_mime(bytes).to_string();
    let width = decoded.width();
    let height = decoded.height();
    let size = u64::try_from(bytes.len()).unwrap_or(0);
    let now = Utc::now().to_rfc3339();

    let images_dir = data_dir.join("images");
    fs::create_dir_all(&images_dir)
        .map_err(|e| AppError::Internal(format!("create images dir failed: {e}")))?;

    let tx = conn.transaction().map_err(db_err)?;
    let (id, rel_path) = resolve_image_slot(&tx, &filename, scene)?;

    let full_path = images_dir.join(&rel_path);
    fs::write(&full_path, bytes)
        .map_err(|e| AppError::Internal(format!("write image bytes failed: {e}")))?;

    tx.execute(
        "INSERT INTO images (id, filename, scene, mime, width, height, size, ahash, rel_path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(filename) DO UPDATE SET
            scene = excluded.scene,
            mime = excluded.mime,
            width = excluded.width,
            height = excluded.height,
            size = excluded.size,
            ahash = excluded.ahash,
            rel_path = excluded.rel_path,
            created_at = excluded.created_at",
        params![
            id,
            filename,
            scene.as_str(),
            mime,
            i64::from(width),
            i64::from(height),
            u64_to_i64(size),
            ahash,
            rel_path,
            now,
        ],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;

    Ok(ImageMeta {
        id,
        filename,
        scene,
        mime,
        width,
        height,
        size,
        ahash,
        created_at: now,
    })
}

fn resolve_image_slot(
    tx: &Connection,
    filename: &str,
    scene: PhotoScene,
) -> Result<(String, String), AppError> {
    let existing: Option<(String, String, String)> = tx
        .query_row(
            "SELECT id, scene, rel_path FROM images WHERE filename = ?1",
            params![filename],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(db_err)?;

    if let Some((old_id, old_scene_str, old_rel)) = existing {
        if let Ok(old_scene) = PhotoScene::parse(&old_scene_str)
            && old_scene != scene
        {
            let methods = load_methods_state(tx)?;
            let all_imgs: Vec<ImageMeta> = load_images_with_paths(tx)?
                .into_iter()
                .map(|(m, _)| m)
                .collect();
            if let Some(old_meta) = all_imgs.iter().find(|m| m.id == old_id) {
                guard_delete_image(old_meta, &all_imgs, methods)?;
            }
        }
        Ok((old_id, old_rel))
    } else {
        let new_id = Uuid::new_v4().to_string();
        let rel = format!("{new_id}.bin");
        Ok((new_id, rel))
    }
}

fn sanitize_filename(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
    if base.is_empty() || base.len() > 128 || base.starts_with('.') {
        return Err(AppError::InvalidInput(
            "图片文件名非空且长度不能超过 128 字符".to_string(),
        ));
    }
    Ok(base.to_string())
}

/// Reads raw image bytes and metadata for preview/download.
///
/// # Errors
/// Returns [`AppError::NotFound`] if `id` does not exist, or [`AppError::Internal`] on read failure.
pub fn get_image_raw(
    conn: &Connection,
    data_dir: &Path,
    id: &str,
) -> Result<(ImageMeta, Vec<u8>), AppError> {
    let (meta, rel_path) = find_image_by_id(conn, id)?
        .ok_or_else(|| AppError::NotFound(format!("未找到图片: {id}")))?;
    let path = data_dir.join("images").join(rel_path);
    let bytes =
        fs::read(&path).map_err(|e| AppError::Internal(format!("read image file failed: {e}")))?;
    Ok((meta, bytes))
}

/// Deletes an imported image after verifying active method locks.
///
/// # Errors
/// Returns [`AppError::NotFound`] if missing, or [`AppError::Conflict`] if locked by an active image method.
pub fn delete_image(conn: &mut Connection, data_dir: &Path, id: &str) -> Result<(), AppError> {
    let tx = conn.transaction().map_err(db_err)?;
    let all_with_paths = load_images_with_paths(&tx)?;
    let (target_meta, rel_path) = all_with_paths
        .iter()
        .find(|(m, _)| m.id == id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("未找到图片: {id}")))?;
    let all_meta: Vec<ImageMeta> = all_with_paths.into_iter().map(|(m, _)| m).collect();
    let methods = load_methods_state(&tx)?;
    guard_delete_image(&target_meta, &all_meta, methods)?;

    tx.execute("DELETE FROM images WHERE id = ?1", params![id])
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    drop(fs::remove_file(data_dir.join("images").join(rel_path)));
    Ok(())
}

/// Queries paginated posts with optional keyword filter.
///
/// # Errors
/// Returns [`AppError::Internal`] on `SQLite` query failure.
pub fn query_posts_page(
    conn: &Connection,
    page: u32,
    page_size: u32,
    keyword: &str,
) -> Result<PostsPageView, AppError> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, 200);
    let kw = keyword.trim();
    let all = crate::queries::load_posts(conn)?;
    let filtered: Vec<Post> = if kw.is_empty() {
        all
    } else {
        all.into_iter()
            .filter(|p| p.title.contains(kw) || p.body.contains(kw))
            .collect()
    };
    let total = filtered.len();
    let start = usize::try_from(page - 1)
        .unwrap_or(0)
        .saturating_mul(usize::try_from(page_size).unwrap_or(50));
    let items = if start >= total {
        Vec::new()
    } else {
        let end = start
            .saturating_add(usize::try_from(page_size).unwrap_or(50))
            .min(total);
        filtered.get(start..end).unwrap_or(&[]).to_vec()
    };
    Ok(PostsPageView {
        items,
        total,
        page,
        page_size,
    })
}

/// Inserts or upserts a single post record.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] on empty title or zero `user_id`.
pub fn upsert_post(conn: &mut Connection, input: PostInput) -> Result<Post, AppError> {
    validate_post_input(&input)?;
    let tx = conn.transaction().map_err(db_err)?;
    let id = match input.id.filter(|&v| v > 0) {
        Some(v) => v,
        None => {
            let max_id: Option<i64> = tx
                .query_row("SELECT MAX(id) FROM posts", [], |r| r.get(0))
                .map_err(db_err)?;
            max_id.map_or(1, |m| i64_to_u64(m).saturating_add(1))
        }
    };
    tx.execute(
        "INSERT INTO posts (id, user_id, title, body) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            user_id = excluded.user_id,
            title = excluded.title,
            body = excluded.body",
        params![
            u64_to_i64(id),
            u64_to_i64(input.user_id),
            input.title.trim(),
            input.body,
        ],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(Post {
        id,
        user_id: input.user_id,
        title: input.title.trim().to_string(),
        body: input.body,
    })
}

/// Updates an existing post by path `id`.
///
/// # Errors
/// Returns [`AppError::NotFound`] if `id` does not exist.
pub fn update_post_by_id(
    conn: &mut Connection,
    id: u64,
    mut input: PostInput,
) -> Result<Post, AppError> {
    validate_post_input(&input)?;
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM posts WHERE id = ?1)",
            params![u64_to_i64(id)],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if !exists {
        return Err(AppError::NotFound(format!("未找到帖子: {id}")));
    }
    input.id = Some(id);
    upsert_post(conn, input)
}

/// Batch imports post records in a single transaction.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] if any record fails validation.
pub fn batch_import_posts(conn: &mut Connection, items: Vec<PostInput>) -> Result<usize, AppError> {
    if items.is_empty() {
        return Err(AppError::InvalidInput("导入列表不能为空".to_string()));
    }
    for item in &items {
        validate_post_input(item)?;
    }
    let tx = conn.transaction().map_err(db_err)?;
    let mut next_id: u64 = tx
        .query_row("SELECT MAX(id) FROM posts", [], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .map_err(db_err)?
        .map_or(1, |m| i64_to_u64(m).saturating_add(1));

    let count = items.len();
    for item in items {
        let id = match item.id.filter(|&v| v > 0) {
            Some(explicit) => {
                if explicit >= next_id {
                    next_id = explicit.saturating_add(1);
                }
                explicit
            }
            None => {
                let assigned = next_id;
                next_id = next_id.saturating_add(1);
                assigned
            }
        };
        tx.execute(
            "INSERT INTO posts (id, user_id, title, body) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                user_id = excluded.user_id,
                title = excluded.title,
                body = excluded.body",
            params![
                u64_to_i64(id),
                u64_to_i64(item.user_id),
                item.title.trim(),
                item.body,
            ],
        )
        .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)?;
    Ok(count)
}

/// Deletes a post by `id` if not locked by an active `posts.search` interface.
///
/// # Errors
/// Returns [`AppError::Conflict`] if deleting the last post while `posts.search` is enabled.
pub fn delete_post_by_id(conn: &mut Connection, id: u64) -> Result<(), AppError> {
    let tx = conn.transaction().map_err(db_err)?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM posts WHERE id = ?1)",
            params![u64_to_i64(id)],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if !exists {
        return Err(AppError::NotFound(format!("未找到帖子: {id}")));
    }
    let total = count_posts(&tx)?;
    let methods = load_methods_state(&tx)?;
    guard_delete_post(total, methods)?;
    tx.execute("DELETE FROM posts WHERE id = ?1", params![u64_to_i64(id)])
        .map_err(db_err)?;
    tx.commit().map_err(db_err)
}

fn validate_post_input(input: &PostInput) -> Result<(), AppError> {
    if input.user_id == 0 {
        return Err(AppError::InvalidInput("userId 须大于 0".to_string()));
    }
    if input.title.trim().is_empty() || input.title.len() > 256 {
        return Err(AppError::InvalidInput(
            "帖子标题长度须在 1 到 256 字符之间".to_string(),
        ));
    }
    if input.body.len() > 16384 {
        return Err(AppError::InvalidInput("帖子正文长度超出上限".to_string()));
    }
    Ok(())
}

/// Converts an [`AppRecord`] into a redacted [`AppMetaView`].
#[must_use]
pub fn to_app_meta(app: &AppRecord) -> AppMetaView {
    let preview = if app.client_secret.len() >= 4 {
        let prefix = app.client_secret.chars().take(2).collect::<String>();
        format!("{prefix}***({}B)", app.client_secret.len())
    } else {
        "***".to_string()
    };
    AppMetaView {
        client_id: app.client_id.clone(),
        name: app.name.clone(),
        has_secret: !app.client_secret.is_empty(),
        secret_preview: preview,
        has_rsa_public_pem: app
            .rsa_public_pem
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty()),
        methods: app.methods.clone(),
        created_at: app.created_at.clone(),
    }
}

/// Creates or updates a caller credential in `apps`.
///
/// # Errors
/// Returns [`AppError::InvalidInput`] on invalid fields or [`AppError::Conflict`] if active fronts break.
pub fn save_app(
    conn: &mut Connection,
    path_id: Option<&str>,
    payload: AppUpsertPayload,
) -> Result<AppMetaView, AppError> {
    let client_id = path_id.unwrap_or(&payload.client_id).trim().to_string();
    if client_id.is_empty() || client_id.len() > 64 {
        return Err(AppError::InvalidInput(
            "client_id 长度须在 1 到 64 字符之间".to_string(),
        ));
    }
    if payload.name.trim().is_empty() || payload.name.len() > 64 {
        return Err(AppError::InvalidInput(
            "应用名称长度须在 1 到 64 字符之间".to_string(),
        ));
    }
    for m in &payload.methods {
        if !is_known_method(m) {
            return Err(AppError::InvalidInput(format!("未知业务子接口: {m}")));
        }
    }

    let tx = conn.transaction().map_err(db_err)?;
    let existing = load_apps(&tx)?
        .into_iter()
        .find(|a| a.client_id == client_id);
    if path_id.is_some() && existing.is_none() {
        return Err(AppError::NotFound(format!("未找到应用凭据: {client_id}")));
    }
    let secret = if payload.client_secret.trim().is_empty() {
        existing
            .as_ref()
            .map(|e| e.client_secret.clone())
            .ok_or_else(|| {
                AppError::InvalidInput(
                    "新建凭据时 client_secret 长度须在 8 到 128 字节之间".to_string(),
                )
            })?
    } else {
        payload.client_secret.clone()
    };
    if !(8..=128).contains(&secret.len()) {
        return Err(AppError::InvalidInput(
            "client_secret 长度须在 8 到 128 字节之间".to_string(),
        ));
    }
    let rsa_pem = payload
        .rsa_public_pem
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let created_at = existing.map_or_else(|| Utc::now().to_rfc3339(), |e| e.created_at);

    let methods_json = serde_json::to_string(&payload.methods).map_err(json_err)?;
    tx.execute(
        "INSERT INTO apps (client_id, client_secret, name, rsa_public_pem, methods_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(client_id) DO UPDATE SET
            client_secret = excluded.client_secret,
            name = excluded.name,
            rsa_public_pem = excluded.rsa_public_pem,
            methods_json = excluded.methods_json",
        params![
            client_id,
            secret,
            payload.name.trim(),
            rsa_pem,
            methods_json,
            created_at,
        ],
    )
    .map_err(db_err)?;

    verify_active_core_apps(&tx)?;
    tx.commit().map_err(db_err)?;

    let rec = AppRecord {
        client_id,
        client_secret: secret,
        name: payload.name.trim().to_string(),
        rsa_public_pem: rsa_pem,
        methods: payload.methods,
        created_at,
    };
    Ok(to_app_meta(&rec))
}

/// Deletes a caller credential if not required by any enabled core front.
///
/// # Errors
/// Returns [`AppError::NotFound`] if missing, or [`AppError::Conflict`] if active fronts lose their last valid app.
pub fn delete_app_by_id(conn: &mut Connection, client_id: &str) -> Result<(), AppError> {
    let tx = conn.transaction().map_err(db_err)?;
    let changed = tx
        .execute("DELETE FROM apps WHERE client_id = ?1", params![client_id])
        .map_err(db_err)?;
    if changed == 0 {
        return Err(AppError::NotFound(format!("未找到应用凭据: {client_id}")));
    }
    verify_active_core_apps(&tx)?;
    tx.commit().map_err(db_err)
}

fn verify_active_core_apps(conn: &Connection) -> Result<(), AppError> {
    let fronts = load_fronts(conn)?;
    let methods: MethodsState = load_methods_state(conn)?;
    let apps = load_apps(conn)?;
    let has_valid = apps
        .iter()
        .any(|a| a.methods.iter().any(|m| methods.is_enabled(m)));
    let broken: Vec<String> = fronts
        .iter()
        .filter(|f: &&FrontRecord| {
            f.enabled
                && !f.name.is_envelope()
                && (f.name == FrontName::SignGateway || !f.auth.contains(&AuthMethod::None))
                && !has_valid
        })
        .map(|f| f.name.as_str().to_string())
        .collect();
    if broken.is_empty() {
        Ok(())
    } else {
        Err(AppError::in_use(
            "仍有已启用接入面依赖有效应用凭据，请先关闭对应接入面",
            broken,
        ))
    }
}
