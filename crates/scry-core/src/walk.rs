use std::path::Path;
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;

use crate::Result;

pub const IGNORE_FILE: &str = ".scryignore";

pub const DEFAULT_IGNORES: &[&str] = &[
    "*.lock",
    "*.bin",
    "*.ipynb",
    "*.pyc",
    "*.safetensors",
    "*.sqlite",
    "*.pt",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub relpath: String,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Lists indexable files under `root`, repo-relative with `/` separators,
/// sorted by path. Skips hidden files, `.gitignore`/`.scryignore` matches,
/// [`DEFAULT_IGNORES`], empty files, and files over `max_file_size`.
pub fn walk_repo(root: &Path, max_file_size: u64) -> Result<Vec<FileEntry>> {
    let mut overrides = OverrideBuilder::new(root);
    for pattern in DEFAULT_IGNORES {
        overrides
            .add(&format!("!{pattern}"))
            .expect("static ignore pattern");
    }

    let mut entries = Vec::new();
    for dirent in WalkBuilder::new(root)
        .add_custom_ignore_filename(IGNORE_FILE)
        .overrides(overrides.build().expect("static override set"))
        .build()
    {
        let Ok(dirent) = dirent else { continue };
        if !dirent.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let meta = match dirent.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.len() == 0 || meta.len() > max_file_size {
            continue;
        }
        let Ok(rel) = dirent.path().strip_prefix(root) else {
            continue;
        };
        let Some(relpath) = rel.to_str() else {
            tracing::warn!(path = %rel.display(), "skipping non-utf8 path");
            continue;
        };
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64);
        entries.push(FileEntry {
            relpath: relpath.replace('\\', "/"),
            size: meta.len(),
            mtime_ms,
        });
    }
    entries.sort_by(|a, b| a.relpath.cmp(&b.relpath));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str, contents: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn walks_files_sorted_and_relative() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "src/main.rs", b"fn main() {}");
        touch(dir.path(), "README.md", b"# hi");
        let paths: Vec<_> = walk_repo(dir.path(), 1_000_000)
            .unwrap()
            .into_iter()
            .map(|e| e.relpath)
            .collect();
        assert_eq!(paths, ["README.md", "src/main.rs"]);
    }

    #[test]
    fn skips_default_ignores_hidden_empty_and_oversized() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Cargo.lock", b"locked");
        touch(dir.path(), "model.safetensors", b"weights");
        touch(dir.path(), ".hidden", b"secret");
        touch(dir.path(), "empty.txt", b"");
        touch(dir.path(), "big.txt", &[b'x'; 64]);
        touch(dir.path(), "ok.txt", b"fine");
        let paths: Vec<_> = walk_repo(dir.path(), 32)
            .unwrap()
            .into_iter()
            .map(|e| e.relpath)
            .collect();
        assert_eq!(paths, ["ok.txt"]);
    }

    #[test]
    fn honors_scryignore() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), ".scryignore", b"vendor/\n");
        touch(dir.path(), "vendor/lib.js", b"x");
        touch(dir.path(), "app.js", b"x");
        let paths: Vec<_> = walk_repo(dir.path(), 1_000_000)
            .unwrap()
            .into_iter()
            .map(|e| e.relpath)
            .collect();
        assert_eq!(paths, ["app.js"]);
    }
}
