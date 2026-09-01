use std::path::Path;

use scry_server::api::Hit;

/// mgrep-compatible hit line: `./path:start-end (NN.NN% match)`, path
/// shown relative to the invoking directory when possible.
pub fn format_hit(hit: &Hit, repo_root: &Path, cwd: &Path) -> String {
    let path = display_path(repo_root, &hit.relpath, cwd);
    format!(
        "{path}:{}-{} ({:.2}% match)",
        hit.start_line,
        hit.end_line,
        hit.score * 100.0
    )
}

fn display_path(repo_root: &Path, relpath: &str, cwd: &Path) -> String {
    let absolute = repo_root.join(relpath);
    match absolute.strip_prefix(cwd) {
        Ok(from_cwd) => format!("./{}", from_cwd.display()),
        Err(_) => absolute.display().to_string(),
    }
}

pub fn print_hits(hits: &[Hit], repo_root: &Path, cwd: &Path, content: bool) {
    for hit in hits {
        println!("{}", format_hit(hit, repo_root, cwd));
        if content {
            println!("{}\n", hit.content);
        }
    }
}

/// Cross-repo results have no local checkout to resolve against, so the
/// path is prefixed with the repo key instead.
pub fn print_global_hits(hits: &[Hit], content: bool) {
    for hit in hits {
        println!(
            "{}/{}:{}-{} ({:.2}% match)",
            hit.repo_key,
            hit.relpath,
            hit.start_line,
            hit.end_line,
            hit.score * 100.0
        );
        if content {
            println!("{}\n", hit.content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(relpath: &str) -> Hit {
        Hit {
            repo_key: "github.com/x/y".to_string(),
            relpath: relpath.to_string(),
            start_line: 10,
            end_line: 24,
            symbol: None,
            score: 0.9312,
            content: String::new(),
        }
    }

    #[test]
    fn formats_relative_to_cwd() {
        let line = format_hit(&hit("src/lib.rs"), Path::new("/repo"), Path::new("/repo"));
        assert_eq!(line, "./src/lib.rs:10-24 (93.12% match)");
    }

    #[test]
    fn falls_back_to_absolute_outside_cwd() {
        let line = format_hit(
            &hit("src/lib.rs"),
            Path::new("/repo"),
            Path::new("/repo/docs"),
        );
        assert_eq!(line, "/repo/src/lib.rs:10-24 (93.12% match)");
    }
}
