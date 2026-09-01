pub mod eval;
pub mod index;
pub mod memory;
pub mod repo;
pub mod search;
pub mod serve;
pub mod status;
pub mod watch;

use anyhow::Result;
use scry_core::config::Config;
use scry_core::repo::{KeySource, RepoIdentity, detect};

use crate::client::ApiClient;

pub struct RepoContext {
    pub config: Config,
    pub identity: RepoIdentity,
    pub client: ApiClient,
}

/// True when `root` is the home directory or an ancestor of it; scry never
/// treats such a directory as a project repo.
pub fn at_or_above_home(root: &std::path::Path) -> bool {
    std::env::var_os("HOME").is_some_and(|home| std::path::Path::new(&home).starts_with(root))
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
    })
}
