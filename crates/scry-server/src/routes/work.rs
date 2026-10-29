use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use super::memories::not_found_if_config;
use crate::AppState;
use crate::api::{
    WorkAddRequest, WorkCloseRequest, WorkItemResponse, WorkListNameResponse, WorkListRequest,
    WorkListResponse, WorkSetRequest, WorkStateRequest,
};
use crate::error::ApiError;

pub async fn work_lists(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkListRequest>,
) -> Result<Json<WorkListResponse>, ApiError> {
    let lists = state
        .store
        .call(move |store| match store.repo_id(&request.repo_key)? {
            Some(repo_id) => store.work_lists(repo_id, request.name.as_deref(), request.all),
            None => Ok(Vec::new()),
        })
        .await?;
    Ok(Json(WorkListResponse {
        lists,
        now: scry_core::memory::now_epoch(),
    }))
}

pub async fn work_add(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkAddRequest>,
) -> Result<Json<WorkItemResponse>, ApiError> {
    let id = state
        .store
        .call(move |store| {
            let repo_id = store.upsert_repo(&request.repo_key)?;
            store.add_work_item(
                repo_id,
                &request.list,
                &request.task,
                &request.status,
                request.note.as_deref(),
            )
        })
        .await
        .map_err(not_found_if_config)?;
    Ok(Json(WorkItemResponse { id }))
}

pub async fn work_set(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkSetRequest>,
) -> Result<Json<WorkItemResponse>, ApiError> {
    let id = request.id;
    state
        .store
        .call(move |store| {
            let repo_id = store
                .repo_id(&request.repo_key)?
                .ok_or_else(|| scry_core::Error::Config(format!("no work item #{id}")))?;
            store.update_work_item(
                repo_id,
                id,
                request.status.as_deref(),
                request.note.as_deref(),
            )
        })
        .await
        .map_err(not_found_if_config)?;
    Ok(Json(WorkItemResponse { id }))
}

pub async fn work_state(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkStateRequest>,
) -> Result<Json<WorkListNameResponse>, ApiError> {
    let list = request.list.clone();
    state
        .store
        .call(move |store| {
            let repo_id = store.upsert_repo(&request.repo_key)?;
            store.set_work_state(repo_id, &request.list, &request.entries)
        })
        .await?;
    Ok(Json(WorkListNameResponse { list }))
}

pub async fn work_close(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkCloseRequest>,
) -> Result<Json<WorkListNameResponse>, ApiError> {
    let list = request.list.clone();
    state
        .store
        .call(move |store| {
            let repo_id = store.repo_id(&request.repo_key)?.ok_or_else(|| {
                scry_core::Error::Config(format!("no work list '{}'", request.list))
            })?;
            store.close_work_list(repo_id, &request.list, !request.reopen)
        })
        .await
        .map_err(not_found_if_config)?;
    Ok(Json(WorkListNameResponse { list }))
}
