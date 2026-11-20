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
}
