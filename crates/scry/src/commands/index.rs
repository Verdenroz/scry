use std::collections::HashMap;

use anyhow::{Result, bail};
use scry_core::hashing;
use scry_core::walk::walk_repo;
use scry_server::api::{FileUpload, SyncRequest, SyncResponse};

use super::RepoContext;
use super::repo_context;

const BATCH_FILES: usize = 16;
const BATCH_BYTES: usize = 2 * 1024 * 1024;

pub async fn run(args: &[String]) -> Result<()> {
    let full = args.iter().any(|a| a == "--full");
    let ctx = repo_context()?;
    if super::at_or_above_home(&ctx.identity.root) {
        bail!(
            "refusing to index {} (at or above your home directory); cd into a project repo",
            ctx.identity.root.display()
        );
    }
    let outcome = sync_repo(&ctx, full).await?;
    println!(
        "indexed {} files ({} embedded, {} reused chunks), deleted {}, unchanged {}",
        outcome.indexed_files,
        outcome.embedded_chunks,
        outcome.reused_chunks,
        outcome.deleted_files,
        outcome.unchanged
    );
    Ok(())
}

#[derive(Debug, Default)]
pub struct SyncOutcome {
    pub indexed_files: usize,
    pub deleted_files: usize,
    pub embedded_chunks: usize,
    pub reused_chunks: usize,
    pub unchanged: usize,
}

/// `full` re-uploads every file regardless of the manifest, which is how
/// a chunker change reaches files whose content has not changed.
pub async fn sync_repo(ctx: &RepoContext, full: bool) -> Result<SyncOutcome> {
    let entries = walk_repo(&ctx.identity.root, ctx.config.index.max_file_size)?;
    if entries.len() > ctx.config.index.max_file_count {
        bail!(
            "{} files exceed max_file_count {}; raise [index] max_file_count or add ignores",
            entries.len(),
            ctx.config.index.max_file_count
        );
    }

    let manifest: HashMap<String, String> = ctx
        .client
        .manifest(&ctx.identity.key)
        .await?
        .files
        .into_iter()
        .map(|f| (f.relpath, f.xxh64))
        .collect();

    let mut outcome = SyncOutcome::default();
    let on_disk: std::collections::HashSet<&str> =
        entries.iter().map(|e| e.relpath.as_str()).collect();
    let mut deletes: Vec<String> = manifest
        .keys()
        .filter(|p| !on_disk.contains(p.as_str()))
        .cloned()
        .collect();

    let mut uploads: Vec<FileUpload> = Vec::new();
    for entry in &entries {
        let path = ctx.identity.root.join(&entry.relpath);
        let hash = hashing::hex(hashing::hash_file(&path)?);
        if !full && manifest.get(&entry.relpath) == Some(&hash) {
            outcome.unchanged += 1;
            continue;
        }
        let Ok(content) = String::from_utf8(std::fs::read(&path)?) else {
            outcome.unchanged += 1;
            continue;
        };
        uploads.push(FileUpload {
            relpath: entry.relpath.clone(),
            content,
            xxh64: hash,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
        });
    }

    let mut batch: Vec<FileUpload> = Vec::new();
    let mut batch_bytes = 0;
    let flush = async |batch: Vec<FileUpload>, deletes: Vec<String>| -> Result<SyncResponse> {
        ctx.client
            .sync(&SyncRequest {
                repo_key: ctx.identity.key.clone(),
                upserts: batch,
                deletes,
            })
            .await
    };
    for upload in uploads {
        if batch.len() >= BATCH_FILES || batch_bytes + upload.content.len() > BATCH_BYTES {
            let response = flush(std::mem::take(&mut batch), std::mem::take(&mut deletes)).await?;
            outcome.absorb(&response);
            batch_bytes = 0;
        }
        batch_bytes += upload.content.len();
        batch.push(upload);
    }
    if !batch.is_empty() || !deletes.is_empty() {
        let response = flush(batch, deletes).await?;
        outcome.absorb(&response);
    }
    Ok(outcome)
}

impl SyncOutcome {
    fn absorb(&mut self, response: &SyncResponse) {
        self.indexed_files += response.indexed_files;
        self.deleted_files += response.deleted_files;
        self.embedded_chunks += response.embedded_chunks;
        self.reused_chunks += response.reused_chunks;
    }
}
