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
