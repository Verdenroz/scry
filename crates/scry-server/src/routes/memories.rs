use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use scry_core::memory::{
    AnchorSpec, MemoryDraft, MemoryHit, brief as build_brief, embed_text, recall_with_vector,
    remember_with_embedding, validate_kind, validate_source,
};
use scry_core::memory_gc;
use scry_core::store::Scope;

use crate::AppState;
use crate::api::{
    BriefRequest, BriefResponse, FeedbackRequest, FeedbackResponse, GcRequest, GcResponse,
    MemoryDto, MemoryStatsResponse, ReaffirmRequest, ReaffirmResponse, RecallRequest,
    RecallResponse, RememberRequest, RememberResponse, RestoreRequest, RestoreResponse,
    SourceStatsDto, StaleMemoryDto, StaleRequest, StaleResponse, SupersedeRequest,
    SupersedeResponse, VerifyRequest, VerifyResponse,
};
use crate::error::ApiError;

pub async fn remember(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RememberRequest>,
) -> Result<Json<RememberResponse>, ApiError> {
    validate_kind(&request.kind).map_err(|e| ApiError::NotFound(e.to_string()))?;
    validate_source(&request.source).map_err(|e| ApiError::NotFound(e.to_string()))?;
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
                source: request.source,
                evidence: request.evidence,
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
            let scope = store.scope(request.repo_key.as_deref())?;
            recall_with_vector(
                store,
                scope,
                &vector,
                request.limit,
                half_life,
                request.touch,
            )
        })
        .await?;
    Ok(Json(RecallResponse {
        memories: memories.into_iter().map(memory_dto).collect(),
    }))
}

fn memory_dto(hit: MemoryHit) -> MemoryDto {
    MemoryDto {
        id: hit.id,
        kind: hit.kind,
        content: hit.content,
        score: hit.score,
        stale: hit.stale,
    }
}

pub async fn feedback(
    State(state): State<Arc<AppState>>,
    Json(request): Json<FeedbackRequest>,
) -> Result<Json<FeedbackResponse>, ApiError> {
    let updated = state
        .store
        .call(move |store| {
            if store.memory(request.id)?.is_none() {
                return Ok(false);
            }
            let scope = match (&request.repo_key, request.helpful) {
                (Some(key), false) => Scope::Repo(store.upsert_repo(key)?),
                (key, _) => store.scope(key.as_deref())?,
            };
            store.memory_feedback(request.id, request.helpful, scope)
        })
        .await?;
    Ok(Json(FeedbackResponse { updated }))
}

pub async fn brief(
    State(state): State<Arc<AppState>>,
    Json(request): Json<BriefRequest>,
) -> Result<Json<BriefResponse>, ApiError> {
    let half_life = state.memory_config.half_life_days;
    let digest = state
        .store
        .call(move |store| {
            let scope = store.scope(request.repo_key.as_deref())?;
            build_brief(store, scope, request.limit, request.stale_days, half_life)
        })
        .await?;
    Ok(Json(BriefResponse {
        memories: digest.top.into_iter().map(memory_dto).collect(),
        went_stale: digest.went_stale.into_iter().map(memory_dto).collect(),
    }))
}

pub async fn supersede(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SupersedeRequest>,
) -> Result<Json<SupersedeResponse>, ApiError> {
    let superseded = state
        .store
        .call(move |store| store.supersede_memory(request.old_id, request.new_id))
        .await?;
    Ok(Json(SupersedeResponse { superseded }))
}

pub async fn memory_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MemoryStatsResponse>, ApiError> {
    let stats = state
        .store
        .call(|store| store.memory_source_stats())
        .await?;
    Ok(Json(MemoryStatsResponse {
        sources: stats
            .into_iter()
            .map(|s| SourceStatsDto {
                source: s.source,
                memories: s.total,
                stale: s.stale,
                retrievals: s.retrievals,
                helpful: s.helpful,
                noise: s.noise,
                archived: s.archived,
            })
            .collect(),
    }))
}

pub async fn verify(
    State(state): State<Arc<AppState>>,
    Json(request): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, ApiError> {
    let Some(verifier) = &state.verify else {
        return Ok(Json(VerifyResponse {
            configured: false,
            score: None,
            supported: None,
        }));
    };
    let score = verifier.score(&request.claim, &request.evidence).await?;
    Ok(Json(VerifyResponse {
        configured: true,
        score: Some(score),
        supported: Some(score >= verifier.threshold()),
    }))
}

pub async fn reaffirm(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ReaffirmRequest>,
) -> Result<Json<ReaffirmResponse>, ApiError> {
    state
        .store
        .call(move |store| store.reaffirm_memory(request.id, &request.evidence))
        .await
        .map_err(not_found_if_config)?;
    Ok(Json(ReaffirmResponse { id: request.id }))
}

pub async fn stale(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StaleRequest>,
) -> Result<Json<StaleResponse>, ApiError> {
    let rows = state
        .store
        .call(move |store| {
            let repo_id = match &request.repo_key {
                Some(key) => store.repo_id(key)?,
                None => None,
            };
            store.stale_memories(repo_id, &request.files)
        })
        .await?;
    Ok(Json(StaleResponse {
        memories: rows
            .into_iter()
            .map(|(row, relpaths)| StaleMemoryDto {
                id: row.id,
                kind: row.kind,
                content: row.content,
                relpaths,
            })
            .collect(),
    }))
}

pub async fn gc(
    State(state): State<Arc<AppState>>,
    Json(request): Json<GcRequest>,
) -> Result<Json<GcResponse>, ApiError> {
    let config = state.memory_config.clone();
    let report = state
        .store
        .call(move |store| memory_gc::gc(store, &config, request.dry_run))
        .await?;
    Ok(Json(report))
}

pub async fn restore(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RestoreRequest>,
) -> Result<Json<RestoreResponse>, ApiError> {
    let id = request.id;
    let archived = state
        .store
        .call(move |store| store.archived_memory(id))
        .await
        .map_err(not_found_if_config)?;
    let embedding = state
        .embedder
        .embed(&[embed_text(&archived.kind, &archived.content)])
        .await?
        .pop()
        .ok_or_else(|| ApiError::Internal("empty embedding response".to_string()))?;
    let status = state
        .store
        .call(move |store| store.restore_memory(id, &embedding))
        .await
        .map_err(not_found_if_config)?;
    Ok(Json(RestoreResponse { id, status }))
}

fn not_found_if_config(error: scry_core::Error) -> ApiError {
    match error {
        scry_core::Error::Config(message) => ApiError::NotFound(message),
        other => other.into(),
    }
}
