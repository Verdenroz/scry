pub mod eval;
pub mod index;
pub mod search;
pub mod serve;
pub mod status;
pub mod watch;

use std::path::PathBuf;

use anyhow::Result;
use scry_core::config::Config;
use scry_core::repo::{KeySource, RepoIdentity, detect};

use crate::client::ApiClient;

pub struct RepoContext {
    pub config: Config,
    pub identity: RepoIdentity,
    pub client: ApiClient,
    pub cwd: PathBuf,
}

pub fn repo_context() -> Result<RepoContext> {
    let config = Config::load(None)?;
    let cwd = std::env::current_dir()?;
    let identity = detect(&cwd)?;
    if identity.source == KeySource::DirName {
        eprintln!(
            "note: no git remote or .scry.toml found; indexing under key '{}'",
            identity.key
        );
    }
    let client = ApiClient::new(&config.client);
    Ok(RepoContext {
        config,
        identity,
        client,
        cwd,
    })
}
