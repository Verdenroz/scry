use anyhow::Result;
use scry_core::config::Config;
use tracing_subscriber::EnvFilter;

pub async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let config = Config::load(None)?;
    scry_server::serve(config).await?;
    Ok(())
}
