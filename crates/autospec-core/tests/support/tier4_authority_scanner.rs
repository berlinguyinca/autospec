use std::fs;
use std::path::{Path, PathBuf};

use crate::matcher::code_tokens;

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
    let Some((production, tests)) = source.split_once("\n#[cfg(test)]") else {
        return source.to_string();
    };
    let tokens = code_tokens(&code_without_comments_and_literals(tests));
    assert!(
        is_terminal_test_module(&tokens),
        "{scope} has production authority after its test module"
    );
    production.to_string()
}

fn is_terminal_test_module(tokens: &[String]) -> bool {
    if !matches!(
        tokens,
        [module, name, open, ..] if module == "mod" && name == "tests" && open == "{"
    ) {
        return false;
    }
    let mut depth = 0_u32;
    for (index, token) in tokens.iter().enumerate().skip(2) {
        match token.as_str() {
            "{" => depth += 1,
            "}" => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
                if depth == 0 {
                    return index + 1 == tokens.len();
                }
            }
            _ => {}
        }
    }
    false
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
