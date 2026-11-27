use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use scry_core::config::Config;
use scry_core::embed::HttpEmbedder;

pub async fn run(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: scry reembed");
    }
    let config = Config::load(None)?;
    if let Err(e) = TcpListener::bind(&config.server.listen) {
        bail!(
            "cannot bind {} ({e}); stop `scry serve` before reembedding",
            config.server.listen
        );
    }
    let db = &config.server.db_path;
    let backup = backup_path(db);
    let embedding = &config.embedding;
    println!(
        "reembedding {} with {} ({}-dim); backup at {}",
        db.display(),
        embedding.model,
        embedding.dim,
        backup.display()
    );
    let embedder = HttpEmbedder::new(embedding.clone());
    let outcome = scry_core::reembed::run(
        db,
        &backup,
        &embedder,
        &embedding.model,
        &embedding.document_prefix,
        |done, total| eprint!("\rembedded {done}/{total}"),
    )
    .await?;
    eprintln!();
    println!(
        "moved {} chunks and {} memories from {} ({}-dim) to {} ({}-dim)",
        outcome.chunks,
        outcome.memories,
        outcome.previous_model,
        outcome.previous_dim,
        embedding.model,
        embedding.dim
    );
    Ok(())
}

fn backup_path(db: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let name = db
        .file_name()
        .map_or("index.db".into(), |n| n.to_string_lossy());
    db.with_file_name(format!("{name}.pre-reembed-{stamp}"))
}
