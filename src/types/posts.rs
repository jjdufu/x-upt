use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PostSort {
    IdAsc,
    IdDesc,
    TitleAsc,
    TitleDesc,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostsRequest {
    pub keyword: String,
    pub user_id: u64,
    pub page: u32,
    pub page_size: u32,
    pub sort: PostSort,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct Post {
    #[serde(rename = "userId")]
    pub user_id: u64,
    pub id: u64,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PostsResponse {
    pub items: Vec<Post>,
    pub total: usize,
    pub page: u32,
    pub page_size: u32,
}
