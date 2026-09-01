use anyhow::{Context, Result, bail};
use scry_server::api::{Hit, SearchRequest};
use serde::Deserialize;

use super::repo_context;

#[derive(Deserialize)]
struct EvalFile {
    case: Vec<EvalCase>,
}

#[derive(Deserialize)]
struct EvalCase {
    query: String,
    expect: Vec<String>,
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

pub async fn run(file: Option<&str>) -> Result<()> {
    let Some(file) = file else {
        bail!("usage: scry eval <cases.toml>");
    };
    let text = std::fs::read_to_string(file).with_context(|| format!("cannot read {file}"))?;
    let cases: EvalFile = toml::from_str(&text)?;
    let ctx = repo_context()?;

    let mut hits_at_10 = 0usize;
    let mut reciprocal_rank_sum = 0f64;
    for case in &cases.case {
        let response = ctx
            .client
            .search(&SearchRequest {
                repo_key: ctx.identity.key.clone(),
                query: case.query.clone(),
                limit: 10,
                path_prefix: None,
            })
            .await?;
        let rank = response.hits.iter().position(|hit| {
            case.expect
                .iter()
                .any(|expectation| matches(hit, expectation))
        });
        match rank {
            Some(rank) => {
                hits_at_10 += 1;
                reciprocal_rank_sum += 1.0 / (rank as f64 + 1.0);
                println!("hit  @{:<2} {}", rank + 1, case.query);
            }
            None => println!("miss     {}", case.query),
        }
    }

    let n = cases.case.len().max(1) as f64;
    println!(
        "\nrecall@10 {:.3}  mrr {:.3}  ({} cases)",
        hits_at_10 as f64 / n,
        reciprocal_rank_sum / n,
        cases.case.len()
    );
    Ok(())
}
