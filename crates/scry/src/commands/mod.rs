pub mod eval;
pub mod index;
pub mod lifecycle;
pub mod memory;
pub mod repo;
pub mod search;
pub mod serve;
pub mod status;
pub mod verify;
pub mod watch;

use anyhow::{Result, bail};
use scry_core::config::Config;
use scry_core::repo::{KeySource, RepoIdentity, detect, path_scope_key};

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

/// Memory scope for this directory: the repo key, or a home-relative path
/// key when the directory is at or above `$HOME`.
pub fn memory_scope_key(identity: &RepoIdentity) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home {
        Some(home) if home.starts_with(&identity.root) => path_scope_key(&identity.root, &home),
        _ => identity.key.clone(),
    }
}

/// The positional argument after a `--` separator: everything that follows,
/// joined. Bails with `usage` when a positional was already given and more
/// follows, since the extra words would be dropped silently.
pub fn take_rest<'a>(
    current: Option<String>,
    rest: impl Iterator<Item = &'a String>,
    usage: &str,
) -> Result<Option<String>> {
    let joined = rest.map(String::as_str).collect::<Vec<_>>().join(" ");
    match (current, joined.is_empty()) {
        (Some(_), false) => bail!("{usage}"),
        (current, true) => Ok(current),
        (None, false) => Ok(Some(joined)),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rest(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn take_rest_joins_when_no_positional_came_first() {
        let words = rest(&["uses", "--flag"]);
        let got = take_rest(None, words.iter(), "usage").unwrap();
        assert_eq!(got.as_deref(), Some("uses --flag"));
        assert_eq!(take_rest(None, [].iter(), "usage").unwrap(), None);
    }

    #[test]
    fn take_rest_bails_instead_of_dropping_words_after_a_positional() {
        let words = rest(&["extra"]);
        let err = take_rest(Some("first".into()), words.iter(), "usage: x").unwrap_err();
        assert_eq!(err.to_string(), "usage: x");
        let kept = take_rest(Some("first".into()), [].iter(), "usage").unwrap();
        assert_eq!(kept.as_deref(), Some("first"));
    }
}
