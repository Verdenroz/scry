use anyhow::{Result, bail};
use scry_server::api::{AnchorDto, FeedbackRequest, RecallRequest, RememberRequest};

use super::repo_context;

const REMEMBER_USAGE: &str = "usage: scry remember \"content\" [--kind lesson|decision|convention|skill|fact|episode] [--pain 0-10] [--cost 0-10] [--anchor path[:start-end]] [--global]";

pub async fn remember(args: &[String]) -> Result<()> {
    let mut content: Option<String> = None;
    let mut kind = "fact".to_string();
    let (mut pain, mut cost) = (None, None);
    let mut anchors: Vec<AnchorDto> = Vec::new();
    let mut global = false;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--kind" => kind = it.next().cloned().unwrap_or(kind),
            "--pain" => pain = it.next().and_then(|v| v.parse().ok()),
            "--cost" => cost = it.next().and_then(|v| v.parse().ok()),
            "--global" => global = true,
            "--anchor" => {
                let Some(spec) = it.next() else {
                    bail!(REMEMBER_USAGE);
                };
                anchors.push(parse_anchor(spec));
            }
            text if !text.starts_with('-') && content.is_none() => {
                content = Some(text.to_string());
            }
            _ => {}
        }
    }
    let Some(content) = content else {
        bail!(REMEMBER_USAGE);
    };

    let ctx = repo_context()?;
    let scoped = !global && !super::at_or_above_home(&ctx.identity.root);
    let response = ctx
        .client
        .remember(&RememberRequest {
            repo_key: scoped.then(|| ctx.identity.key.clone()),
            kind,
            content,
            pain,
            cost,
            anchors,
        })
        .await?;
    println!(
        "remembered #{} (salience {:.2}, surprise {:.2})",
        response.id, response.salience, response.surprise
    );
    Ok(())
}

fn parse_anchor(spec: &str) -> AnchorDto {
    match spec.rsplit_once(':') {
        Some((path, range)) if range.contains('-') || range.chars().all(|c| c.is_ascii_digit()) => {
            let (start, end) = match range.split_once('-') {
                Some((s, e)) => (s.parse().ok(), e.parse().ok()),
                None => (range.parse().ok(), range.parse().ok()),
            };
            AnchorDto {
                relpath: path.to_string(),
                start_line: start,
                end_line: end,
            }
        }
        _ => AnchorDto {
            relpath: spec.to_string(),
            start_line: None,
            end_line: None,
        },
    }
}

pub async fn recall(args: &[String]) -> Result<()> {
    let mut query: Option<String> = None;
    let mut limit = 5;
    let mut min_score = 0.0f64;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-m" | "--max-count" => limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(limit),
            "--min-score" => {
                min_score = it.next().and_then(|v| v.parse().ok()).unwrap_or(min_score);
            }
            text if !text.starts_with('-') && query.is_none() => query = Some(text.to_string()),
            _ => {}
        }
    }
    let Some(query) = query else {
        bail!("usage: scry recall \"query\" [-m N] [--min-score S]");
    };

    let ctx = repo_context()?;
    let scoped = !super::at_or_above_home(&ctx.identity.root);
    let response = ctx
        .client
        .recall(&RecallRequest {
            repo_key: scoped.then(|| ctx.identity.key.clone()),
            query,
            limit,
        })
        .await?;
    for memory in response.memories.iter().filter(|m| m.score >= min_score) {
        let stale = if memory.stale { " (stale)" } else { "" };
        println!(
            "[{} #{}] ({:.2}){} {}",
            memory.kind, memory.id, memory.score, stale, memory.content
        );
    }
    Ok(())
}

pub async fn feedback(args: &[String]) -> Result<()> {
    let (helpful, id) = match (args.first().map(String::as_str), args.get(1)) {
        (Some("helpful"), Some(id)) => (true, id),
        (Some("noise"), Some(id)) => (false, id),
        _ => bail!("usage: scry memory <helpful|noise> <id>"),
    };
    let id: i64 = id.parse()?;
    let ctx = repo_context()?;
    let response = ctx
        .client
        .feedback(&FeedbackRequest { id, helpful })
        .await?;
    println!(
        "{}",
        if response.updated {
            "recorded"
        } else {
            "no such memory"
        }
    );
    Ok(())
}
