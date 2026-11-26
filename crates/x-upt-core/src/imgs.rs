//! Image decoding, PNG trailing-byte trimming, and 64-bit perceptual average hash (aHash).

use image::DynamicImage;
use image::imageops::FilterType;
use serde::{Deserialize, Serialize};

const PNG_SIG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Scene category assigned to an imported benchmark image.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PhotoScene {
    /// Article cover scene.
    ArticleCover,
    /// User avatar scene.
    Avatar,
    /// Wide banner scene.
    Banner,
}

impl PhotoScene {
    /// Canonical snake_case identifier.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ArticleCover => "article_cover",
            Self::Avatar => "avatar",
            Self::Banner => "banner",
        }
    }

    /// Parses a scene identifier.
    ///
    /// # Errors
    /// Returns an error string if `raw` is not one of the 3 scenes.
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "article_cover" => Ok(Self::ArticleCover),
            "avatar" => Ok(Self::Avatar),
            "banner" => Ok(Self::Banner),
            other => Err(format!("unknown scene: {other}")),
        }
    }
}

/// Output image format for `x_upt.photos.render`.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PhotoFormat {
    /// PNG format.
    Png,
    /// JPEG format.
    Jpeg,
    /// GIF format.
    Gif,
}

impl PhotoFormat {
    /// HTTP MIME type for the encoded image.
    #[must_use]
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
        }
    }

    /// Corresponding `image::ImageFormat`.
    #[must_use]
    pub fn image_format(self) -> image::ImageFormat {
        match self {
            Self::Png => image::ImageFormat::Png,
            Self::Jpeg => image::ImageFormat::Jpeg,
            Self::Gif => image::ImageFormat::Gif,
        }
    }
}

/// Persisted metadata of an imported benchmark image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageMeta {
    /// Unique UUID string.
    pub id: String,
    /// Sanitized file name (used by `imgdef.compare` and returned by `photos.render`).
    pub filename: String,
    /// Assigned rendering scene.
    pub scene: PhotoScene,
    /// Detected MIME type (`image/png`, `image/jpeg`, `image/gif`).
    pub mime: String,
    /// Original pixel width.
    pub width: u32,
    /// Original pixel height.
    pub height: u32,
    /// Raw byte length.
    pub size: u64,
    /// 16-char lowercase hex of the 64-bit perceptual aHash.
    pub ahash: String,
    /// RFC 3339 import timestamp.
    pub created_at: String,
}

/// In-memory loaded image used by the runtime snapshot.
#[derive(Debug, Clone)]
pub struct RuntimeImage {
    /// Image metadata.
    pub meta: ImageMeta,
    /// Parsed 64-bit average hash.
    pub ahash_u64: u64,
    /// Raw image bytes.
    pub bytes: Vec<u8>,
}

/// Trims zero-padded trailing bytes after a PNG `IEND` chunk.
#[must_use]
pub fn trim_png_trailing(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 8 || !bytes.starts_with(PNG_SIG) {
        return None;
    }
    let mut i = 8usize;
    while i.saturating_add(12) <= bytes.len() {
        let len_slice = bytes.get(i..i + 4)?;
        let len_arr: [u8; 4] = len_slice.try_into().ok()?;
        let len = usize::try_from(u32::from_be_bytes(len_arr)).ok()?;
        let typ = bytes.get(i + 4..i + 8)?;
        let next = i.checked_add(12)?.checked_add(len)?;
        if next > bytes.len() {
            return None;
        }
        if typ == b"IEND" {
            return bytes.get(..next);
        }
        i = next;
    }
    None
}

/// Decodes PNG, JPEG, or GIF bytes into a [`DynamicImage`].
///
/// # Errors
/// Returns an error string when `bytes` cannot be decoded as a supported image.
pub fn load_image_bytes(bytes: &[u8]) -> Result<DynamicImage, String> {
    if let Ok(img) = image::load_from_memory(bytes) {
        return Ok(img);
    }
    if let Some(trimmed) = trim_png_trailing(bytes)
        && let Ok(img) = image::load_from_memory(trimmed)
    {
        return Ok(img);
    }
    Err("image must be a valid PNG, JPEG, or GIF".to_string())
}

/// Detects MIME string for PNG, JPEG, or GIF bytes.
#[must_use]
pub fn detect_mime(bytes: &[u8]) -> &'static str {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Jpeg) => "image/jpeg",
        Ok(image::ImageFormat::Gif) => "image/gif",
        _ => "image/png",
    }
}

/// Computes the 64-bit 8x8 luminance average hash (aHash) of an image.
#[must_use]
pub fn average_hash(img: &DynamicImage) -> Option<u64> {
    let resized = img.resize_exact(8, 8, FilterType::Nearest).to_luma8();
    let pixels = resized.as_raw();
    if pixels.len() != 64 {
        return None;
    }
    let sum: u64 = pixels.iter().map(|&p| u64::from(p)).sum();
    let avg = sum / 64;
    let mut hash = 0u64;
    for (i, &p) in pixels.iter().enumerate() {
        if u64::from(p) >= avg {
            hash |= 1u64 << i;
        }
    }
    Some(hash)
}

/// Computes Hamming distance (`0`..=`64`) between two 64-bit perceptual hashes.
#[must_use]
pub fn hamming_distance(h1: u64, h2: u64) -> u32 {
    (h1 ^ h2).count_ones()
}
