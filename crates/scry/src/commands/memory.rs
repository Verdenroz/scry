use anyhow::{Result, bail};
use scry_server::api::{
    AnchorDto, BriefRequest, FeedbackRequest, RecallRequest, RememberRequest, SupersedeRequest,
};

use super::{memory_scope_key, repo_context, take_rest};

const REMEMBER_USAGE: &str = "usage: scry remember \"content\" [--kind lesson|decision|convention|skill|fact|episode] [--pain 0-10] [--cost 0-10] [--anchor path[:start-end]] [--source agent|capture] [--evidence \"text\"] [--global]";

const RECALL_USAGE: &str = "usage: scry recall \"query\" [-m N] [--min-score S] [--no-touch]";

pub async fn remember(args: &[String]) -> Result<()> {
    let mut content: Option<String> = None;
    let mut kind = "fact".to_string();
    let (mut pain, mut cost) = (None, None);
    let mut anchors: Vec<AnchorDto> = Vec::new();
    let mut source = "agent".to_string();
    let mut evidence = None;
    let mut global = false;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--kind" => kind = it.next().cloned().unwrap_or(kind),
            "--pain" => pain = it.next().and_then(|v| v.parse().ok()),
            "--cost" => cost = it.next().and_then(|v| v.parse().ok()),
            "--source" => source = it.next().cloned().unwrap_or(source),
            "--evidence" => evidence = it.next().cloned(),
            "--global" => global = true,
            "--" => content = take_rest(content, it.by_ref(), REMEMBER_USAGE)?,
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
    let response = ctx
        .client
        .remember(&RememberRequest {
            repo_key: (!global).then(|| memory_scope_key(&ctx.identity)),
            kind,
            content,
            pain,
            cost,
            source,
            evidence,
            anchors,
        })
        .await?;
    println!(
        "remembered #{} (salience {:.2}, surprise {:.2})",
        response.id, response.salience, response.surprise
    );
    Ok(())
}

pub(super) fn parse_anchor(spec: &str) -> AnchorDto {
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
    let mut touch = true;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-m" | "--max-count" => limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(limit),
            "--min-score" => {
                min_score = it.next().and_then(|v| v.parse().ok()).unwrap_or(min_score);
            }
            "--no-touch" => touch = false,
            "--" => query = take_rest(query, it.by_ref(), RECALL_USAGE)?,
            text if !text.starts_with('-') && query.is_none() => query = Some(text.to_string()),
            _ => {}
        }
    }
    let Some(query) = query else {
        bail!(RECALL_USAGE);
    };

    let ctx = repo_context()?;
    let response = ctx
        .client
        .recall(&RecallRequest {
            repo_key: Some(memory_scope_key(&ctx.identity)),
            query,
            limit,
            touch,
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

pub async fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("brief") => brief(&args[1..]).await,
        Some("supersede") => supersede(&args[1..]).await,
        Some("stats") => stats().await,
        Some("reaffirm") => super::lifecycle::reaffirm(&args[1..]).await,
        Some("stale") => super::lifecycle::stale(&args[1..]).await,
        Some("gc") => super::lifecycle::gc(&args[1..]).await,
        Some("restore") => super::lifecycle::restore(&args[1..]).await,
        Some("verify") => super::verify::run(&args[1..]).await,
        _ => feedback(args).await,
    }
}

async fn brief(args: &[String]) -> Result<()> {
    let (mut limit, mut stale_days) = (8, 7);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-m" | "--max-count" => limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(limit),
            "--stale-days" => {
                stale_days = it.next().and_then(|v| v.parse().ok()).unwrap_or(stale_days);
            }
            _ => {}
        }
    }
    let ctx = repo_context()?;
    let response = ctx
        .client
        .brief(&BriefRequest {
            repo_key: Some(memory_scope_key(&ctx.identity)),
            limit,
            stale_days,
        })
        .await?;
    for memory in &response.memories {
        println!(
            "[{} #{}] ({:.2}) {}",
            memory.kind, memory.id, memory.score, memory.content
        );
    }
    if !response.went_stale.is_empty() {
        println!("may be wrong now (anchored code changed):");
        for memory in &response.went_stale {
            println!(
                "[{} #{}] (stale) {}",
                memory.kind, memory.id, memory.content
            );
        }
    }
    Ok(())
}

async fn supersede(args: &[String]) -> Result<()> {
    let [old_id, new_id, ..] = args else {
        bail!("usage: scry memory supersede <old_id> <new_id>");
    };
    let (old_id, new_id): (i64, i64) = (old_id.parse()?, new_id.parse()?);
    let ctx = repo_context()?;
    let response = ctx
        .client
        .supersede(&SupersedeRequest { old_id, new_id })
        .await?;
    if !response.superseded {
        bail!(
            "cannot supersede #{old_id} by #{new_id}: a memory is missing or #{old_id} is already superseded"
        );
    }
    println!("superseded #{old_id} by #{new_id}");
    Ok(())
}

async fn stats() -> Result<()> {
    let ctx = repo_context()?;
    for s in ctx.client.memory_stats().await?.sources {
        println!(
            "{}: {} memories ({} stale), {} retrievals, {} helpful, {} noise, {} archived",
            s.source, s.memories, s.stale, s.retrievals, s.helpful, s.noise, s.archived
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
        .feedback(&FeedbackRequest {
            id,
            helpful,
            repo_key: Some(memory_scope_key(&ctx.identity)),
        })
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
