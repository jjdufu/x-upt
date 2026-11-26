use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhotoScene {
    ArticleCover,
    Avatar,
    Banner,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PhotoFormat {
    Png,
    Jpeg,
    Gif,
}

impl PhotoFormat {
    #[must_use]
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
        }
    }

    #[must_use]
    pub fn image_format(self) -> image::ImageFormat {
        match self {
            Self::Png => image::ImageFormat::Png,
            Self::Jpeg => image::ImageFormat::Jpeg,
            Self::Gif => image::ImageFormat::Gif,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhotosRequest {
    pub scene: PhotoScene,
    pub width: u32,
    pub height: u32,
    pub format: PhotoFormat,
}

#[derive(Debug, Serialize)]
pub struct ImageInfo {
    pub filename: String,
    pub mime: String,
    pub size: u64,
    pub base64: String,
}

#[derive(Debug, Serialize)]
pub struct PhotosResponse {
    pub scene: PhotoScene,
    pub width: u32,
    pub height: u32,
    pub format: PhotoFormat,
    pub image: ImageInfo,
}
