//! Citation markers in a model's answer. The prompt asks for `[N]`, and
//! small models still drift to `Source N` or `(Source N)`.

const DRIFTED: &str = "Source ";
const SEPARATORS: [&str; 4] = [", and ", ", ", " and ", "; "];

fn closer(head: &str) -> Option<char> {
    match head.chars().next_back()? {
        '(' => Some(')'),
        '[' => Some(']'),
        _ => None,
    }
}

/// The numbers of a `Source N, Source M` or `Source N, M` list that runs up
/// to `close`, and the bytes it spans including `close`.
fn group(text: &str, close: char) -> Option<(Vec<&str>, usize)> {
    let mut numbers = Vec::new();
    let mut rest = text.strip_prefix(DRIFTED)?;
    loop {
        let after = rest.strip_prefix(DRIFTED).unwrap_or(rest);
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        numbers.push(&after[..digits]);
        rest = &after[digits..];
        if let Some(tail) = rest.strip_prefix(close) {
            return Some((numbers, text.len() - tail.len()));
        }
        rest = SEPARATORS.iter().find_map(|s| rest.strip_prefix(s))?;
    }
}

fn push_marker(out: &mut String, number: &str) {
    out.push('[');
    out.push_str(number);
    out.push(']');
}

/// Rewrites `Source N`, `(Source N)`, and `[Source N]` to `[N]`, and a
/// wrapped list such as `[Source 1, Source 2]` to `[1], [2]`.
pub(super) fn normalize(answer: &str) -> String {
    let mut out = String::with_capacity(answer.len());
    let mut rest = answer;
    while let Some(at) = rest.find(DRIFTED) {
        let head = &rest[..at];
        let wrapped = closer(head).and_then(|close| group(&rest[at..], close));
        if let Some((numbers, len)) = wrapped {
            out.push_str(&head[..head.len() - 1]);
            for (i, number) in numbers.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                push_marker(&mut out, number);
            }
            rest = &rest[at + len..];
            continue;
        }
        let after = &rest[at + DRIFTED.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            out.push_str(&rest[..at + DRIFTED.len()]);
        } else {
            out.push_str(head);
            push_marker(&mut out, &after[..digits]);
        }
        rest = &after[digits..];
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
    fn normalize_unwraps_a_list_of_markers() {
        assert_eq!(
            normalize("Cached [Source 1, Source 2] and (Source 3 and Source 4)."),
            "Cached [1], [2] and [3], [4]."
        );
        assert_eq!(
            normalize("Short [Source 2, 3]; prose (Source 1, see also)"),
            "Short [2], [3]; prose ([1], see also)"
        );
    }

    #[test]
    fn normalize_leaves_prose_and_bracket_markers_alone() {
        let text = "Source code in [3] is the Source of truth";
        assert_eq!(normalize(text), text);
    }
}
