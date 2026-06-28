//! Unified API error type mapping to HTTP responses.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ApiError {
    #[error("invalid credentials")]
    Unauthorized,
    #[error("authentication required")]
    NotAuthenticated,
    #[error("insufficient permissions")]
    Forbidden,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("password hashing error: {0}")]
    Hash(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    error: &'static str,
    message: String,
}

impl ApiError {
    fn status_and_kind(&self) -> (StatusCode, &'static str) {
        match self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            ApiError::NotAuthenticated => (StatusCode::UNAUTHORIZED, "not_authenticated"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            ApiError::Database(_) | ApiError::Hash(_) | ApiError::Io(_) | ApiError::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, kind) = self.status_and_kind();
        // Don't leak internal details in production bodies.
        let message = match &self {
            ApiError::Database(e) => {
                tracing::error!(error = %e, "database error");
                "database error".to_string()
            }
            ApiError::Hash(e) => {
                tracing::error!(error = %e, "hash error");
                "internal error".to_string()
            }
            ApiError::Io(e) => {
                tracing::error!(error = %e, "io error");
                "internal error".to_string()
            }
            ApiError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                "internal error".to_string()
            }
            other => other.to_string(),
        };
        (status, Json(ApiErrorBody { error: kind, message })).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}
