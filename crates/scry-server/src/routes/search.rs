use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use scry_core::rerank::fuse;
use scry_core::search::{SearchHit, SearchOptions, query_vector, search_with_vector};

use crate::AppState;
use crate::api::{Hit, SearchRequest, SearchResponse};
use crate::error::ApiError;

/// A reranker is an improvement layer: past this budget or on any error
/// the fused order is returned, so search is never worse than without it.
const RERANK_BUDGET: std::time::Duration = std::time::Duration::from_secs(6);

fn truncated<T>(mut hits: Vec<T>, limit: usize) -> Vec<T> {
    hits.truncate(limit);
    hits
}

/// How many fused candidates to retrieve so the reranker, when it runs,
/// sees its full `top_n` pool.
pub(super) fn pool_size(state: &AppState, rerank: bool, limit: usize) -> usize {
    state
        .rerank
        .as_ref()
        .filter(|_| rerank)
        .map_or(limit, |client| limit.max(client.top_n()))
}

pub(super) async fn reranked(
    state: &AppState,
    rerank: bool,
    query: &str,
    hits: Vec<SearchHit>,
    limit: usize,
) -> Vec<SearchHit> {
    let Some(client) = state.rerank.as_ref().filter(|_| rerank) else {
        return hits;
    };
    let documents: Vec<String> = hits.iter().map(|h| client.document(h)).collect();
    match tokio::time::timeout(RERANK_BUDGET, client.rerank(query, &documents)).await {
        Ok(Ok(ranked)) => fuse(hits, &ranked, client.weight(), limit),
        Ok(Err(error)) => {
            tracing::warn!("rerank failed, returning fused order: {error}");
            truncated(hits, limit)
        }
        Err(_) => {
            tracing::warn!("rerank exceeded {RERANK_BUDGET:?}, returning fused order");
            truncated(hits, limit)
        }
    }
}

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
    let pool = pool_size(&state, request.rerank, request.limit);
    let (query, limit, rerank) = (request.query.clone(), request.limit, request.rerank);
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
                limit: pool,
                path_prefix: request.path_prefix.clone(),
            };
            search_with_vector(store, repo_id, &request.query, &vector, &options).map(Some)
        })
        .await?
        .ok_or_else(|| ApiError::NotFound("repo not indexed".to_string()))?;
    let hits = reranked(&state, rerank, &query, hits, limit).await;
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
