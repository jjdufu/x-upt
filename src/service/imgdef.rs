use std::path::Path;

use base64::Engine;
use image::DynamicImage;
use image::imageops::FilterType;

use crate::service::ServiceError;
use crate::service::imgs;
use crate::types::imgdef::{ImgdefRequest, ImgdefResponse};

fn average_hash(img: &DynamicImage) -> Option<u64> {
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
            hash |= 1 << i;
        }
    }
    Some(hash)
}

fn hamming_distance(h1: u64, h2: u64) -> u32 {
    (h1 ^ h2).count_ones()
}

pub fn compare(req: &ImgdefRequest, imgs_dir: &str) -> Result<ImgdefResponse, ServiceError> {
    if req.threshold > 64 {
        return Err(ServiceError::bad_request(
            "threshold must be between 0 and 64",
        ));
    }
    if req.filename.is_empty() {
        return Err(ServiceError::bad_request(
            "filename must be a file name without path",
        ));
    }
    let safe = Path::new(&req.filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if safe != req.filename {
        return Err(ServiceError::bad_request(
            "filename must be a file name without path",
        ));
    }
    if req.image.trim().is_empty() {
        return Err(ServiceError::bad_request("image must be non-empty"));
    }

    let mut b64 = req.image.trim();
    if let Some(idx) = b64.find("base64,") {
        b64 = &b64[idx + 7..];
    }
    let uploaded_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| ServiceError::bad_request("image must be valid base64"))?;
    let uploaded = imgs::load_image_bytes(&uploaded_bytes)
        .map_err(|_| ServiceError::bad_request("image must be a valid image"))?;

    let target_path = Path::new(imgs_dir).join(safe);
    if !target_path.is_file() {
        return Err(ServiceError::not_found("image file not found"));
    }
    let target = imgs::load_image_file(&target_path)?;

    let uploaded_hash = average_hash(&uploaded)
        .ok_or_else(|| ServiceError::bad_request("image must be a valid image"))?;
    let target_hash = average_hash(&target)
        .ok_or_else(|| ServiceError::internal("failed to hash library image"))?;
    let distance = hamming_distance(uploaded_hash, target_hash);

    Ok(ImgdefResponse {
        matched: distance <= req.threshold,
        filename: safe.to_string(),
        distance,
        threshold: req.threshold,
    })
}
