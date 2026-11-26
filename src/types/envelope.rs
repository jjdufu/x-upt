use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Envelope<T: Serialize> {
    pub code: i32,
    pub msg: String,
    pub request_id: String,
    pub data: T,
}

pub const CODE_OK: i32 = 0;
pub const CODE_BAD_REQUEST: i32 = 40001;
pub const CODE_UNAUTHORIZED: i32 = 40100;
pub const CODE_INVALID_TOKEN: i32 = 40102;
pub const CODE_FORBIDDEN: i32 = 40300;
pub const CODE_UNKNOWN_METHOD: i32 = 40400;
pub const CODE_NOT_FOUND: i32 = 40401;
pub const CODE_UNPROCESSABLE: i32 = 42200;
pub const CODE_INTERNAL: i32 = 50000;

pub const MSG_OK: &str = "ok";
