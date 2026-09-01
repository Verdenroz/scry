use anyhow::Result;
use scry_core::config::Config;

use crate::client::ApiClient;

pub async fn run() -> Result<()> {
    let config = Config::load(None)?;
    let client = ApiClient::new(&config.client);
    let status = client.status().await?;
    println!(
        "server {}: {} repos, {} files, {} chunks, {} memories ({} stale)",
        config.client.server_url,
        status.repos,
        status.files,
        status.chunks,
        status.memories,
        status.stale_memories
    );
    Ok(())
}
