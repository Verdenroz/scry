//! Merges undersized spans into a neighbour so a bare attribute, a `use`
//! group, or a one-line `mod` declaration never embeds on its own.

use super::Span;

/// Spans shorter than this merge forward into the next span; a short
/// trailing span merges back into the previous one.
pub(crate) const MIN_LINES: usize = 4;

const SEPARATOR: &str = " > ";

pub(crate) fn coalesce(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut pending: Option<Span> = None;
    for span in spans {
        let span = match pending.take() {
            Some(short) => merge(short, span),
            None => span,
        };
        if span.end - span.start < MIN_LINES {
            pending = Some(span);
        } else {
            out.push(span);
        }
    }
    if let Some(short) = pending {
        let merged = match out.pop() {
            Some(last) => merge(last, short),
            None => short,
        };
        out.push(merged);
    }
    out
}

fn merge(first: Span, second: Span) -> Span {
    Span {
        start: first.start.min(second.start),
        end: first.end.max(second.end),
        symbol: shared_symbol(first.symbol.as_deref(), second.symbol.as_deref()),
    }
}

/// `S > a` and `S > b` merge under `S`; unrelated symbols merge under none.
fn shared_symbol(a: Option<&str>, b: Option<&str>) -> Option<String> {
    match (a, b) {
        (None, other) | (other, None) => other.map(str::to_string),
        (Some(a), Some(b)) => {
            let shared = a
                .split(" > ")
                .zip(b.split(" > "))
                .take_while(|(x, y)| x == y)
                .fold(0, |len, (x, _)| len + x.len() + SEPARATOR.len());
            (shared > 0).then(|| a[..shared - SEPARATOR.len()].to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: usize, end: usize, symbol: Option<&str>) -> Span {
        Span {
            start,
            end,
            symbol: symbol.map(str::to_string),
        }
    }

    #[test]
    fn short_spans_merge_forward_and_keep_the_definition_symbol() {
        let out = coalesce(vec![span(0, 2, None), span(2, 10, Some("alpha"))]);
        assert_eq!(out, vec![span(0, 10, Some("alpha"))]);
    }

    #[test]
    fn short_trailing_span_merges_back() {
        let out = coalesce(vec![span(0, 10, Some("alpha")), span(10, 11, None)]);
        assert_eq!(out, vec![span(0, 11, Some("alpha"))]);
    }

    #[test]
    fn runs_of_one_liners_accumulate() {
        let out = coalesce(vec![
            span(0, 1, Some("a")),
            span(1, 2, Some("b")),
            span(2, 3, Some("c")),
            span(3, 4, Some("d")),
            span(4, 12, Some("e")),
        ]);
        assert_eq!(out, vec![span(0, 4, None), span(4, 12, Some("e"))]);
    }

    #[test]
    fn sibling_methods_merge_under_their_parent() {
        let out = coalesce(vec![span(0, 2, Some("S > a")), span(2, 5, Some("S > b"))]);
        assert_eq!(out, vec![span(0, 5, Some("S"))]);
    }

    #[test]
    fn long_spans_pass_through() {
        let spans = vec![span(0, 10, Some("a")), span(10, 20, Some("b"))];
        assert_eq!(coalesce(spans.clone()), spans);
    }
}
