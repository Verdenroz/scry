use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::api::ErrorResponse;

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    Unauthorized,
    Unavailable(String),
    Internal(String),
}

impl From<scry_core::Error> for ApiError {
    fn from(error: scry_core::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotFound(message) => (StatusCode::NOT_FOUND, message),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "invalid or missing token".into()),
            Self::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        (status, Json(ErrorResponse { error: message })).into_response()
    }
}
