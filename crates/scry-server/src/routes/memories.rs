use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use scry_core::memory::{
    AnchorSpec, MemoryDraft, embed_text, recall_with_vector, remember_with_embedding, validate_kind,
};

use crate::AppState;
use crate::api::{
    FeedbackRequest, FeedbackResponse, MemoryDto, RecallRequest, RecallResponse, RememberRequest,
    RememberResponse,
};
use crate::error::ApiError;

pub async fn remember(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RememberRequest>,
) -> Result<Json<RememberResponse>, ApiError> {
    validate_kind(&request.kind).map_err(|e| ApiError::NotFound(e.to_string()))?;
    let embedding = state
        .embedder
        .embed(&[embed_text(&request.kind, &request.content)])
        .await?
        .pop()
        .ok_or_else(|| ApiError::Internal("empty embedding response".to_string()))?;

    let (id, salience, surprise) = state
        .store
        .call(move |store| {
            let repo_id = match &request.repo_key {
                Some(key) => Some(store.upsert_repo(key)?),
                None => None,
            };
            let draft = MemoryDraft {
                repo_id,
                kind: request.kind,
                content: request.content,
                pain: request.pain,
                cost: request.cost,
                anchors: request
                    .anchors
                    .into_iter()
                    .map(|a| AnchorSpec {
                        relpath: a.relpath,
                        start_line: a.start_line,
                        end_line: a.end_line,
                    })
                    .collect(),
            };
            remember_with_embedding(store, draft, embedding)
        })
        .await?;
    Ok(Json(RememberResponse {
        id,
        salience,
        surprise,
    }))
}

pub async fn recall(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RecallRequest>,
) -> Result<Json<RecallResponse>, ApiError> {
    let vector = state
        .embedder
        .embed(std::slice::from_ref(&request.query))
        .await?
        .pop()
        .ok_or_else(|| ApiError::Internal("empty embedding response".to_string()))?;
    let half_life = state.memory_config.half_life_days;
    let memories = state
        .store
        .call(move |store| {
            let repo_id = match &request.repo_key {
                Some(key) => store.repo_id(key)?,
                None => None,
            };
            recall_with_vector(store, repo_id, &vector, request.limit, half_life)
        })
        .await?;
    Ok(Json(RecallResponse {
        memories: memories
            .into_iter()
            .map(|hit| MemoryDto {
                id: hit.id,
                kind: hit.kind,
                content: hit.content,
                score: hit.score,
                stale: hit.stale,
            })
            .collect(),
    }))
}

pub async fn feedback(
    State(state): State<Arc<AppState>>,
    Json(request): Json<FeedbackRequest>,
) -> Result<Json<FeedbackResponse>, ApiError> {
    let updated = state
        .store
        .call(move |store| store.memory_feedback(request.id, request.helpful))
        .await?;
    Ok(Json(FeedbackResponse { updated }))
}
