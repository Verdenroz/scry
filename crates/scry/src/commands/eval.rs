use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use scry_server::api::{Hit, SearchRequest};
use serde::Deserialize;

use super::repo_context;
use crate::client::ApiClient;

const USAGE: &str = "usage: scry eval <cases.toml> [--runs N] [--limit N]";

#[derive(Deserialize)]
struct EvalFile {
    #[serde(default)]
    meta: Meta,
    case: Vec<EvalCase>,
}

#[derive(Deserialize, Default)]
struct Meta {
    repo: Option<String>,
}

#[derive(Deserialize)]
struct EvalCase {
    query: String,
    expect: Vec<String>,
    path_prefix: Option<String>,
}

struct CaseResult {
    rank: Option<usize>,
    elapsed: Duration,
}

struct RunSummary {
    recall: f64,
    mrr: f64,
    p50: Duration,
    p95: Duration,
}

fn matches(hit: &Hit, expectation: &str) -> bool {
    match expectation.rsplit_once(':') {
        Some((path, line)) if line.chars().all(|c| c.is_ascii_digit()) => {
            let line: u32 = line.parse().unwrap_or(0);
            hit.relpath == path && hit.start_line <= line && line <= hit.end_line
        }
        _ => hit.relpath == expectation,
    }
}

struct Args {
    file: String,
    runs: usize,
    limit: usize,
}

fn parse_args(args: &[String]) -> Result<Args> {
    let mut file = None;
    let (mut runs, mut limit) = (1, 10);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--runs" => runs = it.next().and_then(|v| v.parse().ok()).unwrap_or(runs),
            "--limit" => limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(limit),
            _ => file = Some(arg.clone()),
        }
    }
    let Some(file) = file else {
        bail!("{USAGE}");
    };
    Ok(Args {
        file,
        runs: runs.max(1),
        limit: limit.max(1),
    })
}

async fn search_case(
    client: &ApiClient,
    repo_key: &str,
    case: &EvalCase,
    limit: usize,
) -> Result<CaseResult> {
    let started = Instant::now();
    let response = client
        .search(&SearchRequest {
            repo_key: Some(repo_key.to_string()),
            query: case.query.clone(),
            limit,
            path_prefix: case.path_prefix.clone(),
        })
        .await?;
    let rank = response.hits.iter().position(|hit| {
        case.expect
            .iter()
            .any(|expectation| matches(hit, expectation))
    });
    Ok(CaseResult {
        rank,
        elapsed: started.elapsed(),
    })
}

fn percentile(sorted: &[Duration], pct: usize) -> Duration {
    sorted
        .get((sorted.len().saturating_sub(1)) * pct / 100)
        .copied()
        .unwrap_or_default()
}

fn summarize(results: &[CaseResult]) -> RunSummary {
    let n = results.len().max(1) as f64;
    let hits = results.iter().filter(|r| r.rank.is_some()).count() as f64;
    let reciprocal: f64 = results
        .iter()
        .filter_map(|r| r.rank)
        .map(|rank| 1.0 / (rank as f64 + 1.0))
        .sum();
    let mut latencies: Vec<Duration> = results.iter().map(|r| r.elapsed).collect();
    latencies.sort();
    RunSummary {
        recall: hits / n,
        mrr: reciprocal / n,
        p50: percentile(&latencies, 50),
        p95: percentile(&latencies, 95),
    }
}

fn rank_label(rank: Option<usize>) -> String {
    rank.map_or_else(|| "miss".to_string(), |rank| format!("@{:<3}", rank + 1))
}

fn print_report(cases: &[EvalCase], runs: &[Vec<CaseResult>], limit: usize) {
    for (i, case) in cases.iter().enumerate() {
        let ranks: Vec<String> = runs.iter().map(|run| rank_label(run[i].rank)).collect();
        println!("{}  {}", ranks.join(" "), case.query);
    }
    let summaries: Vec<RunSummary> = runs.iter().map(|run| summarize(run)).collect();
    println!();
    for (i, s) in summaries.iter().enumerate() {
        println!(
            "run {}: recall@{limit} {:.3}  mrr {:.3}  p50 {}ms  p95 {}ms  ({} cases)",
            i + 1,
            s.recall,
            s.mrr,
            s.p50.as_millis(),
            s.p95.as_millis(),
            cases.len()
        );
    }
    if summaries.len() > 1 {
        let (lo, hi) = spread(summaries.iter().map(|s| s.recall));
        let (mlo, mhi) = spread(summaries.iter().map(|s| s.mrr));
        println!("spread: recall {lo:.3}-{hi:.3}  mrr {mlo:.3}-{mhi:.3}");
    }
}

fn spread(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    })
}

pub async fn run(args: &[String]) -> Result<()> {
    let args = parse_args(args)?;
    let text = std::fs::read_to_string(&args.file)
        .with_context(|| format!("cannot read {}", args.file))?;
    let cases: EvalFile = toml::from_str(&text)?;
    let ctx = repo_context()?;
    let repo_key = cases.meta.repo.unwrap_or(ctx.identity.key);

    let mut runs = Vec::with_capacity(args.runs);
    for _ in 0..args.runs {
        let mut results = Vec::with_capacity(cases.case.len());
        for case in &cases.case {
            results.push(search_case(&ctx.client, &repo_key, case, args.limit).await?);
        }
        runs.push(results);
    }
    print_report(&cases.case, &runs, args.limit);
    Ok(())
}
