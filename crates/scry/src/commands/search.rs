use anyhow::{Result, bail};
use scry_server::api::{AnswerRequest, SearchRequest, WebSearchRequest};

use super::repo_context;
use crate::cli::SearchArgs;
use crate::output::print_hits;

pub async fn run(args: SearchArgs) -> Result<()> {
    let ctx = repo_context()?;
    if super::at_or_above_home(&ctx.identity.root) {
        bail!(
            "{} is not inside a project repo; cd into one to search it",
            ctx.cwd.display()
        );
    }
    if args.answer {
        let response = ctx
            .client
            .answer(&AnswerRequest {
                query: args.query,
                repo_key: Some(ctx.identity.key.clone()),
                web: args.web,
            })
            .await?;
        println!("{}\n", response.answer);
        for citation in &response.citations {
            println!("{}: {}", citation.n, citation.source);
        }
        return Ok(());
    }

    let request = SearchRequest {
        repo_key: ctx.identity.key.clone(),
        query: args.query.clone(),
        limit: args.max_count,
        path_prefix: path_prefix(&ctx, args.path.as_deref())?,
    };
    let response = ctx.client.search(&request).await?;
    print_hits(&response.hits, &ctx.identity.root, &ctx.cwd, args.content);

    if args.web {
        let web = ctx
            .client
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

/// Search scope defaults to the invoking directory, like grep: an explicit
/// `[path]` narrows further, and everything is repo-root-relative on the wire.
fn path_prefix(ctx: &super::RepoContext, path: Option<&str>) -> Result<Option<String>> {
    let target = match path {
        Some(path) => {
            let joined = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                ctx.cwd.join(path)
            };
            std::path::absolute(joined)?
        }
        None => ctx.cwd.clone(),
    };
    let Ok(relative) = target.strip_prefix(&ctx.identity.root) else {
        bail!("search path {} is outside the repo", target.display());
    };
    let prefix = relative.to_string_lossy().replace('\\', "/");
    Ok((!prefix.is_empty()).then_some(prefix))
}
