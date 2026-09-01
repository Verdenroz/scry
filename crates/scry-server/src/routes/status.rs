use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::AppState;
use crate::api::StatusResponse;
use crate::error::ApiError;

pub async fn status(State(state): State<Arc<AppState>>) -> Result<Json<StatusResponse>, ApiError> {
    let ((repos, files, chunks), (memories, stale_memories)) = state
        .store
        .call(|store| store.counts().and_then(|c| Ok((c, store.memory_counts()?))))
        .await?;
    Ok(Json(StatusResponse {
        repos,
        files,
        chunks,
        memories,
        stale_memories,
    }))
}
