//! Index pipeline: walk, hash-diff against the store, chunk changed files,
//! embed only chunks whose vector is not already known, upsert. The
//! per-file path is shared by local indexing and the server sync endpoint.

use std::collections::HashMap;
use std::path::Path;

use crate::chunker::chunk_file;
use crate::config::IndexConfig;
use crate::embed::Embedder;
use crate::hashing;
use crate::store::{NewChunk, Store};
use crate::walk::walk_repo;
use crate::{Error, Result};

/// Embedding inputs are cut here; a longer chunk only occurs on minified
/// or generated content where the prefix carries the signal anyway.
const EMBED_MAX_CHARS: usize = 6000;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct IndexOutcome {
    pub indexed_files: usize,
    pub unchanged_files: usize,
    pub deleted_files: usize,
    pub embedded_chunks: usize,
    pub reused_chunks: usize,
}

pub fn embed_input(repo_key: &str, relpath: &str, symbol: Option<&str>, content: &str) -> String {
    let mut text = format!("{repo_key} > {relpath}");
    if let Some(symbol) = symbol {
        text.push_str(" > ");
        text.push_str(symbol);
    }
    text.push('\n');
    text.push_str(content);
    if text.len() > EMBED_MAX_CHARS {
        let cut = (0..=EMBED_MAX_CHARS)
            .rev()
            .find(|i| text.is_char_boundary(*i));
        text.truncate(cut.unwrap_or(0));
    }
    text
}

pub struct PreparedFile {
    pub chunks: Vec<crate::chunker::Chunk>,
    pub inputs: Vec<String>,
    pub hashes: Vec<String>,
}

pub fn prepare_file(repo_key: &str, relpath: &str, content: &str) -> PreparedFile {
    let chunks = chunk_file(relpath, content);
    let inputs: Vec<String> = chunks
        .iter()
        .map(|c| embed_input(repo_key, relpath, c.symbol.as_deref(), &c.content))
        .collect();
    let hashes = inputs
        .iter()
        .map(|input| hashing::hex(hashing::hash_bytes(input.as_bytes())))
        .collect();
    PreparedFile {
        chunks,
        inputs,
        hashes,
    }
}

pub type EmbeddingMap = HashMap<String, Vec<f32>>;

#[derive(Debug, Clone)]
pub struct FileMeta {
    pub relpath: String,
    pub xxh64: String,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Splits a prepared file's chunks into vectors already in the store and
/// (hash, input) pairs that still need embedding.
pub fn known_vectors(
    store: &Store,
    prepared: &PreparedFile,
) -> Result<(EmbeddingMap, Vec<(String, String)>)> {
    let mut vectors = HashMap::new();
    let mut missing: Vec<(String, String)> = Vec::new();
    for (input, hash) in prepared.inputs.iter().zip(&prepared.hashes) {
        if vectors.contains_key(hash) || missing.iter().any(|(h, _)| h == hash) {
            continue;
        }
        match store.vector_for_hash(hash)? {
            Some(vector) => {
                vectors.insert(hash.clone(), vector);
            }
            None => missing.push((hash.clone(), input.clone())),
        }
    }
    Ok((vectors, missing))
}

pub fn commit_file(
    store: &mut Store,
    repo_id: i64,
    meta: &FileMeta,
    prepared: PreparedFile,
    vectors: &EmbeddingMap,
) -> Result<()> {
    let new_chunks: Vec<NewChunk> = prepared
        .chunks
        .into_iter()
        .zip(&prepared.hashes)
        .map(|(chunk, hash)| NewChunk {
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            symbol: chunk.symbol,
            content: chunk.content,
            content_hash: hash.clone(),
            embedding: vectors[hash].clone(),
        })
        .collect();
    let file_id = store.upsert_file(
        repo_id,
        &meta.relpath,
        &meta.xxh64,
        meta.size,
        meta.mtime_ms,
    )?;
    store.replace_file_chunks(file_id, repo_id, &new_chunks)
}

/// Chunks, embeds, and stores one file's content. Returns
/// (embedded, reused) chunk counts.
pub async fn index_file_content(
    store: &mut Store,
    embedder: &dyn Embedder,
    repo_key: &str,
    repo_id: i64,
    meta: &FileMeta,
    content: &str,
) -> Result<(usize, usize)> {
    let prepared = prepare_file(repo_key, &meta.relpath, content);
    let (mut vectors, missing) = known_vectors(store, &prepared)?;
    let reused = vectors.len();
    let mut embedded = 0;
    if !missing.is_empty() {
        let texts: Vec<String> = missing.iter().map(|(_, input)| input.clone()).collect();
        let fresh = embedder.embed(&texts).await?;
        embedded = fresh.len();
        for ((hash, _), vector) in missing.into_iter().zip(fresh) {
            vectors.insert(hash, vector);
        }
    }
    commit_file(store, repo_id, meta, prepared, &vectors)?;
    Ok((embedded, reused))
}

pub async fn index_repo(
    store: &mut Store,
    embedder: &dyn Embedder,
    repo_key: &str,
    root: &Path,
    config: &IndexConfig,
) -> Result<IndexOutcome> {
    let entries = walk_repo(root, config.max_file_size)?;
    if entries.len() > config.max_file_count {
        return Err(Error::Config(format!(
            "{} files exceed max_file_count {}; raise [index] max_file_count or add ignores",
            entries.len(),
            config.max_file_count
        )));
    }

    let repo_id = store.upsert_repo(repo_key)?;
    let stored: HashMap<String, String> = store
        .list_files(repo_id)?
        .into_iter()
        .map(|f| (f.relpath, f.xxh64))
        .collect();

    let mut outcome = IndexOutcome::default();
    let on_disk: std::collections::HashSet<&str> =
        entries.iter().map(|e| e.relpath.as_str()).collect();
    for relpath in stored.keys().filter(|p| !on_disk.contains(p.as_str())) {
        store.delete_file(repo_id, relpath)?;
        outcome.deleted_files += 1;
    }

    for entry in &entries {
        let path = root.join(&entry.relpath);
        let hash = hashing::hex(hashing::hash_file(&path)?);
        if stored.get(&entry.relpath) == Some(&hash) {
            outcome.unchanged_files += 1;
            continue;
        }
        let Ok(content) = String::from_utf8(std::fs::read(&path)?) else {
            outcome.unchanged_files += 1;
            continue;
        };
        let meta = FileMeta {
            relpath: entry.relpath.clone(),
            xxh64: hash,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
        };
        let (embedded, reused) =
            index_file_content(store, embedder, repo_key, repo_id, &meta, &content).await?;
        outcome.embedded_chunks += embedded;
        outcome.reused_chunks += reused;
        outcome.indexed_files += 1;
    }
    Ok(outcome)
}
