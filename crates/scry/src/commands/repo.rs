use anyhow::{Result, bail};
use scry_core::config::Config;
use scry_server::api::PruneRequest;

use crate::client::ApiClient;

pub async fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("prune") => prune(&args[1..]).await,
        _ => bail!("usage: scry repo prune <key> [--into <key>]"),
    }
}

async fn prune(args: &[String]) -> Result<()> {
    let mut key: Option<String> = None;
    let mut into: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--into" => into = it.next().cloned(),
            text if !text.starts_with('-') && key.is_none() => key = Some(text.to_string()),
            _ => {}
        }
    }
    let Some(key) = key else {
        bail!("usage: scry repo prune <key> [--into <key>]");
    };

    let config = Config::load(None)?;
    let client = ApiClient::new(&config.client);
    let response = client
        .prune(&PruneRequest {
            repo_key: key.clone(),
            migrate_memories_to: into,
        })
        .await?;
    println!("pruned {} ({} files removed)", key, response.deleted_files);
    Ok(())
}
