//! Arg dispatch. Known subcommands parse strictly; anything else is a
//! search, parsed tolerantly the way mgrep does it: unknown flags are
//! skipped and excess positionals ignored, so agent-invented flags like
//! `--type python` never break a query.

use anyhow::{Result, bail};

use crate::commands;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "scry - self-hosted semantic code search

USAGE:
  scry \"natural language query\" [path]   search the current repo
  scry serve                             run the index server
  scry index                             sync this repo to the server
  scry watch                             sync continuously while you work
  scry status                            show server index counts
  scry eval <cases.toml> [--runs N] [--limit N]  score retrieval against a golden set
  scry remember \"insight\" [--kind K] [--pain N] [--anchor path[:a-b]]
  scry recall \"query\" [-m N]             recall memories about this codebase
  scry memory <helpful|noise> <id>       reinforce or demote a memory

SEARCH OPTIONS:
  -m, --max-count <n>   max results (default 10)
  -c, --content         print matching chunk content
  -a, --answer          answer the query with cited sources
  -w, --web             include web results
  --repo <key>          search a specific indexed repo from anywhere;
                        outside a repo, all indexed repos are searched
";

#[derive(Debug, PartialEq, Eq)]
pub struct SearchArgs {
    pub query: String,
    pub path: Option<String>,
    pub max_count: usize,
    pub content: bool,
    pub answer: bool,
    pub web: bool,
    pub repo: Option<String>,
}

const VALUE_FLAGS: &[&str] = &["-m", "--max-count", "--max-file-size", "--max-file-count"];

pub fn parse_search_args(args: &[String]) -> Result<SearchArgs> {
    let mut positionals: Vec<&str> = Vec::new();
    let mut max_count = 10;
    let (mut content, mut answer, mut web) = (false, false, false);
    let mut repo = None;
    let mut it = args.iter().peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-c" | "--content" => content = true,
            "-a" | "--answer" => answer = true,
            "-w" | "--web" => web = true,
            "--repo" => repo = it.next().cloned(),
            "-m" | "--max-count" => {
                if let Some(value) = it.next() {
                    max_count = value.parse().unwrap_or(max_count);
                }
            }
            flag if VALUE_FLAGS.contains(&flag) => {
                it.next();
            }
            "-i" | "-r" | "-s" | "-d" | "--sync" | "--dry-run" | "--no-rerank" => {}
            flag if flag.starts_with('-') => {}
            positional => positionals.push(positional),
        }
    }
    let Some(query) = positionals.first() else {
        bail!("no query given\n\n{USAGE}");
    };
    Ok(SearchArgs {
        query: query.to_string(),
        path: positionals.get(1).map(|p| p.to_string()),
        max_count,
        content,
        answer,
        web,
        repo,
    })
}

pub async fn dispatch(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("scry {VERSION}");
            Ok(())
        }
        Some("serve") => commands::serve::run().await,
        Some("index") => commands::index::run().await,
        Some("watch") => commands::watch::run().await,
        Some("status") => commands::status::run().await,
        Some("eval") => commands::eval::run(&args[1..]).await,
        Some("remember") => commands::memory::remember(&args[1..]).await,
        Some("recall") => commands::memory::recall(&args[1..]).await,
        Some("memory") => commands::memory::feedback(&args[1..]).await,
        Some("repo") => commands::repo::run(&args[1..]).await,
        Some("login" | "logout") => {
            println!("scry is self-hosted; auth is a bearer token in the config (SCRY_TOKEN)");
            Ok(())
        }
        _ => commands::search::run(parse_search_args(args)?).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plain_query_and_path() {
        let parsed =
            parse_search_args(&strings(&["How are chunks defined?", "src/models"])).unwrap();
        assert_eq!(parsed.query, "How are chunks defined?");
        assert_eq!(parsed.path.as_deref(), Some("src/models"));
        assert_eq!(parsed.max_count, 10);
    }

    #[test]
    fn tolerates_unknown_flags_and_excess_positionals() {
        let parsed = parse_search_args(&strings(&[
            "How are chunks defined?",
            "src/models",
            "--type",
            "python",
            "--context",
            "3",
        ]))
        .unwrap();
        assert_eq!(parsed.query, "How are chunks defined?");
        assert_eq!(parsed.path.as_deref(), Some("src/models"));
    }

    #[test]
    fn parses_known_flags() {
        let parsed =
            parse_search_args(&strings(&["-m", "5", "-c", "--web", "--answer", "query"])).unwrap();
        assert_eq!(parsed.max_count, 5);
        assert!(parsed.content && parsed.web && parsed.answer);
        assert_eq!(parsed.query, "query");
    }

    #[test]
    fn rejects_empty_invocation_flags_only() {
        assert!(parse_search_args(&strings(&["--no-rerank"])).is_err());
    }
}
