use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use scry_core::search::{SearchOptions, query_vector, search_with_vector};

use crate::AppState;
use crate::api::{Hit, SearchRequest, SearchResponse};
use crate::error::ApiError;

pub async fn search(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    let vector = query_vector(
        state.embedder.as_ref(),
        state.chat.as_ref(),
        state.hyde,
        &request.query,
    )
    .await?;
    let hits = state
        .store
        .call(move |store| {
            let repo_id = match &request.repo_key {
                Some(key) => match store.repo_id(key)? {
                    Some(id) => Some(id),
                    None => return Ok(None),
                },
                None => None,
            };
            let options = SearchOptions {
                limit: request.limit,
                path_prefix: request.path_prefix.clone(),
            };
            search_with_vector(store, repo_id, &request.query, &vector, &options).map(Some)
        })
        .await?
        .ok_or_else(|| ApiError::NotFound("repo not indexed".to_string()))?;
    Ok(Json(SearchResponse {
        hits: hits
            .into_iter()
            .map(|hit| Hit {
                repo_key: hit.repo_key,
                relpath: hit.relpath,
                start_line: hit.start_line,
                end_line: hit.end_line,
                symbol: hit.symbol,
                score: hit.score,
                content: hit.content,
            })
            .collect(),
    }))
}
