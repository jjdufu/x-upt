use std::io::Cursor;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use base64::Engine;
use image::{DynamicImage, imageops::FilterType};
use sha2::{Digest, Sha256};

use crate::service::ServiceError;
use crate::service::imgs;
use crate::types::photos::{ImageInfo, PhotoFormat, PhotoScene, PhotosRequest, PhotosResponse};

pub const MIN_EDGE: u32 = 8;
pub const MAX_EDGE: u32 = 1024;

#[must_use]
pub fn scene_indices(scene: PhotoScene) -> RangeInclusive<u32> {
    match scene {
        PhotoScene::ArticleCover => 1..=40,
        PhotoScene::Avatar => 41..=70,
        PhotoScene::Banner => 71..=100,
    }
}

fn scene_key(scene: PhotoScene) -> &'static str {
    match scene {
        PhotoScene::ArticleCover => "article_cover",
        PhotoScene::Avatar => "avatar",
        PhotoScene::Banner => "banner",
    }
}

fn format_key(format: PhotoFormat) -> &'static str {
    match format {
        PhotoFormat::Png => "png",
        PhotoFormat::Jpeg => "jpeg",
        PhotoFormat::Gif => "gif",
    }
}

fn scene_files(imgs_dir: &Path, scene: PhotoScene) -> Vec<PathBuf> {
    scene_indices(scene)
        .map(|i| imgs_dir.join(format!("img_{i:03}.png")))
        .filter(|path| path.is_file())
        .collect()
}

fn pick_index(
    count: usize,
    scene: PhotoScene,
    width: u32,
    height: u32,
    format: PhotoFormat,
) -> usize {
    let mut hasher = Sha256::new();
    hasher.update(scene_key(scene).as_bytes());
    hasher.update(b"|");
    hasher.update(width.to_le_bytes());
    hasher.update(b"|");
    hasher.update(height.to_le_bytes());
    hasher.update(b"|");
    hasher.update(format_key(format).as_bytes());
    let digest = hasher.finalize();
    let mut n = 0u64;
    for (i, b) in digest.iter().take(8).enumerate() {
        n |= u64::from(*b) << (8 * i);
    }
    usize::try_from(n).unwrap_or(0) % count
}

fn encode_image(img: &DynamicImage, format: PhotoFormat) -> Result<Vec<u8>, ServiceError> {
    let mut bytes = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);
    let result = match format {
        PhotoFormat::Jpeg => {
            DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut cursor, format.image_format())
        }
        PhotoFormat::Gif => {
            DynamicImage::ImageRgba8(img.to_rgba8()).write_to(&mut cursor, format.image_format())
        }
        PhotoFormat::Png => img.write_to(&mut cursor, format.image_format()),
    };
    result.map_err(|_| ServiceError::internal("failed to encode image"))?;
    Ok(bytes)
}

pub fn render_photo(req: &PhotosRequest, imgs_dir: &str) -> Result<PhotosResponse, ServiceError> {
    if !(MIN_EDGE..=MAX_EDGE).contains(&req.width) || !(MIN_EDGE..=MAX_EDGE).contains(&req.height) {
        return Err(ServiceError::bad_request(
            "width and height must be between 8 and 1024",
        ));
    }

    let files = scene_files(Path::new(imgs_dir), req.scene);
    if files.is_empty() {
        return Err(ServiceError::not_found("no images found for scene"));
    }

    let idx = pick_index(files.len(), req.scene, req.width, req.height, req.format);
    let chosen = &files[idx];

    let source = imgs::load_image_file(chosen)?;
    let resized = source.resize_exact(req.width, req.height, FilterType::Triangle);
    let encoded = encode_image(&resized, req.format)?;
    let size = u64::try_from(encoded.len()).unwrap_or(u64::MAX);
    let filename = chosen
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(PhotosResponse {
        scene: req.scene,
        width: req.width,
        height: req.height,
        format: req.format,
        image: ImageInfo {
            filename,
            mime: req.format.mime().to_string(),
            size,
            base64: base64::engine::general_purpose::STANDARD.encode(&encoded),
        },
    })
}
