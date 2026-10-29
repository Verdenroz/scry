use anyhow::{Result, bail};
use scry_server::api::{GcRequest, ReaffirmRequest, RestoreRequest, StaleRequest};

use super::{memory_scope_key, repo_context};

pub async fn reaffirm(args: &[String]) -> Result<()> {
    let mut id: Option<i64> = None;
    let mut evidence = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--evidence" => evidence = it.next().cloned(),
            text if id.is_none() => id = text.parse().ok(),
            _ => {}
        }
    }
    let (Some(id), Some(evidence)) = (id, evidence) else {
        bail!("usage: scry memory reaffirm <id> --evidence \"text\"");
    };
    let ctx = repo_context()?;
    ctx.client
        .reaffirm(&ReaffirmRequest { id, evidence })
        .await?;
    println!("reaffirmed #{id}");
    Ok(())
}

pub async fn stale(args: &[String]) -> Result<()> {
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--file" {
            files.extend(it.next().cloned());
        }
    }
    let ctx = repo_context()?;
    let response = ctx
        .client
        .stale(&StaleRequest {
            repo_key: Some(memory_scope_key(&ctx.identity)),
            files,
        })
        .await?;
    for memory in response.memories {
        println!(
            "[{} #{}] (stale) {} {}",
            memory.kind,
            memory.id,
            memory.relpaths.join(","),
            memory.content
        );
    }
    Ok(())
}

pub async fn gc(args: &[String]) -> Result<()> {
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let ctx = repo_context()?;
    let report = ctx.client.gc(&GcRequest { dry_run }).await?;
    println!("superseded: {}", report.superseded);
    println!("stale: {}", report.stale);
    println!("episodes: {}", report.episodes);
    println!("noise: {}", report.noise);
    let total = report.superseded + report.stale + report.episodes + report.noise;
    println!(
        "{} {total}",
        if dry_run { "would archive" } else { "archived" }
    );
    Ok(())
}

pub async fn restore(args: &[String]) -> Result<()> {
    let Some(id) = args.first().and_then(|id| id.parse().ok()) else {
        bail!("usage: scry memory restore <id>");
    };
    let ctx = repo_context()?;
    ctx.client.restore(&RestoreRequest { id }).await?;
    println!("restored #{id}");
    Ok(())
}
