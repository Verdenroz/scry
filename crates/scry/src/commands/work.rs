use anyhow::{Result, bail};
use scry_core::store::WorkList;
use scry_server::api::{
    WorkAddRequest, WorkCloseRequest, WorkListRequest, WorkSetRequest, WorkStateRequest,
};

use super::{memory_scope_key, repo_context};

const USAGE: &str = "usage: scry work [--all]
       scry work show <list> [--all]
       scry work add <list> \"task\" [--status todo|doing|done|dropped] [--note \"text\"]
       scry work set <id> [todo|doing|done|dropped] [--note \"text\"]
       scry work state <list> key=value...   (key= clears the key)
       scry work close|reopen <list>";

pub async fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None | Some("--all") => show(None, args).await,
        Some("show") => match args.get(1) {
            Some(list) => show(Some(list.clone()), &args[2..]).await,
            None => bail!("{USAGE}"),
        },
        Some("add") => add(&args[1..]).await,
        Some("set") => set(&args[1..]).await,
        Some("state") => state(&args[1..]).await,
        Some(verb @ ("close" | "reopen")) => match args.get(1) {
            Some(list) => close(list, verb == "reopen").await,
            None => bail!("{USAGE}"),
        },
        Some(_) => bail!("{USAGE}"),
    }
}

async fn show(name: Option<String>, args: &[String]) -> Result<()> {
    let all = args.iter().any(|arg| arg == "--all");
    let ctx = repo_context()?;
    let response = ctx
        .client
        .work_lists(&WorkListRequest {
            repo_key: memory_scope_key(&ctx.identity),
            name,
            all,
        })
        .await?;
    print!("{}", render(&response.lists, response.now));
    Ok(())
}

async fn add(args: &[String]) -> Result<()> {
    let mut positionals = Vec::new();
    let (mut status, mut note) = ("todo".to_string(), None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--status" => status = it.next().cloned().unwrap_or(status),
            "--note" => note = it.next().cloned(),
            text => positionals.push(text.to_string()),
        }
    }
    let [list, task] = positionals.as_slice() else {
        bail!("{USAGE}");
    };
    let ctx = repo_context()?;
    let response = ctx
        .client
        .work_add(&WorkAddRequest {
            repo_key: memory_scope_key(&ctx.identity),
            list: list.clone(),
            task: task.clone(),
            status,
            note,
        })
        .await?;
    println!("#{}", response.id);
    Ok(())
}

async fn set(args: &[String]) -> Result<()> {
    let mut positionals = Vec::new();
    let mut note = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--note" => note = it.next().cloned(),
            text => positionals.push(text),
        }
    }
    let (id, status) = match positionals.as_slice() {
        [id] if note.is_some() => (id, None),
        [id, status] => (id, Some(status.to_string())),
        _ => bail!("{USAGE}"),
    };
    let Ok(id) = id.trim_start_matches('#').parse::<i64>() else {
        bail!("{USAGE}");
    };
    let ctx = repo_context()?;
    ctx.client
        .work_set(&WorkSetRequest {
            repo_key: memory_scope_key(&ctx.identity),
            id,
            status,
            note,
        })
        .await?;
    println!("updated #{id}");
    Ok(())
}

async fn state(args: &[String]) -> Result<()> {
    let Some((list, pairs)) = args.split_first() else {
        bail!("{USAGE}");
    };
    let entries = pairs
        .iter()
        .map(|pair| match pair.split_once('=') {
            Some((key, "")) => Ok((key.to_string(), None)),
            Some((key, value)) => Ok((key.to_string(), Some(value.to_string()))),
            None => bail!("{USAGE}"),
        })
        .collect::<Result<Vec<_>>>()?;
    if entries.is_empty() {
        bail!("{USAGE}");
    }
    let ctx = repo_context()?;
    ctx.client
        .work_state(&WorkStateRequest {
            repo_key: memory_scope_key(&ctx.identity),
            list: list.clone(),
            entries,
        })
        .await?;
    println!("updated {list}");
    Ok(())
}

async fn close(list: &str, reopen: bool) -> Result<()> {
    let ctx = repo_context()?;
    ctx.client
        .work_close(&WorkCloseRequest {
            repo_key: memory_scope_key(&ctx.identity),
            list: list.to_string(),
            reopen,
        })
        .await?;
    println!("{} {list}", if reopen { "reopened" } else { "closed" });
    Ok(())
}

fn age(seconds: i64) -> String {
    match seconds {
        s if s < 60 => "just now".to_string(),
        s if s < 3_600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3_600),
        s => format!("{}d ago", s / 86_400),
    }
}

fn render(lists: &[WorkList], now: i64) -> String {
    let mut out = String::new();
    for list in lists {
        let closed = if list.closed { "closed, " } else { "" };
        out += &format!(
            "## {} ({closed}updated {})\n",
            list.name,
            age(now - list.updated_at)
        );
        for (key, value) in &list.state {
            out += &format!("  {key}: {value}\n");
        }
        for item in &list.items {
            out += &format!("  #{} [{}] {}\n", item.id, item.status, item.task);
            if let Some(note) = &item.note {
                out += &format!("      note: {note}\n");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use scry_core::store::WorkItem;

    #[test]
    fn renders_state_before_items_with_notes_indented() {
        let lists = vec![WorkList {
            name: "ship".to_string(),
            closed: false,
            updated_at: 1_000,
            state: vec![("pr".to_string(), "#12".to_string())],
            items: vec![WorkItem {
                id: 4,
                task: "write tests".to_string(),
                status: "doing".to_string(),
                note: Some("flaky on CI".to_string()),
                updated_at: 1_000,
            }],
        }];
        assert_eq!(
            render(&lists, 1_000 + 7_200),
            "## ship (updated 2h ago)\n  pr: #12\n  #4 [doing] write tests\n      note: flaky on CI\n"
        );
    }

    #[test]
    fn age_picks_the_coarsest_whole_unit() {
        assert_eq!(age(59), "just now");
        assert_eq!(age(61), "1m ago");
        assert_eq!(age(3_600), "1h ago");
        assert_eq!(age(3 * 86_400 + 5), "3d ago");
    }
}
