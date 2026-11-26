#![doc(test(attr(deny(warnings), forbid(unsafe_code))))]
//! `SQLite` WAL persistence, zero-preset state management, and precondition gate engine for `x-upt`.

pub mod dto;
pub mod ops_assets;
pub mod ops_envelopes;
pub mod ops_switches;
mod queries;
mod schema;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use chrono::Utc;
use rusqlite::Connection;
use x_upt_core::domain::{
    AppRegistry, BusinessMethod, EnvelopeProfileSpec, FrontName, FrontRecord, GlobalSettings,
};
use x_upt_core::error::AppError;
use x_upt_core::imgs::{ImageMeta, PhotoScene, RuntimeImage};
use x_upt_core::preconditions::MethodStatus;
use x_upt_core::services::{CoreDataset, Post};
use x_upt_crypto::profile::compile_profiles;

use crate::dto::{
    AppMetaView, AppUpsertPayload, EnvelopeProfilesView, FrontStatusView, FrontUpdatePayload,
    PostInput, PostsPageView, RuntimeSnapshotData, SecretMetaView, SecretUpsertPayload,
    SystemStatusView,
};

/// Thread-safe `SQLite` store managing `<data-dir>/x-upt.db` and `<data-dir>/images/`.
#[derive(Debug)]
pub struct Store {
    data_dir: PathBuf,
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens or creates `<data_dir>/x-upt.db` in WAL mode and runs idempotent migrations.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] if directory creation or `SQLite` initialization fails.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, AppError> {
        let dir = data_dir.as_ref().to_path_buf();
        fs::create_dir_all(dir.join("images"))
            .map_err(|e| AppError::Internal(format!("failed to create data/images dir: {e}")))?;
        fs::create_dir_all(dir.join("logs"))
            .map_err(|e| AppError::Internal(format!("failed to create data/logs dir: {e}")))?;
        let db_path = dir.join("x-upt.db");
        let mut conn = Connection::open(&db_path).map_err(|e| {
            AppError::Internal(format!("failed to open {}: {e}", db_path.display()))
        })?;
        let now = Utc::now().to_rfc3339();
        schema::init_database(&mut conn, &now)?;
        Ok(Self {
            data_dir: dir,
            conn: Mutex::new(conn),
        })
    }

    /// Data directory root path.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn lock_conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Builds a complete immutable [`RuntimeSnapshotData`] from current `SQLite` tables and image files.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] if database reads fail.
    pub fn build_snapshot(&self) -> Result<RuntimeSnapshotData, AppError> {
        let conn = self.lock_conn();
        let settings = queries::load_settings(&conn)?;
        let methods = queries::load_methods_state(&conn)?;
        let posts = queries::load_posts(&conn)?;
        let app_records = queries::load_apps(&conn)?;
        let front_list = queries::load_fronts(&conn)?;
        let imgs_with_paths = queries::load_images_with_paths(&conn)?;
        let secrets = ops_envelopes::secrets_by_name(&conn)?;
        let mut profiles_map = queries::load_profiles_map(&conn)?;
        drop(conn);

        let mut scene_images: HashMap<PhotoScene, Vec<RuntimeImage>> = HashMap::new();
        let mut image_hashes: HashMap<String, u64> = HashMap::new();
        for (meta, rel_path) in imgs_with_paths {
            let full = self.data_dir.join("images").join(&rel_path);
            let Ok(bytes) = fs::read(&full) else {
                continue;
            };
            let ahash_u64 = u64::from_str_radix(&meta.ahash, 16).unwrap_or(0);
            image_hashes.insert(meta.filename.clone(), ahash_u64);
            scene_images
                .entry(meta.scene)
                .or_default()
                .push(RuntimeImage {
                    meta,
                    ahash_u64,
                    bytes,
                });
        }

        let mut fronts = HashMap::new();
        for f in &front_list {
            fronts.insert(f.name, f.clone());
        }

        let mut profiles_by_front = Vec::new();
        let mut compiled_envelopes = HashMap::new();
        for name in FrontName::all().into_iter().filter(|n| n.is_envelope()) {
            let specs = profiles_map.remove(&name).unwrap_or_default();
            if !specs.is_empty()
                && let Ok(compiled) = compile_profiles(name, &specs, &secrets)
            {
                compiled_envelopes.insert(name, compiled);
            }
            profiles_by_front.push((name, specs));
        }

        Ok(RuntimeSnapshotData {
            settings,
            dataset: CoreDataset {
                methods,
                posts,
                scene_images,
                image_hashes,
            },
            apps: AppRegistry::from_records(app_records),
            fronts,
            front_list,
            profiles_by_front,
            compiled_envelopes,
        })
    }

    /// Computes full system status for `GET /api/v1/status`.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn system_status(
        &self,
        version: &str,
        admin_listen: &str,
        admin_port: u16,
        running: &HashSet<FrontName>,
    ) -> Result<SystemStatusView, AppError> {
        let conn = self.lock_conn();
        ops_switches::compute_system_status(&conn, version, admin_listen, admin_port, running)
    }

    /// Lists all 15 front status views.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn list_fronts(
        &self,
        version: &str,
        admin_listen: &str,
        admin_port: u16,
        running: &HashSet<FrontName>,
    ) -> Result<Vec<FrontStatusView>, AppError> {
        Ok(self
            .system_status(version, admin_listen, admin_port, running)?
            .fronts)
    }

    /// Evaluates a single front's [`FrontStatusView`].
    ///
    /// # Errors
    /// Returns [`AppError::NotFound`] if missing or [`AppError::Internal`] on `SQLite` error.
    pub fn front_status(
        &self,
        front: FrontName,
        version: &str,
        admin_listen: &str,
        admin_port: u16,
        running: &HashSet<FrontName>,
    ) -> Result<FrontStatusView, AppError> {
        self.list_fronts(version, admin_listen, admin_port, running)?
            .into_iter()
            .find(|f| f.name == front.as_str())
            .ok_or_else(|| AppError::NotFound(format!("未找到接入面: {}", front.as_str())))
    }

    /// Reads global settings.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn get_settings(&self) -> Result<GlobalSettings, AppError> {
        let conn = self.lock_conn();
        queries::load_settings(&conn)
    }

    /// Updates global settings.
    ///
    /// # Errors
    /// Returns [`AppError::InvalidInput`] on out-of-range settings.
    pub fn update_settings(&self, next: GlobalSettings) -> Result<GlobalSettings, AppError> {
        let mut conn = self.lock_conn();
        ops_switches::update_global_settings(&mut conn, next)
    }

    /// Enables or disables a business method with precondition and dependency lock enforcement.
    ///
    /// # Errors
    /// Returns [`AppError::Conflict`] when blocked.
    pub fn set_method(
        &self,
        method: BusinessMethod,
        enable: bool,
    ) -> Result<MethodStatus, AppError> {
        let mut conn = self.lock_conn();
        ops_switches::set_business_method(&mut conn, method, enable)
    }

    /// Updates configuration of `front`.
    ///
    /// # Errors
    /// Returns [`AppError`] on validation or conflict failure.
    pub fn update_front(
        &self,
        admin_port: u16,
        front: FrontName,
        payload: FrontUpdatePayload,
    ) -> Result<FrontRecord, AppError> {
        let mut conn = self.lock_conn();
        ops_switches::update_front_record(&mut conn, admin_port, front, payload)
    }

    /// Verifies preconditions before binding a front listener.
    ///
    /// # Errors
    /// Returns [`AppError::Conflict`] if preconditions are unmet.
    pub fn check_front_can_enable(
        &self,
        admin_port: u16,
        front: FrontName,
    ) -> Result<FrontRecord, AppError> {
        let conn = self.lock_conn();
        ops_switches::check_front_can_enable(&conn, admin_port, front)
    }

    /// Persists `enabled` state for `front`.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn set_front_enabled(&self, front: FrontName, enabled: bool) -> Result<(), AppError> {
        let conn = self.lock_conn();
        ops_switches::persist_front_enabled(&conn, front, enabled)
    }

    /// Lists all persisted [`FrontRecord`] entries.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn raw_fronts(&self) -> Result<Vec<FrontRecord>, AppError> {
        let conn = self.lock_conn();
        queries::load_fronts(&conn)
    }

    /// Lists imported image metadata.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn list_images(&self) -> Result<Vec<ImageMeta>, AppError> {
        let conn = self.lock_conn();
        Ok(queries::load_images_with_paths(&conn)?
            .into_iter()
            .map(|(m, _)| m)
            .collect())
    }

    /// Imports and persists a benchmark image.
    ///
    /// # Errors
    /// Returns [`AppError::InvalidInput`] on malformed image/filename.
    pub fn import_image(
        &self,
        scene: PhotoScene,
        filename: &str,
        bytes: &[u8],
    ) -> Result<ImageMeta, AppError> {
        let mut conn = self.lock_conn();
        ops_assets::import_image(&mut conn, &self.data_dir, scene, filename, bytes)
    }

    /// Reads raw image bytes by ID.
    ///
    /// # Errors
    /// Returns [`AppError::NotFound`] if missing.
    pub fn get_image_raw(&self, id: &str) -> Result<(ImageMeta, Vec<u8>), AppError> {
        let conn = self.lock_conn();
        ops_assets::get_image_raw(&conn, &self.data_dir, id)
    }

    /// Deletes an imported image by ID.
    ///
    /// # Errors
    /// Returns [`AppError::Conflict`] if locked by an active image method.
    pub fn delete_image(&self, id: &str) -> Result<(), AppError> {
        let mut conn = self.lock_conn();
        ops_assets::delete_image(&mut conn, &self.data_dir, id)
    }

    /// Queries paginated posts.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn list_posts(
        &self,
        page: u32,
        page_size: u32,
        keyword: &str,
    ) -> Result<PostsPageView, AppError> {
        let conn = self.lock_conn();
        ops_assets::query_posts_page(&conn, page, page_size, keyword)
    }

    /// Creates or upserts a post.
    ///
    /// # Errors
    /// Returns [`AppError::InvalidInput`] on invalid post fields.
    pub fn create_post(&self, input: PostInput) -> Result<Post, AppError> {
        let mut conn = self.lock_conn();
        ops_assets::upsert_post(&mut conn, input)
    }

    /// Updates a post by ID.
    ///
    /// # Errors
    /// Returns [`AppError::NotFound`] if missing.
    pub fn update_post(&self, id: u64, input: PostInput) -> Result<Post, AppError> {
        let mut conn = self.lock_conn();
        ops_assets::update_post_by_id(&mut conn, id, input)
    }

    /// Batch imports posts.
    ///
    /// # Errors
    /// Returns [`AppError::InvalidInput`] on empty or malformed items.
    pub fn import_posts(&self, items: Vec<PostInput>) -> Result<usize, AppError> {
        let mut conn = self.lock_conn();
        ops_assets::batch_import_posts(&mut conn, items)
    }

    /// Deletes a post by ID.
    ///
    /// # Errors
    /// Returns [`AppError::Conflict`] if deleting the last post while `posts.search` is active.
    pub fn delete_post(&self, id: u64) -> Result<(), AppError> {
        let mut conn = self.lock_conn();
        ops_assets::delete_post_by_id(&mut conn, id)
    }

    /// Lists caller credentials with redacted secrets.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn list_apps(&self) -> Result<Vec<AppMetaView>, AppError> {
        let conn = self.lock_conn();
        Ok(queries::load_apps(&conn)?
            .iter()
            .map(ops_assets::to_app_meta)
            .collect())
    }

    /// Creates or updates a caller credential.
    ///
    /// # Errors
    /// Returns [`AppError`] on validation or conflict failure.
    pub fn save_app(
        &self,
        path_id: Option<&str>,
        payload: AppUpsertPayload,
    ) -> Result<AppMetaView, AppError> {
        let mut conn = self.lock_conn();
        ops_assets::save_app(&mut conn, path_id, payload)
    }

    /// Deletes a caller credential.
    ///
    /// # Errors
    /// Returns [`AppError::Conflict`] if required by an active core front.
    pub fn delete_app(&self, client_id: &str) -> Result<(), AppError> {
        let mut conn = self.lock_conn();
        ops_assets::delete_app_by_id(&mut conn, client_id)
    }

    /// Lists envelope secret metadata.
    ///
    /// # Errors
    /// Returns [`AppError::Internal`] on `SQLite` error.
    pub fn list_secrets(&self) -> Result<Vec<SecretMetaView>, AppError> {
        let conn = self.lock_conn();
        ops_envelopes::list_secret_metas(&conn)
    }

    /// Uploads or generates envelope secret(s).
    ///
    /// # Errors
    /// Returns [`AppError`] on validation or conflict failure.
    pub fn upsert_secret(
        &self,
        payload: SecretUpsertPayload,
    ) -> Result<Vec<SecretMetaView>, AppError> {
        let mut conn = self.lock_conn();
        ops_envelopes::upsert_envelope_secret(&mut conn, payload)
    }

    /// Deletes an envelope secret.
    ///
    /// # Errors
    /// Returns [`AppError::Conflict`] if referenced by an enabled envelope front.
    pub fn delete_secret(&self, name: &str) -> Result<(), AppError> {
        let mut conn = self.lock_conn();
        ops_envelopes::delete_envelope_secret(&mut conn, name)
    }

    /// Reads envelope profiles for `front`.
    ///
    /// # Errors
    /// Returns [`AppError::InvalidInput`] if `front` is not an envelope front.
    pub fn get_profiles(&self, front: FrontName) -> Result<EnvelopeProfilesView, AppError> {
        let conn = self.lock_conn();
        ops_envelopes::get_front_profiles(&conn, front)
    }

    /// Replaces envelope profiles for `front`.
    ///
    /// # Errors
    /// Returns [`AppError`] on invalid profile/secret references or active front conflict.
    pub fn replace_profiles(
        &self,
        front: FrontName,
        profiles: Vec<EnvelopeProfileSpec>,
    ) -> Result<EnvelopeProfilesView, AppError> {
        let mut conn = self.lock_conn();
        ops_envelopes::replace_front_profiles(&mut conn, front, profiles)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use x_upt_core::domain::{
        BusinessMethod, FrontName, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH,
    };
    use x_upt_core::imgs::PhotoScene;

    use crate::Store;
    use crate::dto::{AppUpsertPayload, PostInput};

    fn sample_png_bytes() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut img = image::RgbImage::new(12, 12);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = image::Rgb([u8::try_from(x * 15)?, u8::try_from(y * 15)?, 90]);
        }
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let mut out = Vec::new();
        dyn_img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
        Ok(out)
    }

    #[test]
    fn zero_preset_and_precondition_gates_persist_across_reopen()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("x-upt-store-test-{}", uuid::Uuid::new_v4()));
        let png = sample_png_bytes()?;

        {
            let store = Store::open(&dir)?;
            let status = store.system_status("0.1.0", "127.0.0.1:18768", 18768, &HashSet::new())?;
            assert_eq!(status.counts.images, 0);
            assert_eq!(status.counts.posts, 0);
            assert_eq!(status.counts.apps, 0);
            assert!(status.methods.iter().all(|m| !m.enabled && !m.can_enable));
            assert!(status.fronts.iter().all(|f| !f.enabled && !f.can_enable));

            // Without images/posts, enabling switches MUST return Conflict.
            assert!(
                store
                    .set_method(BusinessMethod::PhotosRender, true)
                    .is_err()
            );
            assert!(
                store
                    .set_method(BusinessMethod::ImgdefCompare, true)
                    .is_err()
            );
            assert!(store.set_method(BusinessMethod::PostsSearch, true).is_err());
            assert!(
                store
                    .check_front_can_enable(18768, FrontName::Rest)
                    .is_err()
            );

            // Import 1 image -> imgdef unlocks, photos.render still blocked until all 3 scenes exist.
            let cover = store.import_image(PhotoScene::ArticleCover, "cover.png", &png)?;
            assert!(
                store
                    .set_method(BusinessMethod::ImgdefCompare, true)
                    .is_ok()
            );
            assert!(
                store
                    .set_method(BusinessMethod::PhotosRender, true)
                    .is_err()
            );

            store.import_image(PhotoScene::Avatar, "avatar.png", &png)?;
            store.import_image(PhotoScene::Banner, "banner.png", &png)?;
            assert!(store.set_method(BusinessMethod::PhotosRender, true).is_ok());

            // Active dependency lock: deleting the only article_cover image is blocked.
            assert!(store.delete_image(&cover.id).is_err());

            // Import post + app -> enable posts.search and REST front.
            let post = store.create_post(PostInput {
                id: Some(1),
                user_id: 7,
                title: "hello".into(),
                body: "world".into(),
            })?;
            assert!(store.set_method(BusinessMethod::PostsSearch, true).is_ok());
            assert!(store.delete_post(post.id).is_err());

            store.save_app(
                None,
                AppUpsertPayload {
                    client_id: "demo".into(),
                    client_secret: "secret_12345".into(),
                    name: "Demo".into(),
                    rsa_public_pem: None,
                    methods: vec![METHOD_POSTS_SEARCH.into(), METHOD_PHOTOS_RENDER.into()],
                },
            )?;
            assert!(store.check_front_can_enable(18768, FrontName::Rest).is_ok());
            store.set_front_enabled(FrontName::Rest, true)?;
            assert!(store.delete_app("demo").is_err());
        }

        // Re-open store from disk and verify persisted switches & snapshot.
        {
            let reopened = Store::open(&dir)?;
            let snap = reopened.build_snapshot()?;
            assert!(snap.dataset.methods.photos_render);
            assert!(snap.dataset.methods.imgdef_compare);
            assert!(snap.dataset.methods.posts_search);
            assert_eq!(snap.dataset.posts.len(), 1);
            assert_eq!(snap.apps.len(), 1);
            assert!(snap.fronts.get(&FrontName::Rest).is_some_and(|f| f.enabled));
        }

        drop(std::fs::remove_dir_all(&dir));
        Ok(())
    }
}
