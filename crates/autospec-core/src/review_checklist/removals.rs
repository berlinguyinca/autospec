//! Removed-line surfacing for the review checklist.
//!
//! A reviewer must not only check what a change added but also what it deleted
//! out from under the change. This module walks the removed lines of a file's
//! diff, keeps the ones that read as a comment or a config value, and records
//! for each whether it sat next to an added line. Both kinds of removal are
//! kept — an isolated deletion and one adjacent to a change — because the
//! checklist's job is to make a reviewer *see* the deletion, and the adjacency
//! flag tells them how much to lean on it.

use serde::Serialize;

use super::classify::classify_file_type;
use super::diff::{ChangedFile, LineKind};

/// How far (in diff lines) a removal is looked at for an adjacent change.
pub const ADJACENCY_WINDOW: usize = 3;

/// What kind of thing a removed line was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum RemovalKind {
    /// The line read as a comment in the file's language.
    Comment,
    /// The line read as a configuration value (`key: value` or `key = value`).
    ConfigValue,
    /// A removed line that is neither a comment nor a config value.
    Plain,
}

impl RemovalKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            RemovalKind::Comment => "comment",
            RemovalKind::ConfigValue => "config value",
            RemovalKind::Plain => "line",
        }
    }
}

/// One removed comment or config value, with the context a reviewer needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemovalItem {
    /// The file the removal came from.
    pub path: String,
    /// Old-file line number, when the hunk header made it known.
    pub old_line: Option<usize>,
    pub kind: RemovalKind,
    /// The removed text as it appeared in the diff.
    pub content: String,
    /// Whether an added line was within [`ADJACENCY_WINDOW`] of this removal.
    pub adjacent_to_change: bool,
}

/// Collect the removed comments and config values across the changed files,
/// each flagged for adjacency to an added line. The result is in file order,
/// and within a file it is in old-line order.
pub fn collect_removals(changed_files: &[ChangedFile]) -> Vec<RemovalItem> {
    let mut removals = Vec::new();
    for file in changed_files {
        removals.extend(collect_for_file(file));
    }
    removals
}

/// The removals for one file, in old-line order.
fn collect_for_file(file: &ChangedFile) -> Vec<RemovalItem> {
    let counts = added_index_counts(&file.lines, ADJACENCY_WINDOW);
    let file_type = classify_file_type(&file.path);
    let mut removals = Vec::new();
    for (index, line) in file.lines.iter().enumerate() {
        if line.kind != LineKind::Removed {
            continue;
        }
        let kind = classify_removal(&line.content, file_type);
        if kind == RemovalKind::Plain {
            continue;
        }
        removals.push(RemovalItem {
            path: file.path.clone(),
            old_line: line.old_line,
            kind,
            content: line.content.clone(),
            adjacent_to_change: counts[index] > 0,
        });
    }
    removals
}

/// For each diff-line index, the number of added lines within `window`
/// indices of it. Keeps the adjacency rule in one place.
fn added_index_counts(lines: &[super::diff::DiffLine], window: usize) -> Vec<usize> {
    let added: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.kind == LineKind::Added)
        .map(|(index, _)| index)
        .collect();
    lines
        .iter()
        .enumerate()
        .map(|(index, _)| {
            added
                .iter()
                .filter(|&&a| a.abs_diff(index) <= window)
                .count()
        })
        .collect()
}

/// Classify the text of a removed line: comment, config value, or plain.
///
/// Comment markers are decided by the file's language family, so a `#` in
/// Python is a comment while a `#` in Rust is a doc attribute and not the
/// "explanatory comment" a reviewer is worried about.
pub fn classify_removal(content: &str, file_type: super::classify::FileType) -> RemovalKind {
    let trimmed = content.trim();
    if is_comment_line(trimmed, file_type) {
        return RemovalKind::Comment;
    }
    if is_config_value(trimmed) {
        return RemovalKind::ConfigValue;
    }
    RemovalKind::Plain
}

/// Whether a trimmed line is a comment in the given file type.
pub fn is_comment_line(trimmed: &str, file_type: super::classify::FileType) -> bool {
    let markers = comment_markers(file_type);
    markers.iter().any(|marker| trimmed.starts_with(marker))
}

/// The comment-start markers for a file type.
fn comment_markers(file_type: super::classify::FileType) -> &'static [&'static str] {
    match file_type {
        // C-like languages use `//` and `/*`; a leading `#` is an attribute in
        // Rust and a private field in TypeScript, not a comment.
        super::classify::FileType::TypeScript | super::classify::FileType::Rust => &["//", "/*"],
        // `#` is the comment marker in these languages and file kinds.
        super::classify::FileType::Python
        | super::classify::FileType::Shell
        | super::classify::FileType::Markdown
        | super::classify::FileType::Containerfile
        | super::classify::FileType::Workflow
        | super::classify::FileType::Config => &["#"],
        // The catch-all recognises every marker so a stray comment still shows.
        super::classify::FileType::Other => &["//", "#", "/*"],
    }
}

/// Whether a trimmed line reads as a configuration value: a `key: value` or
/// `key = value` pair. A bare key with no value (a section header) is not one.
pub fn is_config_value(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }
    // `key = value` — the key is a plain name and the value is non-empty; a
    // trailing `;` marks a code statement rather than a config line.
    if let Some((key, value)) = trimmed.split_once('=') {
        return is_config_key(key) && !value.trim().is_empty() && !value.trim_end().ends_with(';');
    }
    // `key: value` — the key is a plain name and there is a value after the
    // colon.
    matches!(
        trimmed.split_once(':'),
        Some((key, value)) if is_config_key(key) && !value.trim().is_empty()
    )
}

/// Whether a (possibly padded) string is a plain configuration key: it starts
/// with a letter or digit and is otherwise only word characters, dots, dashes
/// or underscores.
fn is_config_key(key: &str) -> bool {
    let key = key.trim();
    match key.chars().next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_checklist::classify::FileType;
    use crate::review_checklist::diff::{DiffLine, LineKind};

    fn file(lines: Vec<DiffLine>) -> ChangedFile {
        ChangedFile {
            path: "app/config.ts".to_string(),
            lines,
        }
    }
    fn line(kind: LineKind, content: &str) -> DiffLine {
        DiffLine {
            kind,
            content: content.to_string(),
            old_line: None,
        }
    }

    #[test]
    fn classifies_comment_and_config_value_and_plain() {
        assert_eq!(
            classify_removal("# keep this warm", FileType::Python),
            RemovalKind::Comment
        );
        assert_eq!(
            classify_removal("// legacy timeout", FileType::TypeScript),
            RemovalKind::Comment
        );
        assert_eq!(
            classify_removal("timeout: 30", FileType::Config),
            RemovalKind::ConfigValue
        );
        assert_eq!(
            classify_removal("retries = 5", FileType::Config),
            RemovalKind::ConfigValue
        );
        assert_eq!(
            classify_removal("return compute(value);", FileType::Python),
            RemovalKind::Plain
        );
    }

    #[test]
    fn a_hash_comment_is_only_a_comment_in_comment_langs() {
        assert!(is_comment_line("# note", FileType::Shell));
        assert!(!is_comment_line("#[derive]", FileType::Rust));
    }

    #[test]
    fn config_value_needs_both_sides_of_the_separator() {
        assert!(is_config_value("port: 8080"));
        assert!(is_config_value("host = localhost"));
        assert!(!is_config_value("section_header:"));
        assert!(!is_config_value("- just a list item"));
        assert!(!is_config_value(""));
    }

    #[test]
    fn adjacent_removal_is_flagged_and_far_ones_are_not() {
        let lines = vec![
            line(LineKind::Context, "a"),
            line(LineKind::Removed, "// near the change"),
            line(LineKind::Added, "b"),
            line(LineKind::Context, "c"),
            line(LineKind::Context, "d"),
            line(LineKind::Context, "e"),
            line(LineKind::Removed, "// far from any change"),
        ];
        let removals = collect_removals(&[file(lines)]);
        assert_eq!(removals.len(), 2);
        assert_eq!(removals[0].kind, RemovalKind::Comment);
        assert!(removals[0].adjacent_to_change);
        assert!(!removals[1].adjacent_to_change);
    }

    #[test]
    fn plain_removals_are_not_surfaced() {
        let lines = vec![
            line(LineKind::Removed, "x = 1;"),
            line(LineKind::Added, "y = 2;"),
        ];
        assert!(collect_removals(&[file(lines)]).is_empty());
    }
}
