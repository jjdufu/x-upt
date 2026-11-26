use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImgdefRequest {
    pub filename: String,
    pub image: String,
    pub threshold: u32,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ImgdefResponse {
    pub matched: bool,
    pub filename: String,
    pub distance: u32,
    pub threshold: u32,
}
