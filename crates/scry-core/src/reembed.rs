//! Moves an existing db to a new embedding model or dimension by rebuilding
//! every chunk and memory vector from the text the db already holds, so
//! memories, work lists and repos indexed from other machines survive.

use std::path::Path;

use crate::Result;
use crate::embed::Embedder;
use crate::index::embed_input;
use crate::memory::embed_text;
use crate::store::Store;

#[derive(Debug, PartialEq, Eq)]
pub struct ReembedOutcome {
    pub previous_model: String,
    pub previous_dim: usize,
    pub chunks: usize,
    pub memories: usize,
}

/// Inputs embedded between progress reports.
const PROGRESS_BATCH: usize = 512;

/// Backs the db up to `backup` before touching it, then swaps in vectors
/// from `embedder` stamped as `embedding_model` and `document_prefix`.
/// `progress` gets (embedded, total) after each batch. Nothing in the db
/// changes unless every input embeds and the db did not change meanwhile.
pub async fn run(
    db: &Path,
    backup: &Path,
    embedder: &dyn Embedder,
    embedding_model: &str,
    document_prefix: &str,
    progress: impl Fn(usize, usize),
) -> Result<ReembedOutcome> {
    let (previous_model, previous_dim, chunks, memories) = {
        let store = Store::open_for_reembed(db)?;
        store.backup_to(backup)?;
        store.migrate_schema()?;
        let (model, dim) = store.embedding_meta()?;
        (model, dim, store.chunk_sources()?, store.memory_sources()?)
    };

    let inputs: Vec<String> = chunks
        .iter()
        .map(|c| embed_input(&c.repo_key, &c.relpath, c.symbol.as_deref(), &c.content))
        .chain(memories.iter().map(|m| embed_text(&m.kind, &m.content)))
        .collect();
    let mut vectors = Vec::with_capacity(inputs.len());
    for batch in inputs.chunks(PROGRESS_BATCH) {
        vectors.extend(embedder.embed(batch).await?);
        progress(vectors.len(), inputs.len());
    }
    let memory_vectors = vectors.split_off(chunks.len());

    let outcome = ReembedOutcome {
        previous_model,
        previous_dim,
        chunks: chunks.len(),
        memories: memories.len(),
    };
    let chunks: Vec<_> = chunks.into_iter().zip(vectors).collect();
    let memories: Vec<_> = memories.into_iter().zip(memory_vectors).collect();
    let mut store = Store::open_for_reembed(db)?;
    store.replace_all_vectors(
        embedding_model,
        embedder.dim(),
        document_prefix,
        &chunks,
        &memories,
    )?;
    Ok(outcome)
}
