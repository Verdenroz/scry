use std::path::PathBuf;

use anyhow::{Result, bail};
use scry_core::config::Config;
use scry_core::repo::{KeySource, RepoIdentity, detect};
use scry_server::api::{AnswerRequest, SearchRequest, WebSearchRequest};

use crate::cli::SearchArgs;
use crate::client::ApiClient;
use crate::output::{print_global_hits, print_hits};

/// Search scope: inside a repo it is that repo (cwd-scoped like grep);
/// `--repo <key>` targets any indexed repo; at or above home, with no
/// repo named, every indexed repo is searched.
struct Scope {
    repo_key: Option<String>,
    path_prefix: Option<String>,
    local_root: Option<PathBuf>,
}

fn resolve_scope(args: &SearchArgs, cwd: &std::path::Path) -> Result<Scope> {
    if let Some(repo) = &args.repo {
        return Ok(Scope {
            repo_key: Some(repo.clone()),
            path_prefix: None,
            local_root: None,
        });
    }
    if super::at_or_above_home(cwd) {
        return Ok(Scope {
            repo_key: None,
            path_prefix: None,
            local_root: None,
        });
    }
    let identity = detect(cwd)?;
    if identity.source == KeySource::DirName {
        eprintln!(
            "note: no git remote or .scry.toml found; searching key '{}'",
            identity.key
        );
    }
    let path_prefix = path_prefix(&identity, cwd, args.path.as_deref())?;
    Ok(Scope {
        repo_key: Some(identity.key.clone()),
        path_prefix,
        local_root: Some(identity.root),
    })
}

pub async fn run(args: SearchArgs) -> Result<()> {
    let config = Config::load(None)?;
    let client = ApiClient::new(&config.client);
    let cwd = std::env::current_dir()?;
    let scope = resolve_scope(&args, &cwd)?;

    if args.answer {
        let response = client
            .answer(&AnswerRequest {
                query: args.query,
                repo_key: scope.repo_key,
                web: args.web,
            })
            .await?;
        println!("{}\n", response.answer);
        for citation in &response.citations {
            println!("{}: {}", citation.n, citation.source);
        }
        return Ok(());
    }

    let response = client
        .search(&SearchRequest {
            repo_key: scope.repo_key,
            query: args.query.clone(),
            limit: args.max_count,
            path_prefix: scope.path_prefix,
            rerank: args.rerank,
        })
        .await?;
    match &scope.local_root {
        Some(root) => print_hits(&response.hits, root, &cwd, args.content),
        None => print_global_hits(&response.hits, args.content),
    }

    if args.web {
        let web = client
            .web_search(&WebSearchRequest {
                query: args.query,
                limit: args.max_count.min(5),
            })
            .await?;
        for hit in &web.results {
            println!("{} ({:.2}% match)", hit.url, hit.score * 100.0);
        }
    }
    Ok(())
}

fn path_prefix(
    identity: &RepoIdentity,
    cwd: &std::path::Path,
    path: Option<&str>,
) -> Result<Option<String>> {
    let target = match path {
        Some(path) => {
            let joined = if std::path::Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                cwd.join(path)
            };
            std::path::absolute(joined)?
        }
        None => cwd.to_path_buf(),
    };
    let Ok(relative) = target.strip_prefix(&identity.root) else {
        bail!("search path {} is outside the repo", target.display());
    };
    let prefix = relative.to_string_lossy().replace('\\', "/");
    Ok((!prefix.is_empty()).then_some(prefix))
}
