use std::time::Duration;

use anyhow::{Result, bail};
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;

use super::{index::sync_repo, repo_context};

pub async fn run() -> Result<()> {
    let ctx = repo_context()?;
    if super::at_or_above_home(&ctx.identity.root) {
        bail!(
            "refusing to watch {} (at or above your home directory)",
            ctx.identity.root.display()
        );
    }

    let outcome = sync_repo(&ctx, false).await?;
    println!(
        "watching {} (key {}): {} files indexed, {} unchanged",
        ctx.identity.root.display(),
        ctx.identity.key,
        outcome.indexed_files,
        outcome.unchanged
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    let mut debouncer = new_debouncer(
        Duration::from_millis(500),
        move |result: notify_debouncer_mini::DebounceEventResult| {
            if result.is_ok() {
                let _ = tx.blocking_send(());
            }
        },
    )?;
    debouncer
        .watcher()
        .watch(&ctx.identity.root, RecursiveMode::Recursive)?;

    while rx.recv().await.is_some() {
        match sync_repo(&ctx, false).await {
            Ok(outcome) if outcome.indexed_files + outcome.deleted_files > 0 => {
                println!(
                    "synced: {} files indexed, {} deleted",
                    outcome.indexed_files, outcome.deleted_files
                );
            }
            Ok(_) => {}
            Err(error) => eprintln!("sync failed: {error}"),
        }
    }
    Ok(())
}
