//! Pure text predicates used by the issue-body lint rules.
//!
//! These answer questions about a span of prose — does it terminate sentences, does it
//! name something concrete, does a word occur on a boundary — and hold no knowledge of
//! issue structure. Extracted from `mod.rs` so the rule logic there stays readable and
//! the file stops growing; nothing here touches `IssueDocument`.

pub(super) fn count_sentence_terminals(text: &str) -> usize {
    let chars = text.chars().collect::<Vec<_>>();
    chars
        .iter()
        .enumerate()
        .filter(|(index, character)| match character {
            '?' | '!' => true,
            '.' => chars.get(index + 1).is_none_or(|next| next.is_whitespace()),
            _ => false,
        })
        .count()
}

pub(super) fn has_concrete_goal_object(text: &str) -> bool {
    has_backtick_span(text)
        || has_standalone_number(text)
        || text
            .split(|character: char| !is_word_character(character))
            .any(is_upper_snake_label)
        || has_path_token(text)
}

fn has_backtick_span(text: &str) -> bool {
    let mut remainder = text;
    while let Some(open) = remainder.find('`') {
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find('`') else {
            return false;
        };
        if close > 0 {
            return true;
        }
        remainder = &after_open[close + 1..];
    }
    false
}

fn has_standalone_number(text: &str) -> bool {
    let mut characters = text.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        if !character.is_ascii_digit() {
            continue;
        }
        let mut end = start + character.len_utf8();
        while let Some(&(index, next)) = characters.peek() {
            if !next.is_ascii_digit() {
                break;
            }
            end = index + next.len_utf8();
            characters.next();
        }
        if is_word_boundary(text, start, end) {
            return true;
        }
    }
    false
}

fn has_path_token(text: &str) -> bool {
    text.as_bytes().iter().enumerate().any(|(index, byte)| {
        (*byte == b'/'
            && text[index + 1..]
                .bytes()
                .next()
                .is_some_and(is_path_character))
            || (*byte == b'*'
                && text[index + 1..]
                    .bytes()
                    .next()
                    .is_some_and(|next| next == b'.')
                && text[index + 2..]
                    .bytes()
                    .next()
                    .is_some_and(|next| next.is_ascii_alphabetic()))
    })
}

fn is_upper_snake_label(token: &str) -> bool {
    token.len() >= 2
        && token.starts_with(|character: char| character.is_ascii_uppercase())
        && token.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

pub(super) fn is_path_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'/' | b'-')
}

pub(super) fn first_word_match(text: &str, candidates: &[&str]) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut first = None;
    for candidate in candidates {
        let mut offset = 0;
        while let Some(found) = lower[offset..].find(candidate) {
            let start = offset + found;
            let end = start + candidate.len();
            if is_word_boundary(&lower, start, end) {
                let value = text.get(start..end)?.to_owned();
                if first
                    .as_ref()
                    .is_none_or(|(existing, _): &(usize, String)| start < *existing)
                {
                    first = Some((start, value));
                }
                break;
            }
            offset = end;
        }
    }
    first.map(|(_, value)| value)
}

pub(super) fn is_word_boundary(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(is_word_character) && !after.is_some_and(is_word_character)
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Does `text` call the dangerous builtin, as opposed to merely containing its name?
///
/// The SECURITY rule matched a bare substring, so it reported `ast.literal_eval` — the stdlib
/// parser that accepts literals and rejects every expression form, i.e. the safe alternative
/// the rule's own directive recommends (#3057). An identifier character immediately before the
/// name means some other function; anything else, including a dot, means this one, because the
/// dotted `builtins` form is the dangerous call itself. That is the one way this differs from
/// the `xit` fix in §1.6 of the gate design doc, which had to exclude the dot to stop reporting
/// the stdlib exit call.
///
/// The needle is assembled rather than written out, so this file does not contain the token it
/// searches for — SECURITY scans this repo's own sources, and `detect_todo_left` splits its
/// marker for the same reason.
pub(super) fn contains_dangerous_eval_call(text: &str) -> bool {
    const NEEDLE: &str = concat!("eval", "(");
    text.match_indices(NEEDLE).any(|(index, _)| {
        !text[..index]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

#[cfg(test)]
mod eval_call_tests {
    use super::contains_dangerous_eval_call as dangerous;

    // Assembled for the same reason the predicate assembles its needle.
    fn call(prefix: &str) -> String {
        format!("value = {prefix}{}text)", concat!("eval", "("))
    }

    #[test]
    fn a_bare_or_dotted_call_is_dangerous() {
        assert!(dangerous(&call("")));
        assert!(dangerous(&call("builtins.")));
        assert!(dangerous(&call("self.")));
    }

    #[test]
    fn an_identifier_ending_in_the_name_is_not() {
        assert!(!dangerous(&call("literal_")));
        assert!(!dangerous(&call("ast.literal_")));
        assert!(!dangerous(&call("my")));
        assert!(!dangerous(&call("lead_")));
    }

    #[test]
    fn the_name_without_a_call_is_not() {
        assert!(!dangerous("value = eval"));
        assert!(!dangerous("evaluate(text)"));
        assert!(!dangerous(""));
    }

    #[test]
    fn a_call_at_the_start_of_the_line_is_dangerous() {
        assert!(dangerous(concat!("eval", "(text)")));
    }
}
