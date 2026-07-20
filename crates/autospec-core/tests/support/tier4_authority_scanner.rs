use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read Tier 4 module directory") {
        let path = entry.expect("read Tier 4 module entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

pub(crate) fn collect_tier4_verifier_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    collect_tier4_verifier_sources_inside(directory, sources, false);
}

fn collect_tier4_verifier_sources_inside(
    directory: &Path,
    sources: &mut Vec<PathBuf>,
    inside_tier4: bool,
) {
    let inside_tier4 = inside_tier4
        || directory
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "tier4");
    for entry in fs::read_dir(directory).expect("read Tier 4 verifier directory") {
        let path = entry.expect("read Tier 4 verifier entry").path();
        if path.is_dir() {
            collect_tier4_verifier_sources_inside(&path, sources, inside_tier4);
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && (inside_tier4
                || path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem == "tier4" || stem.starts_with("tier4_")))
        {
            sources.push(path);
        }
    }
}

pub(crate) fn production_code(source: &str, scope: &str) -> String {
    strip_test_items(source, scope)
}

fn strip_test_items(source: &str, scope: &str) -> String {
    const TEST_ATTRIBUTE: &[u8] = b"#[cfg(test)]";
    let masked = source_mask(source);
    let mut production = String::with_capacity(source.len());
    let mut copied_through = 0;
    let mut cursor = 0;
    while let Some(offset) = find_bytes(&masked[cursor..], TEST_ATTRIBUTE) {
        let item_start = cursor + offset;
        let item_end = test_item_end(&masked, item_start + TEST_ATTRIBUTE.len(), scope);
        production.push_str(&source[copied_through..item_start]);
        production.push('\n');
        copied_through = item_end;
        cursor = item_end;
    }
    production.push_str(&source[copied_through..]);
    production
}

fn test_item_end(masked: &[u8], start: usize, scope: &str) -> usize {
    let delimiter = masked[start..]
        .iter()
        .position(|byte| matches!(*byte, b'{' | b';'))
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("{scope} has an unterminated cfg(test) item"));
    if masked[delimiter] == b';' {
        return delimiter + 1;
    }
    let mut depth = 0_u32;
    for (index, byte) in masked.iter().enumerate().skip(delimiter) {
        match *byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth
                    .checked_sub(1)
                    .unwrap_or_else(|| panic!("{scope} has an unbalanced cfg(test) item"));
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
    }
    panic!("{scope} has an unterminated cfg(test) item")
}

pub(crate) fn has_write_capable_github_argv(source: &str) -> bool {
    let literals = string_literals(source);
    let has_write_subcommand = literals.windows(2).any(|window| {
        matches!(
            (window[0].as_str(), window[1].as_str()),
            ("issue", "create" | "edit" | "comment" | "close" | "reopen")
                | (
                    "pr",
                    "create" | "edit" | "comment" | "merge" | "close" | "reopen"
                )
                | ("label", "create" | "edit" | "delete")
        )
    });
    let has_write_api = literals.iter().any(|literal| literal == "api")
        && literals
            .iter()
            .any(|literal| literal == "-X" || literal == "--method")
        && literals
            .iter()
            .any(|literal| matches!(literal.as_str(), "POST" | "PUT" | "PATCH" | "DELETE"));
    has_write_subcommand || has_write_api
}

pub(crate) fn code_without_comments_and_literals(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut code = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = raw_string_end(bytes, index) {
            code.push(' ');
            index = end;
            continue;
        }
        match bytes[index..] {
            [b'/', b'/', ..] => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                code.push(' ');
            }
            [b'/', b'*', ..] => {
                index = block_comment_end(bytes, index + 2);
                code.push(' ');
            }
            [b'"', ..] => {
                index = quoted_end(bytes, index + 1);
                code.push(' ');
            }
            _ => {
                code.push(bytes[index] as char);
                index += 1;
            }
        }
    }
    code
}

fn source_mask(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        let end = if let Some(end) = raw_string_end(bytes, index) {
            Some(end)
        } else {
            match bytes[index..] {
                [b'/', b'/', ..] => Some(line_comment_end(bytes, index + 2)),
                [b'/', b'*', ..] => Some(block_comment_end(bytes, index + 2)),
                [b'"', ..] => Some(quoted_end(bytes, index + 1)),
                _ => None,
            }
        };
        if let Some(end) = end {
            for byte in &mut masked[index..end] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
            index = end;
        } else {
            index += 1;
        }
    }
    masked
}

fn string_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut literals = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = raw_string_end(bytes, index) {
            let quote = bytes[index..end]
                .iter()
                .position(|byte| *byte == b'"')
                .map(|offset| index + offset)
                .expect("raw string quote");
            let hashes = bytes[index..quote]
                .iter()
                .filter(|byte| **byte == b'#')
                .count();
            literals.push(String::from_utf8_lossy(&bytes[quote + 1..end - hashes - 1]).into());
            index = end;
            continue;
        }
        match bytes[index..] {
            [b'/', b'/', ..] => index = line_comment_end(bytes, index + 2),
            [b'/', b'*', ..] => index = block_comment_end(bytes, index + 2),
            [b'"', ..] => {
                let end = quoted_end(bytes, index + 1);
                literals.push(String::from_utf8_lossy(&bytes[index + 1..end - 1]).into());
                index = end;
            }
            _ => index += 1,
        }
    }
    literals
}

fn line_comment_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn block_comment_end(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 1;
    while index < bytes.len() && depth > 0 {
        match bytes[index..] {
            [b'/', b'*', ..] => {
                depth += 1;
                index += 2;
            }
            [b'*', b'/', ..] => {
                depth -= 1;
                index += 2;
            }
            _ => index += 1,
        }
    }
    index
}

fn quoted_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
        } else if bytes[index] == b'"' {
            return index + 1;
        } else {
            index += 1;
        }
    }
    index
}

fn raw_string_end(bytes: &[u8], index: usize) -> Option<usize> {
    let mut cursor = match bytes.get(index) {
        Some(b'r') => index + 1,
        Some(b'b') if bytes.get(index + 1) == Some(&b'r') => index + 2,
        _ => return None,
    };
    let hash_start = cursor;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    let hashes = cursor - hash_start;
    cursor += 1;
    while cursor < bytes.len() {
        let end = cursor.saturating_add(hashes + 1);
        if bytes[cursor] == b'"'
            && end <= bytes.len()
            && bytes[cursor + 1..end].iter().all(|byte| *byte == b'#')
        {
            return Some(end);
        }
        cursor += 1;
    }
    Some(cursor)
}
