use super::{Chunk, Span, lines_chunk};

pub const WINDOW: usize = 40;
pub const OVERLAP: usize = 10;
const SNAP: usize = 5;

pub fn chunk(content: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = content.lines().collect();
    spans(&lines, 0, lines.len())
        .into_iter()
        .map(|span| lines_chunk(&lines, span))
        .collect()
}

/// Windows `lines[from..to]`, snapping each cut back to the nearest blank
/// line within [`SNAP`] lines. Windows holding only blank lines are dropped.
pub(crate) fn spans(lines: &[&str], from: usize, to: usize) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start = from;
    while start < to {
        let mut end = (start + WINDOW).min(to);
        if end < to {
            let snap_floor = end.saturating_sub(SNAP).max(start + 1);
            if let Some(blank) = (snap_floor..end)
                .rev()
                .find(|i| lines[*i].trim().is_empty())
            {
                end = blank;
            }
        }
        if end <= start {
            end = (start + WINDOW).min(to);
        }
        if lines[start..end].iter().any(|line| !line.trim().is_empty()) {
            spans.push(Span {
                start,
                end,
                symbol: None,
            });
        }
        if end >= to {
            break;
        }
        start = end.saturating_sub(OVERLAP).max(start + 1);
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_file_is_one_chunk() {
        let chunks = chunk("a\nb\nc");
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 3));
        assert_eq!(chunks[0].content, "a\nb\nc");
    }

    #[test]
    fn long_file_overlaps_and_covers_all_lines() {
        let text: Vec<String> = (1..=100).map(|i| format!("line {i}")).collect();
        let chunks = chunk(&text.join("\n"));
        assert!(chunks.len() > 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks.last().unwrap().end_line, 100);
        for pair in chunks.windows(2) {
            assert!(pair[1].start_line <= pair[0].end_line + 1);
        }
    }

    #[test]
    fn cuts_snap_to_blank_lines() {
        let mut lines: Vec<String> = (1..=60).map(|i| format!("line {i}")).collect();
        lines[37] = String::new();
        let chunks = chunk(&lines.join("\n"));
        assert_eq!(chunks[0].end_line, 37);
    }
}
