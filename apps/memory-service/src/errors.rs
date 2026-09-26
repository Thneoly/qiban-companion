//! Error surface: storage/domain failures plus the HTTP wire envelope.

use axum::http::StatusCode;

/// Storage and domain failures. Messages are user-facing Chinese, matching
/// the companion-storage crate convention.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("数据库来自更新的服务版本，已保持原样")]
    NewerSchema,
    #[error("无法识别的旧库结构，已保持原样")]
    IncompatibleSchema,
    #[error("未找到该记忆")]
    NotFound,
    #[error("{0}")]
    Validation(&'static str),
    #[error("序号已达上限")]
    CounterOverflow,
    #[error("本地存储暂时不可用")]
    Unavailable,
}

/// JS-safe integer ceiling (2^53-1); counters refuse to wrap instead of
/// losing precision for any JSON consumer.
pub const MAX_COUNTER: i64 = 9_007_199_254_740_991;

/// Static-code HTTP error rendered as `{"error":{"code":...}}`, mirroring the
/// coordinator service envelope.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub &'static str);

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = axum::Json(serde_json::json!({"error": {"code": self.1}}));
        (self.0, body).into_response()
    }
}

impl From<MemoryError> for ApiError {
    fn from(error: MemoryError) -> Self {
        match error {
            MemoryError::NotFound => ApiError(StatusCode::NOT_FOUND, "not_found"),
            MemoryError::Validation(_) => ApiError(StatusCode::BAD_REQUEST, "invalid_request"),
            MemoryError::NewerSchema | MemoryError::IncompatibleSchema => {
                ApiError(StatusCode::SERVICE_UNAVAILABLE, "schema_mismatch")
            }
            MemoryError::CounterOverflow => ApiError(StatusCode::SERVICE_UNAVAILABLE, "capacity"),
            MemoryError::Sql(_) | MemoryError::Io(_) | MemoryError::Unavailable => {
                ApiError(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable")
            }
        }
    }
}
