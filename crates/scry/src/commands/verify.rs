use std::time::Instant;

use anyhow::{Context, Result, bail};
use scry_core::config::Config;
use scry_server::api::{VerifyRequest, VerifyResponse};
use serde::Deserialize;

use super::take_rest;
use crate::client::ApiClient;

const USAGE: &str =
    "usage: scry memory verify [--evidence \"text\"] [--] \"claim\" | --eval <pairs.jsonl>";

#[derive(Deserialize)]
struct Pair {
    claim: String,
    evidence: String,
    label: u8,
}

struct Scored {
    supported_label: bool,
    score: f64,
    accepted: bool,
    millis: f64,
}

fn parse_args(args: &[String]) -> Result<(Option<String>, Option<String>, Option<String>)> {
    let (mut claim, mut evidence, mut eval) = (None, None, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--evidence" => evidence = it.next().cloned(),
            "--eval" => eval = it.next().cloned(),
            "--" => claim = take_rest(claim, it.by_ref(), USAGE)?,
            text if !text.starts_with('-') && claim.is_none() => claim = Some(text.to_string()),
            _ => {}
        }
    }
    Ok((claim, evidence, eval))
}

pub async fn run(args: &[String]) -> Result<()> {
    let (claim, evidence, eval) = parse_args(args)?;
    let client = ApiClient::new(&Config::load(None)?.client);
    if let Some(path) = eval {
        return calibrate(&client, &path).await;
    }
    let (Some(claim), Some(evidence)) = (claim, evidence) else {
        bail!(USAGE);
    };
    let response = client.verify(&VerifyRequest { claim, evidence }).await?;
    match (response.score, response.supported) {
        (Some(score), Some(supported)) => {
            let verdict = if supported {
                "supported"
            } else {
                "unsupported"
            };
            println!("{verdict} {score:.2}");
        }
        _ => println!("not configured"),
    }
    Ok(())
}

async fn calibrate(client: &ApiClient, path: &str) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {path}"))?;
    let mut scored = Vec::new();
    let mut errors = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let pair: Pair = serde_json::from_str(line).context("bad pair row")?;
        let started = Instant::now();
        let outcome = client
            .verify(&VerifyRequest {
                claim: pair.claim,
                evidence: pair.evidence,
            })
            .await;
        let millis = started.elapsed().as_secs_f64() * 1000.0;
        scored.push(score_outcome(pair.label, outcome, millis, &mut errors)?);
    }
    print_calibration(&scored, &errors);
    Ok(())
}

/// A verifier error counts as a rejection so a model that fails some rows
/// can still be measured; an unconfigured server aborts the run.
fn score_outcome(
    label: u8,
    outcome: Result<VerifyResponse>,
    millis: f64,
    errors: &mut Vec<String>,
) -> Result<Scored> {
    match outcome {
        Ok(response) => score_row(label, response, millis),
        Err(error) => {
            errors.push(error.to_string());
            Ok(Scored {
                supported_label: label == 1,
                score: 0.0,
                accepted: false,
                millis,
            })
        }
    }
}

fn score_row(label: u8, response: VerifyResponse, millis: f64) -> Result<Scored> {
    let (Some(score), Some(accepted)) = (response.score, response.supported) else {
        bail!("verifier is not configured on the server");
    };
    Ok(Scored {
        supported_label: label == 1,
        score,
        accepted,
        millis,
    })
}

fn scores(rows: &[Scored], supported_label: bool) -> Vec<f64> {
    rows.iter()
        .filter(|r| r.supported_label == supported_label)
        .map(|r| r.score)
        .collect()
}

fn auroc(rows: &[Scored]) -> Option<f64> {
    let (positives, negatives) = (scores(rows, true), scores(rows, false));
    if positives.is_empty() || negatives.is_empty() {
        return None;
    }
    let wins: f64 = positives
        .iter()
        .flat_map(|p| negatives.iter().map(move |n| (p, n)))
        .map(|(p, n)| match p.total_cmp(n) {
            std::cmp::Ordering::Greater => 1.0,
            std::cmp::Ordering::Equal => 0.5,
            std::cmp::Ordering::Less => 0.0,
        })
        .sum();
    Some(wins / (positives.len() * negatives.len()) as f64)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    match values.len() {
        0 => 0.0,
        n if n % 2 == 1 => values[n / 2],
        n => (values[n / 2 - 1] + values[n / 2]) / 2.0,
    }
}

fn print_calibration(rows: &[Scored], errors: &[String]) {
    println!(
        "pairs: {} ({} supported)",
        rows.len(),
        scores(rows, true).len()
    );
    match auroc(rows) {
        Some(auroc) => println!("AUROC: {auroc:.3}"),
        None => println!("AUROC: n/a (needs both labels)"),
    }
    let mut latencies: Vec<f64> = rows.iter().map(|r| r.millis).collect();
    println!("median latency: {:.0} ms", median(&mut latencies));
    let false_accepts = rows
        .iter()
        .filter(|r| !r.supported_label && r.accepted)
        .count();
    let false_rejects = rows
        .iter()
        .filter(|r| r.supported_label && !r.accepted)
        .count();
    println!(
        "at configured threshold: {false_accepts} false accepts, {false_rejects} false rejects"
    );
    if let Some(worst) = scores(rows, false).into_iter().max_by(f64::total_cmp) {
        let supported = scores(rows, true);
        let rejected = supported.iter().filter(|&&score| score <= worst).count();
        println!(
            "zero false accepts above threshold: {worst:.2} (rejects {rejected} of {} supported)",
            supported.len()
        );
    }
    if let Some(first) = errors.first() {
        println!(
            "errors: {} (counted as rejections), first: {first}",
            errors.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(supported_label: bool, score: f64) -> Scored {
        Scored {
            supported_label,
            score,
            accepted: score >= 0.5,
            millis: 0.0,
        }
    }

    #[test]
    fn auroc_counts_ties_as_half() {
        let perfect = [
            row(true, 0.9),
            row(true, 0.7),
            row(false, 0.2),
            row(false, 0.1),
        ];
        assert_eq!(auroc(&perfect), Some(1.0));
        assert_eq!(auroc(&[row(true, 0.5), row(false, 0.5)]), Some(0.5));
        let mixed = [row(true, 0.9), row(true, 0.3), row(false, 0.5)];
        assert_eq!(auroc(&mixed), Some(0.5));
        assert_eq!(auroc(&[row(true, 0.9)]), None);
    }

    #[test]
    fn errors_count_as_rejections_and_unconfigured_aborts() {
        let mut errors = Vec::new();
        let failed = score_outcome(1, Err(anyhow::anyhow!("no yes/no")), 5.0, &mut errors).unwrap();
        assert_eq!(
            (failed.score, failed.accepted, failed.supported_label),
            (0.0, false, true)
        );
        assert_eq!(errors, ["no yes/no"]);
        let unconfigured = VerifyResponse {
            configured: false,
            score: None,
            supported: None,
        };
        assert!(score_outcome(1, Ok(unconfigured), 5.0, &mut errors).is_err());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn separator_takes_the_rest_as_the_claim() {
        let args: Vec<String> = ["--evidence", "ev", "--", "uses", "--flag", "-x"]
            .map(String::from)
            .to_vec();
        let (claim, evidence, eval) = parse_args(&args).unwrap();
        assert_eq!(claim.as_deref(), Some("uses --flag -x"));
        assert_eq!(evidence.as_deref(), Some("ev"));
        assert_eq!(eval, None);
    }

    #[test]
    fn median_handles_even_and_odd() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), 2.5);
    }
}
