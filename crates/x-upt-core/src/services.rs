//! Deterministic `x_upt.posts.search`, `x_upt.photos.render`, and `x_upt.imgdef.compare` kernel.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use base64::Engine;
use image::{DynamicImage, imageops::FilterType};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::domain::{
    AuthedApp, METHOD_IMGDEF_COMPARE, METHOD_PHOTOS_RENDER, METHOD_POSTS_SEARCH, MethodsState,
    is_known_method,
};
use crate::error::{CoreError, CoreErrorKind};
use crate::imgs::{
    PhotoFormat, PhotoScene, RuntimeImage, average_hash, hamming_distance, load_image_bytes,
};

/// Minimum allowed rendered image edge in pixels.
pub const MIN_PHOTO_EDGE: u32 = 8;
/// Maximum allowed rendered image edge in pixels.
pub const MAX_PHOTO_EDGE: u32 = 1024;

/// Sort order for `x_upt.posts.search`.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PostSort {
    /// Ascending by post ID.
    IdAsc,
    /// Descending by post ID.
    IdDesc,
    /// Ascending lexicographically by title.
    TitleAsc,
    /// Descending lexicographically by title.
    TitleDesc,
}

/// Request payload for `x_upt.posts.search`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PostsRequest {
    /// Substring filter matched against `title` or `body`.
    pub keyword: String,
    /// Author user ID filter.
    pub user_id: u64,
    /// 1-based page index.
    pub page: u32,
    /// Page size (`1`..=`20`).
    pub page_size: u32,
    /// Sort mode.
    pub sort: PostSort,
}

/// Single post record stored in `SQLite` and returned by `x_upt.posts.search`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Post {
    /// Author user ID (`userId` in JSON).
    #[serde(rename = "userId")]
    pub user_id: u64,
    /// Unique post ID.
    pub id: u64,
    /// Post title.
    pub title: String,
    /// Post body text.
    pub body: String,
}

/// Response payload for `x_upt.posts.search`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PostsResponse {
    /// Page items.
    pub items: Vec<Post>,
    /// Total matching post count.
    pub total: usize,
    /// Current page index.
    pub page: u32,
    /// Requested page size.
    pub page_size: u32,
}

/// Request payload for `x_upt.photos.render`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PhotosRequest {
    /// Target photo scene.
    pub scene: PhotoScene,
    /// Output width (`8`..=`1024`).
    pub width: u32,
    /// Output height (`8`..=`1024`).
    pub height: u32,
    /// Output image encoding format.
    pub format: PhotoFormat,
}

/// Rendered image payload inside [`PhotosResponse`].
#[derive(Debug, Clone, Serialize)]
pub struct ImageInfo {
    /// Source benchmark filename selected deterministically.
    pub filename: String,
    /// Output MIME type.
    pub mime: String,
    /// Encoded byte size.
    pub size: u64,
    /// Standard Base64-encoded image bytes.
    pub base64: String,
}

/// Response payload for `x_upt.photos.render`.
#[derive(Debug, Clone, Serialize)]
pub struct PhotosResponse {
    /// Echoed scene.
    pub scene: PhotoScene,
    /// Echoed width.
    pub width: u32,
    /// Echoed height.
    pub height: u32,
    /// Echoed format.
    pub format: PhotoFormat,
    /// Rendered image details.
    pub image: ImageInfo,
}

/// Request payload for `x_upt.imgdef.compare`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImgdefRequest {
    /// Benchmark image filename (without path separators).
    pub filename: String,
    /// Base64-encoded image (optional `data:*;base64,` prefix allowed).
    pub image: String,
    /// Maximum Hamming distance (`0`..=`64`) considered a match.
    pub threshold: u32,
}

/// Response payload for `x_upt.imgdef.compare`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImgdefResponse {
    /// Whether `distance <= threshold`.
    pub matched: bool,
    /// Compared benchmark filename.
    pub filename: String,
    /// Computed 64-bit aHash Hamming distance.
    pub distance: u32,
    /// Echoed threshold.
    pub threshold: u32,
}

/// Immutable in-memory dataset for the 3 deterministic business methods.
#[derive(Debug, Clone, Default)]
pub struct CoreDataset {
    /// Enabled method switches.
    pub methods: MethodsState,
    /// All imported posts.
    pub posts: Vec<Post>,
    /// Imported images grouped and sorted by `(filename, id)` per scene.
    pub scene_images: HashMap<PhotoScene, Vec<RuntimeImage>>,
    /// Imported image perceptual hashes indexed by `filename`.
    pub image_hashes: HashMap<String, u64>,
}

/// Single business invocation passed into [`CoreDataset::dispatch`].
#[derive(Debug, Clone)]
pub struct Call {
    /// Authenticated caller ID.
    pub client_id: String,
    /// Methods granted to the caller.
    pub allowed_methods: HashSet<String>,
    /// Target business method wire name.
    pub method: String,
    /// JSON business parameters.
    pub biz_content: serde_json::Value,
}

impl Call {
    /// Constructs a [`Call`] from an [`AuthedApp`].
    #[must_use]
    pub fn from_app(app: &AuthedApp, method: String, biz_content: serde_json::Value) -> Self {
        Self {
            client_id: app.client_id.clone(),
            allowed_methods: app.methods.clone(),
            method,
            biz_content,
        }
    }
}

impl CoreDataset {
    /// Executes a deterministic business method call against the current snapshot.
    ///
    /// # Errors
    /// Returns [`CoreError`] when the method is empty, unknown/disabled, ungranted, or invalid.
    pub fn dispatch(&self, call: Call) -> Result<serde_json::Value, CoreError> {
        let method = call.method.trim();
        if method.is_empty() {
            return Err(CoreError::new(CoreErrorKind::BadRequest(
                "method must be non-empty".to_string(),
            )));
        }
        if !is_known_method(method) || !self.methods.is_enabled(method) {
            return Err(CoreError::new(CoreErrorKind::UnknownMethod));
        }
        if !call.allowed_methods.contains(method) {
            return Err(CoreError::new(CoreErrorKind::Forbidden));
        }
        if !call.biz_content.is_object() {
            return Err(CoreError::new(CoreErrorKind::BadRequest(
                "biz_content must be an object".to_string(),
            )));
        }

        match method {
            METHOD_POSTS_SEARCH => {
                let req: PostsRequest = parse_biz(&call.biz_content)?;
                let resp = search_posts(&req, &self.posts)?;
                encode_resp(resp)
            }
            METHOD_PHOTOS_RENDER => {
                let req: PhotosRequest = parse_biz(&call.biz_content)?;
                let imgs = self
                    .scene_images
                    .get(&req.scene)
                    .map_or(&[][..], Vec::as_slice);
                let resp = render_photo(&req, imgs)?;
                encode_resp(resp)
            }
            METHOD_IMGDEF_COMPARE => {
                let req: ImgdefRequest = parse_biz(&call.biz_content)?;
                let resp = compare_image(&req, &self.image_hashes)?;
                encode_resp(resp)
            }
            _ => Err(CoreError::new(CoreErrorKind::UnknownMethod)),
        }
    }
}

/// Validates a job submission before enqueueing without running the business body.
#[must_use]
pub fn validate_job_call(
    app: &AuthedApp,
    methods: MethodsState,
    method: &str,
    biz: &serde_json::Value,
) -> Option<(u16, &'static str, String)> {
    let method = method.trim();
    if method.is_empty() {
        return Some((400, "bad_request", "method must be non-empty".to_string()));
    }
    if !is_known_method(method) || !methods.is_enabled(method) {
        return Some((400, "unknown_method", "unknown method".to_string()));
    }
    if !app.methods.contains(method) {
        return Some((
            403,
            "forbidden",
            "method is not granted to this client".to_string(),
        ));
    }
    if !biz.is_object() {
        return Some((
            400,
            "bad_request",
            "biz_content must be an object".to_string(),
        ));
    }
    None
}

/// Deterministic post search over a slice of [`Post`] records.
///
/// # Errors
/// Returns [`CoreError`] when `page < 1` or `page_size` is outside `1..=20`.
pub fn search_posts(req: &PostsRequest, posts: &[Post]) -> Result<PostsResponse, CoreError> {
    if req.page < 1 {
        return Err(CoreError::new(CoreErrorKind::BadRequest(
            "page must be >= 1".to_string(),
        )));
    }
    if !(1..=20).contains(&req.page_size) {
        return Err(CoreError::new(CoreErrorKind::BadRequest(
            "page_size must be between 1 and 20".to_string(),
        )));
    }

    let mut hits: Vec<Post> = posts
        .iter()
        .filter(|post| {
            post.user_id == req.user_id
                && (post.title.contains(&req.keyword) || post.body.contains(&req.keyword))
        })
        .cloned()
        .collect();

    match req.sort {
        PostSort::IdAsc => hits.sort_by_key(|p| p.id),
        PostSort::IdDesc => hits.sort_by_key(|p| std::cmp::Reverse(p.id)),
        PostSort::TitleAsc => hits.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id))),
        PostSort::TitleDesc => hits.sort_by(|a, b| b.title.cmp(&a.title).then(a.id.cmp(&b.id))),
    }

    let total = hits.len();
    let page = usize::try_from(req.page).unwrap_or(usize::MAX);
    let page_size = usize::try_from(req.page_size).unwrap_or(0);
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let items = hits.into_iter().skip(offset).take(page_size).collect();

    Ok(PostsResponse {
        items,
        total,
        page: req.page,
        page_size: req.page_size,
    })
}

/// Deterministic photo rendering over imported images belonging to `req.scene`.
///
/// # Errors
/// Returns [`CoreError`] if dimensions are out of bounds, no scene images exist, or encoding fails.
pub fn render_photo(
    req: &PhotosRequest,
    scene_images: &[RuntimeImage],
) -> Result<PhotosResponse, CoreError> {
    if !(MIN_PHOTO_EDGE..=MAX_PHOTO_EDGE).contains(&req.width)
        || !(MIN_PHOTO_EDGE..=MAX_PHOTO_EDGE).contains(&req.height)
    {
        return Err(CoreError::new(CoreErrorKind::BadRequest(
            "width and height must be between 8 and 1024".to_string(),
        )));
    }
    if scene_images.is_empty() {
        return Err(CoreError::new(CoreErrorKind::NotFound(
            "no images found for scene".to_string(),
        )));
    }

    let idx = pick_photo_index(
        scene_images.len(),
        req.scene,
        req.width,
        req.height,
        req.format,
    );
    let chosen = scene_images.get(idx).ok_or_else(|| {
        CoreError::new(CoreErrorKind::Internal(
            "scene index out of range".to_string(),
        ))
    })?;

    let source =
        load_image_bytes(&chosen.bytes).map_err(|e| CoreError::new(CoreErrorKind::Internal(e)))?;
    let resized = source.resize_exact(req.width, req.height, FilterType::Triangle);
    let encoded = encode_image(&resized, req.format)?;
    let size = u64::try_from(encoded.len()).unwrap_or(u64::MAX);

    Ok(PhotosResponse {
        scene: req.scene,
        width: req.width,
        height: req.height,
        format: req.format,
        image: ImageInfo {
            filename: chosen.meta.filename.clone(),
            mime: req.format.mime().to_string(),
            size,
            base64: base64::engine::general_purpose::STANDARD.encode(&encoded),
        },
    })
}

/// Perceptual average-hash comparison against imported benchmark image hashes.
///
/// # Errors
/// Returns [`CoreError`] when parameters are invalid, image decoding fails, or `filename` is unknown.
pub fn compare_image(
    req: &ImgdefRequest,
    image_hashes: &HashMap<String, u64>,
) -> Result<ImgdefResponse, CoreError> {
    if req.threshold > 64 {
        return Err(CoreError::new(CoreErrorKind::BadRequest(
            "threshold must be between 0 and 64".to_string(),
        )));
    }
    if !is_safe_filename(&req.filename) {
        return Err(CoreError::new(CoreErrorKind::BadRequest(
            "filename must be a file name without path".to_string(),
        )));
    }
    let trimmed = req.image.trim();
    if trimmed.is_empty() {
        return Err(CoreError::new(CoreErrorKind::BadRequest(
            "image must be non-empty".to_string(),
        )));
    }

    let b64 = match trimmed.find("base64,") {
        Some(idx) => trimmed.get(idx + 7..).unwrap_or(""),
        None => trimmed,
    };
    let uploaded_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| {
            CoreError::new(CoreErrorKind::BadRequest(
                "image must be valid base64".to_string(),
            ))
        })?;
    let uploaded = load_image_bytes(&uploaded_bytes)
        .map_err(|e| CoreError::new(CoreErrorKind::BadRequest(e)))?;

    let Some(&target_hash) = image_hashes.get(&req.filename) else {
        return Err(CoreError::new(CoreErrorKind::NotFound(
            "image file not found".to_string(),
        )));
    };
    let uploaded_hash = average_hash(&uploaded).ok_or_else(|| {
        CoreError::new(CoreErrorKind::BadRequest(
            "image must be a valid image".to_string(),
        ))
    })?;
    let distance = hamming_distance(uploaded_hash, target_hash);

    Ok(ImgdefResponse {
        matched: distance <= req.threshold,
        filename: req.filename.clone(),
        distance,
        threshold: req.threshold,
    })
}

/// Checks that `raw` is a clean single-component file name without path traversal.
#[must_use]
pub fn is_safe_filename(raw: &str) -> bool {
    if raw.is_empty() || raw.len() > 128 || raw == "." || raw == ".." {
        return false;
    }
    !raw.contains('/') && !raw.contains('\\') && !raw.contains('\0')
}

fn pick_photo_index(
    count: usize,
    scene: PhotoScene,
    width: u32,
    height: u32,
    format: PhotoFormat,
) -> usize {
    let format_str = match format {
        PhotoFormat::Png => "png",
        PhotoFormat::Jpeg => "jpeg",
        PhotoFormat::Gif => "gif",
    };
    let mut hasher = Sha256::new();
    hasher.update(scene.as_str().as_bytes());
    hasher.update(b"|");
    hasher.update(width.to_le_bytes());
    hasher.update(b"|");
    hasher.update(height.to_le_bytes());
    hasher.update(b"|");
    hasher.update(format_str.as_bytes());
    let digest = hasher.finalize();
    let mut n = 0u64;
    for (i, b) in digest.iter().take(8).enumerate() {
        n |= u64::from(*b) << (8 * i);
    }
    usize::try_from(n).unwrap_or(0) % count
}

fn encode_image(img: &DynamicImage, format: PhotoFormat) -> Result<Vec<u8>, CoreError> {
    let mut bytes = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);
    let res = match format {
        PhotoFormat::Jpeg => {
            DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut cursor, format.image_format())
        }
        PhotoFormat::Gif => {
            DynamicImage::ImageRgba8(img.to_rgba8()).write_to(&mut cursor, format.image_format())
        }
        PhotoFormat::Png => img.write_to(&mut cursor, format.image_format()),
    };
    res.map_err(|_| {
        CoreError::new(CoreErrorKind::Internal(
            "failed to encode image".to_string(),
        ))
    })?;
    Ok(bytes)
}

fn parse_biz<T: DeserializeOwned>(value: &serde_json::Value) -> Result<T, CoreError> {
    serde_json::from_value(value.clone())
        .map_err(|e| CoreError::new(CoreErrorKind::Unprocessable(e.to_string())))
}

fn encode_resp<T: Serialize>(resp: T) -> Result<serde_json::Value, CoreError> {
    serde_json::to_value(resp).map_err(|_| {
        CoreError::new(CoreErrorKind::Internal(
            "failed to serialize response".to_string(),
        ))
    })
}
