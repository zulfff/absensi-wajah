//! API error type and its HTTP mapping.
//!
//! Every error carries a stable `code` so the client can react, and a message
//! safe to show. Internal DB errors are logged, not leaked.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("too many requests")]
    TooManyRequests,

    #[error("payload too large")]
    PayloadTooLarge,

    #[error("internal error")]
    Internal(#[from] anyhow::Error),

    #[error(transparent)]
    Db(#[from] db::DbError),

    #[error(transparent)]
    Face(#[from] face_core::FaceError),
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl ApiError {
    fn code(&self) -> &'static str {
        match self {
            ApiError::NotFound(_) => "not_found",
            ApiError::BadRequest(_) => "bad_request",
            ApiError::Unauthorized => "unauthorized",
            ApiError::Forbidden => "forbidden",
            ApiError::Conflict(_) => "conflict",
            ApiError::TooManyRequests => "too_many_requests",
            ApiError::PayloadTooLarge => "payload_too_large",
            ApiError::Internal(_) => "internal",
            ApiError::Db(_) => "internal",
            ApiError::Face(_) => "face_error",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            ApiError::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ApiError::Internal(_) | ApiError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::Face(_) => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }

    /// A message safe to return to the client.
    pub fn public_message(&self) -> String {
        match self {
            ApiError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                "terjadi kesalahan internal".into()
            }
            ApiError::Db(e) => {
                tracing::error!(error = %e, "database error");
                "terjadi kesalahan basis data".into()
            }
            ApiError::Face(e) => {
                tracing::warn!(error = %e, "face pipeline error");
                e.to_string()
            }
            other => other.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ErrorBody {
            code: self.code(),
            message: self.public_message(),
        };
        (status, Json(body)).into_response()
    }
}
