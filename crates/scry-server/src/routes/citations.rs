//! Citation markers in a model's answer. The prompt asks for `[N]`, and
//! small models still drift to `Source N` or `(Source N)`.

const DRIFTED: &str = "Source ";

/// Rewrites `Source N`, `(Source N)`, and `[Source N]` to `[N]`.
pub(super) fn normalize(answer: &str) -> String {
    let mut out = String::with_capacity(answer.len());
    let mut rest = answer;
    while let Some(at) = rest.find(DRIFTED) {
        let after = &rest[at + DRIFTED.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            out.push_str(&rest[..at + DRIFTED.len()]);
            rest = after;
            continue;
        }
        let (mut head, mut tail) = (&rest[..at], &after[digits..]);
        let wrapped = (head.ends_with('(') && tail.starts_with(')'))
            || (head.ends_with('[') && tail.starts_with(']'));
        if wrapped {
            head = &head[..head.len() - 1];
            tail = &tail[1..];
        }
        out.push_str(head);
        out.push('[');
        out.push_str(&after[..digits]);
        out.push(']');
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Source numbers in `1..=count` that the answer cites as `[N]` or
/// `[N, M]`, ascending.
pub(super) fn cited(answer: &str, count: usize) -> Vec<usize> {
    let mut numbers: Vec<usize> = answer
        .split('[')
        .skip(1)
        .filter_map(|part| part.split_once(']'))
        .flat_map(|(inside, _)| inside.split(','))
        .filter_map(|n| n.trim().parse().ok())
        .filter(|n| (1..=count).contains(n))
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rewrites_drifted_markers_to_brackets() {
        assert_eq!(
            normalize("Bind fails softly (Source 2), see Source 5 and [Source 1]. [2]"),
            "Bind fails softly [2], see [5] and [1]. [2]"
        );
    }

    #[test]
    fn normalize_leaves_prose_and_bracket_markers_alone() {
        let text = "Source code in [3] is the Source of truth";
        assert_eq!(normalize(text), text);
    }

    #[test]
    fn cited_collects_in_range_numbers_once() {
        assert_eq!(cited("a [2] b [5, 1] c [2] d [9] e [x]", 5), vec![1, 2, 5]);
        assert!(cited("no markers", 5).is_empty());
    }
}
