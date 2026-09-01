//! Index pipeline: walk, hash-diff against the store, chunk changed files,
//! embed only chunks whose vector is not already known, upsert.

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

        let chunks = chunk_file(&entry.relpath, &content);
        let inputs: Vec<String> = chunks
            .iter()
            .map(|c| embed_input(repo_key, &entry.relpath, c.symbol.as_deref(), &c.content))
            .collect();
        let hashes: Vec<String> = inputs
            .iter()
            .map(|input| hashing::hex(hashing::hash_bytes(input.as_bytes())))
            .collect();

        let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
        let mut missing: Vec<(String, String)> = Vec::new();
        for (input, hash) in inputs.iter().zip(&hashes) {
            if vectors.contains_key(hash) || missing.iter().any(|(h, _)| h == hash) {
                continue;
            }
            match store.vector_for_hash(hash)? {
                Some(vector) => {
                    vectors.insert(hash.clone(), vector);
                    outcome.reused_chunks += 1;
                }
                None => missing.push((hash.clone(), input.clone())),
            }
        }
        if !missing.is_empty() {
            let texts: Vec<String> = missing.iter().map(|(_, input)| input.clone()).collect();
            let embedded = embedder.embed(&texts).await?;
            outcome.embedded_chunks += embedded.len();
            for ((hash, _), vector) in missing.into_iter().zip(embedded) {
                vectors.insert(hash, vector);
            }
        }

        let new_chunks: Vec<NewChunk> = chunks
            .into_iter()
            .zip(&hashes)
            .map(|(chunk, hash)| NewChunk {
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                symbol: chunk.symbol,
                content: chunk.content,
                content_hash: hash.clone(),
                embedding: vectors[hash].clone(),
            })
            .collect();

        let file_id =
            store.upsert_file(repo_id, &entry.relpath, &hash, entry.size, entry.mtime_ms)?;
        store.replace_file_chunks(file_id, repo_id, &new_chunks)?;
        outcome.indexed_files += 1;
    }
    Ok(outcome)
}
