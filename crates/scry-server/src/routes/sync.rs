use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use scry_core::index::{FileMeta, commit_file, known_vectors, prepare_file};

use crate::AppState;
use crate::api::{
    ManifestEntry, ManifestRequest, ManifestResponse, PruneRequest, PruneResponse, SyncRequest,
    SyncResponse,
};
use crate::error::ApiError;

pub async fn prune(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PruneRequest>,
) -> Result<Json<PruneResponse>, ApiError> {
    let deleted_files = state
        .store
        .call(move |store| {
            let migrate_to = match &request.migrate_memories_to {
                Some(key) => Some(store.upsert_repo(key)?),
                None => None,
            };
            store.prune_repo(&request.repo_key, migrate_to)
        })
        .await?;
    Ok(Json(PruneResponse { deleted_files }))
}

pub async fn manifest(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ManifestRequest>,
) -> Result<Json<ManifestResponse>, ApiError> {
    let files = state
        .store
        .call(move |store| -> scry_core::Result<Vec<ManifestEntry>> {
            let Some(repo_id) = store.repo_id(&request.repo_key)? else {
                return Ok(Vec::new());
            };
            Ok(store
                .list_files(repo_id)?
                .into_iter()
                .map(|f| ManifestEntry {
                    relpath: f.relpath,
                    xxh64: f.xxh64,
                    size: f.size,
                    mtime_ms: f.mtime,
                })
                .collect())
        })
        .await?;
    Ok(Json(ManifestResponse { files }))
}

pub async fn sync(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SyncRequest>,
) -> Result<Json<SyncResponse>, ApiError> {
    let repo_key = request.repo_key.clone();
    let deletes = request.deletes.clone();
    let (repo_id, deleted_files) = state
        .store
        .call(move |store| -> scry_core::Result<(i64, usize)> {
            let repo_id = store.upsert_repo(&repo_key)?;
            for relpath in &deletes {
                store.delete_file(repo_id, relpath)?;
            }
            Ok((repo_id, deletes.len()))
        })
        .await?;

    let mut response = SyncResponse {
        deleted_files,
        ..SyncResponse::default()
    };
    for upload in request.upserts {
        let prepared = prepare_file(&request.repo_key, &upload.relpath, &upload.content);
        let (prepared, mut vectors, missing) = state
            .store
            .call(move |store| {
                let resolved = known_vectors(store, &prepared);
                resolved.map(|(vectors, missing)| (prepared, vectors, missing))
            })
            .await?;
        response.reused_chunks += vectors.len();
        if !missing.is_empty() {
            let texts: Vec<String> = missing.iter().map(|(_, input)| input.clone()).collect();
            let fresh = state.embedder.embed(&texts).await?;
            response.embedded_chunks += fresh.len();
            for ((hash, _), vector) in missing.into_iter().zip(fresh) {
                vectors.insert(hash, vector);
            }
        }
        let meta = FileMeta {
            relpath: upload.relpath,
            xxh64: upload.xxh64,
            size: upload.size,
            mtime_ms: upload.mtime_ms,
        };
        state
            .store
            .call(move |store| commit_file(store, repo_id, &meta, prepared, &vectors))
            .await?;
        response.indexed_files += 1;
    }
    Ok(Json(response))
}
