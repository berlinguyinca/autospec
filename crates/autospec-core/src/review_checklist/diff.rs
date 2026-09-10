//! Unified-diff model for the review checklist.
//!
//! A minimal, total reader over standard unified diff text. It is deliberately
//! narrow — it only tracks, per changed file, the ordered lines and which side
//! of the change each came from — because the checklist needs line positions to
//! decide what a deletion is adjacent to. It is not a full patch applier and
//! never fails: a line it cannot place is dropped, and a file section it cannot
//! open is skipped, so any diff-shaped input yields a well-formed model.

use serde::Serialize;

/// Which side of a change a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum LineKind {
    /// A line added by the change (prefixed `+` in the patch).
    Added,
    /// A line removed by the change (prefixed `-`).
    Removed,
    /// A line present on both sides, shown for context (prefixed ` ` or empty).
    Context,
}

/// One line of a file's diff, with the side it came from and its position in
/// the old file when that is known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub content: String,
    /// Old-file 1-based line number, known for removed and context lines from
    /// the `@@` hunk header.
    pub old_line: Option<usize>,
}

/// One changed file: its path and the ordered diff lines for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedFile {
    pub path: String,
    pub lines: Vec<DiffLine>,
}

/// Parse unified diff text into a list of changed files, in patch order.
///
/// A file section opens at `diff --git` and is read until the next one. The
/// `@@ -old,oldlen +new,newlen @@` header seeds the old-line counter for the
/// removed/context lines that follow; `+` lines carry no old-line number.
pub fn parse_unified_diff(patch: &str) -> Vec<ChangedFile> {
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut current: Option<ChangedFile> = None;
    let mut old_line: Option<usize> = None;
    let mut in_hunk = false;

    for raw in patch.lines() {
        let line = raw.trim_end_matches('\n');
        let trimmed = line.trim_start();

        if trimmed.starts_with("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(ChangedFile {
                path: String::new(),
                lines: Vec::new(),
            });
            old_line = None;
            in_hunk = false;
            continue;
        }

        let Some(file) = current.as_mut() else {
            continue;
        };

        if trimmed.starts_with("+++ ") {
            file.path = plus_path(trimmed);
            old_line = None;
            in_hunk = false;
            continue;
        }
        if trimmed == "---" || trimmed.starts_with("--- ") {
            continue;
        }

        if let Some(body) = trimmed.strip_prefix("@@") {
            old_line = parse_hunk_old_start(body);
            in_hunk = true;
            continue;
        }
        if trimmed.starts_with("\\ No newline") {
            continue;
        }

        if !in_hunk {
            continue;
        }

        if line.is_empty() {
            // Some diff emitters write a bare empty line rather than a single
            // leading space for a blank context line; treat it as context.
            file.lines.push(DiffLine {
                kind: LineKind::Context,
                content: String::new(),
                old_line,
            });
            old_line = Some(old_line.unwrap_or(1).saturating_add(1));
            continue;
        }

        let Some(kind) = classify_prefix(line) else {
            continue;
        };
        let content = line.get(1..).unwrap_or("").to_string();
        let number = if kind == LineKind::Added {
            None
        } else {
            old_line
        };
        file.lines.push(DiffLine {
            kind,
            content,
            old_line: number,
        });
        if kind != LineKind::Added {
            old_line = Some(old_line.unwrap_or(1).saturating_add(1));
        }
    }

    if let Some(file) = current.take() {
        files.push(file);
    }
    files
}

/// The target path from a `+++ b/<path>` header, tolerating `+++ /dev/null`.
fn plus_path(header: &str) -> String {
    let rest = header.trim_start_matches("+++ ");
    if rest == "/dev/null" {
        return String::new();
    }
    rest.strip_prefix("b/")
        .map(str::to_string)
        .unwrap_or_else(|| rest.to_string())
}

/// The old-start from the text after `@@` in `@@ -old,oldlen ... @@`; the
/// comma is optional when the hunk is a single line, so `-3` alone means line 3.
fn parse_hunk_old_start(body: &str) -> Option<usize> {
    let after = body.trim_start().strip_prefix('-')?;
    let number = after.split(',').next()?.trim();
    number.parse::<usize>().ok()
}

/// The line kind from a leading patch marker, if any.
fn classify_prefix(line: &str) -> Option<LineKind> {
    match line.chars().next() {
        Some('+') => Some(LineKind::Added),
        Some('-') => Some(LineKind::Removed),
        Some(' ') => Some(LineKind::Context),
        Some('\t') => Some(LineKind::Context),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_file(files: Vec<ChangedFile>) -> ChangedFile {
        assert_eq!(files.len(), 1);
        files.into_iter().next().expect("one file")
    }

    #[test]
    fn parses_added_removed_and_context_lines_in_order() {
        let patch = "\
diff --git a/app.ts b/app.ts
--- a/app.ts
+++ b/app.ts
@@ -3,4 +3,4 @@
 let a = 1;
-let b = 2;
+let b = 3;
 let c = 4;
";
        let file = only_file(parse_unified_diff(patch));
        assert_eq!(file.path, "app.ts");
        assert_eq!(
            file.lines.iter().map(|l| l.kind).collect::<Vec<_>>(),
            vec![
                LineKind::Context,
                LineKind::Removed,
                LineKind::Added,
                LineKind::Context,
            ]
        );
        assert_eq!(file.lines[0].old_line, Some(3));
        assert_eq!(file.lines[1].old_line, Some(4));
        assert_eq!(file.lines[2].old_line, None);
        assert_eq!(file.lines[2].content, "let b = 3;");
    }

    #[test]
    fn splits_multiple_files_in_patch_order() {
        let patch = "\
diff --git a/one.py b/one.py
--- a/one.py
+++ b/one.py
@@ -1,1 +1,1 @@
-x
+y
diff --git a/two.rs b/two.rs
--- a/two.rs
+++ b/two.rs
@@ -1,1 +1,1 @@
-a
+b
";
        let files = parse_unified_diff(patch);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "one.py");
        assert_eq!(files[1].path, "two.rs");
    }

    #[test]
    fn handles_blank_context_lines_and_single_line_hunks() {
        let patch = "\
diff --git a/x.md b/x.md
--- a/x.md
+++ b/x.md
@@ -1,3 +1,2 @@
 title

-old
 keep
";
        let file = only_file(parse_unified_diff(patch));
        // context, blank context, removed, context
        assert_eq!(file.lines.len(), 4);
        assert_eq!(file.lines[1].kind, LineKind::Context);
        assert_eq!(file.lines[1].content, "");
    }
}
