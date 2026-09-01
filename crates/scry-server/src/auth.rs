use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::error::ApiError;

pub async fn require_bearer(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(expected) = &state.auth_token else {
        return Ok(next.run(request).await);
    };
    let presented = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match presented {
        Some(token) if token.as_bytes().ct_eq(expected.as_bytes()).into() => {
            Ok(next.run(request).await)
        }
        _ => Err(ApiError::Unauthorized),
    }
}
