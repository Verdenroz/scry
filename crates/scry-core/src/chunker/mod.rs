//! Splits file content into retrieval chunks. Recognized languages get
//! tree-sitter definition-level chunks with symbol paths; everything else
//! falls back to blank-line-snapped windows. Line numbers are 1-based and
//! inclusive so hits print as `path:start-end`.

pub mod line_window;
mod treesitter;

pub use treesitter::Language;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub start_line: u32,
    pub end_line: u32,
    pub symbol: Option<String>,
    pub content: String,
}

pub fn chunk_file(relpath: &str, content: &str) -> Vec<Chunk> {
    if content.contains('\0') {
        return Vec::new();
    }
    match Language::from_path(relpath) {
        Some(language) => {
            treesitter::chunk(language, content).unwrap_or_else(|| line_window::chunk(content))
        }
        None => line_window::chunk(content),
    }
}

pub(crate) fn lines_chunk(
    lines: &[&str],
    start: usize,
    end: usize,
    symbol: Option<String>,
) -> Chunk {
    Chunk {
        start_line: start as u32 + 1,
        end_line: end as u32,
        symbol,
        content: lines[start..end].join("\n"),
    }
}
