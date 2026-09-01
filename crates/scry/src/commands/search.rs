use anyhow::{Result, bail};
use scry_server::api::SearchRequest;

use super::repo_context;
use crate::cli::SearchArgs;
use crate::output::print_hits;

pub async fn run(args: SearchArgs) -> Result<()> {
    let ctx = repo_context()?;
    if args.answer {
        eprintln!("note: --answer is not wired up yet; showing matches");
    }
    if args.web {
        eprintln!("note: --web is not wired up yet; showing local matches");
    }

    let request = SearchRequest {
        repo_key: ctx.identity.key.clone(),
        query: args.query,
        limit: args.max_count,
        path_prefix: path_prefix(&ctx, args.path.as_deref())?,
    };
    let response = ctx.client.search(&request).await?;
    print_hits(&response.hits, &ctx.identity.root, &ctx.cwd, args.content);
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
