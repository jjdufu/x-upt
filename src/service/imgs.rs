use std::path::Path;

use image::DynamicImage;

use crate::service::ServiceError;

const PNG_SIG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[must_use]
pub fn trim_png_trailing(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 8 || !bytes.starts_with(PNG_SIG) {
        return None;
    }
    let mut i = 8usize;
    while i.saturating_add(12) <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?) as usize;
        let typ = bytes.get(i + 4..i + 8)?;
        let next = i.checked_add(12)?.checked_add(len)?;
        if next > bytes.len() {
            return None;
        }
        if typ == b"IEND" {
            return Some(&bytes[..next]);
        }
        i = next;
    }
    None
}

pub fn load_image_bytes(bytes: &[u8]) -> Result<DynamicImage, ServiceError> {
    if let Ok(img) = image::load_from_memory(bytes) {
        return Ok(img);
    }
    if let Some(trimmed) = trim_png_trailing(bytes)
        && let Ok(img) = image::load_from_memory(trimmed)
    {
        return Ok(img);
    }
    Err(ServiceError::bad_request("image must be a valid image"))
}

pub fn load_image_file(path: &Path) -> Result<DynamicImage, ServiceError> {
    let bytes = std::fs::read(path).map_err(|_| ServiceError::internal("failed to read image"))?;
    match load_image_bytes(&bytes) {
        Ok(img) => Ok(img),
        Err(_) => Err(ServiceError::internal("failed to decode library image")),
    }
}
