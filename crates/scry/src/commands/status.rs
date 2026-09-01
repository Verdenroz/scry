use anyhow::Result;
use scry_core::config::Config;

use crate::client::ApiClient;

pub async fn run() -> Result<()> {
    let config = Config::load(None)?;
    let client = ApiClient::new(&config.client);
    let status = client.status().await?;
    println!(
        "server {}: {} repos, {} files, {} chunks",
        config.client.server_url, status.repos, status.files, status.chunks
    );
    Ok(())
}
