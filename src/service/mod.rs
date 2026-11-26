pub mod apps;
pub mod imgdef;
pub mod imgs;
pub mod photos;
pub mod posts;
mod posts_gen;

#[derive(Debug)]
pub enum ServiceError {
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

impl ServiceError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}
