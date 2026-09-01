//! Repo identity. Every checkout of the same remote maps to one index key,
//! so devices share index rows regardless of where the repo is cloned.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    Remote,
    ProjectConfig,
    DirName,
}

#[derive(Debug, Clone)]
pub struct RepoIdentity {
    pub key: String,
    pub root: PathBuf,
    pub source: KeySource,
}

/// Canonical key for a git remote URL: `host/path`, lowercased host,
/// credentials, scheme, port, and trailing `.git` stripped.
pub fn normalize_remote_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest.to_string(),
        // scp-like syntax: [user@]host:path
        None => {
            let (host, path) = url.split_once(':')?;
            format!("{host}/{}", path.trim_start_matches('/'))
        }
    };
    let (authority, path) = rest.split_once('/')?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{}/{path}", host.to_lowercase()))
}

#[derive(Deserialize)]
struct ProjectFile {
    project: ProjectSection,
}

#[derive(Deserialize)]
struct ProjectSection {
    name: String,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8(out.stdout).ok()?;
    let line = stdout.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

fn project_config_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".scry.toml")).ok()?;
    let file: ProjectFile = toml::from_str(&text).ok()?;
    Some(file.project.name)
}

/// Resolves the repo containing `dir`: git toplevel as root (else `dir`),
/// key from the origin remote, then `.scry.toml` `[project] name`, then
/// the directory basename.
pub fn detect(dir: &Path) -> Result<RepoIdentity> {
    let dir = dir.canonicalize()?;
    let root = git(&dir, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.clone());

    if let Some(key) =
        git(&root, &["remote", "get-url", "origin"]).and_then(|url| normalize_remote_url(&url))
    {
        return Ok(RepoIdentity {
            key,
            root,
            source: KeySource::Remote,
        });
    }
    if let Some(key) = project_config_name(&root) {
        return Ok(RepoIdentity {
            key,
            root,
            source: KeySource::ProjectConfig,
        });
    }
    let key = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| Error::Config(format!("no repo name derivable for {}", root.display())))?;
    Ok(RepoIdentity {
        key,
        root,
        source: KeySource::DirName,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_scp_and_https_to_same_key() {
        let a = normalize_remote_url("git@github.com:Verdenroz/scry.git").unwrap();
        let b = normalize_remote_url("https://github.com/Verdenroz/scry").unwrap();
        assert_eq!(a, "github.com/Verdenroz/scry");
        assert_eq!(a, b);
    }

    #[test]
    fn strips_credentials_port_and_trailing_slash() {
        assert_eq!(
            normalize_remote_url("https://user:pass@GitHub.com:8443/a/b.git/"),
            Some("github.com/a/b".to_string())
        );
        assert_eq!(
            normalize_remote_url("ssh://git@github.com:22/a/b.git"),
            Some("github.com/a/b".to_string())
        );
    }

    #[test]
    fn rejects_urls_without_a_path() {
        assert_eq!(normalize_remote_url("github.com"), None);
        assert_eq!(normalize_remote_url("https://github.com/"), None);
    }

    #[test]
    fn detect_falls_back_to_dir_name() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("myproj");
        std::fs::create_dir(&repo).unwrap();
        let id = detect(&repo).unwrap();
        assert_eq!(id.key, "myproj");
        assert_eq!(id.source, KeySource::DirName);
    }

    #[test]
    fn detect_prefers_scry_toml_over_dir_name() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("myproj");
        std::fs::create_dir(&repo).unwrap();
        std::fs::write(repo.join(".scry.toml"), "[project]\nname = \"custom\"\n").unwrap();
        let id = detect(&repo).unwrap();
        assert_eq!(id.key, "custom");
        assert_eq!(id.source, KeySource::ProjectConfig);
    }
}
