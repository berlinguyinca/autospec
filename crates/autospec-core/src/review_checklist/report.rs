//! The review checklist report.
//!
//! The checklist is the thing a reviewer reads. It has one entry per changed
//! file (with the checks that apply to that file's type) and the list of
//! removed comments and config values. `complete` is the invariant the whole
//! module is built around: the checklist is only `complete` when every changed
//! file has an entry, so a reviewer can tell a finished checklist from a
//! truncated one at a glance.

use serde::Serialize;

use super::classify::FileType;
use super::diff::ChangedFile;
use super::removals::RemovalItem;

/// The checklist entry for one changed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileCheckEntry {
    /// The file path as it appeared in the diff.
    pub path: String,
    /// The file type the checks were chosen from.
    pub file_type: FileType,
    /// The checks that apply to this file's type, in review order.
    pub checks: Vec<String>,
    /// True when the file's type carries no mechanical checks; the entry is
    /// present so the file is not silently skipped.
    pub no_applicable_check: bool,
}

/// A complete review checklist over a set of changed files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewChecklist {
    /// One entry per changed file, in the order the files were given.
    pub files: Vec<FileCheckEntry>,
    /// Removed comments and config values, across all changed files.
    pub removals: Vec<RemovalItem>,
    /// True when every changed file has an entry. A checklist is only ever
    /// reported as complete when it was — see [`is_complete`].
    pub complete: bool,
}

impl ReviewChecklist {
    /// Serialize to compact JSON. Serialization cannot fail for these types,
    /// but a defensive fallback keeps the method total.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|error| {
            format!("{{\"error\":\"failed to serialize review checklist: {error}\"}}")
        })
    }

    /// Render a short human-readable form: one line per file, then one line per
    /// removal.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for entry in &self.files {
            let flag = if entry.no_applicable_check {
                " (no applicable check)"
            } else {
                ""
            };
            out.push_str(&format!(
                "{} [{}]:{} {}\n",
                entry.path,
                entry.file_type.label(),
                flag,
                entry.checks.join("; ")
            ));
        }
        for removal in &self.removals {
            let adj = if removal.adjacent_to_change {
                " adjacent"
            } else {
                ""
            };
            let where_ = removal
                .old_line
                .map(|n| format!("@{}", n))
                .unwrap_or_default();
            out.push_str(&format!(
                "removed {} {}{}{}: {}\n",
                removal.kind.label(),
                removal.path,
                where_,
                adj,
                removal.content
            ));
        }
        out
    }
}

/// The checklist is complete exactly when it has one entry per changed file.
///
/// The caller computes this from the inputs rather than trusting the report,
/// so a report can be checked for completeness even if it was built elsewhere.
pub fn is_complete(checklist: &ReviewChecklist, changed_files: &[ChangedFile]) -> bool {
    checklist.files.len() == changed_files.len()
        && checklist
            .files
            .iter()
            .zip(changed_files.iter())
            .all(|(entry, file)| entry.path == file.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_checklist::build_review_checklist;

    fn file(path: &str) -> ChangedFile {
        ChangedFile {
            path: path.to_string(),
            lines: Vec::new(),
        }
    }

    #[test]
    fn json_round_trips_and_marks_completeness() {
        let files = vec![file("a.rs"), file("b.md")];
        let checklist = build_review_checklist(&files);
        let json = checklist.to_json();
        assert!(json.contains("\"complete\":true"));
        assert!(json.contains("a.rs"));
        assert!(json.contains("b.md"));
    }

    #[test]
    fn text_lists_every_file_and_flags_uncheckable_ones() {
        let files = vec![file("a.rs"), file("notes.md")];
        let checklist = build_review_checklist(&files);
        let text = checklist.to_text();
        assert!(text.contains("a.rs"));
        // A Rust file has real checks; the note is flagged as uncheckable.
        assert!(text.contains("build is green"));
        assert!(text.contains("no applicable check"));
    }

    #[test]
    fn completeness_requires_one_entry_per_file_in_order() {
        let files = vec![file("a.rs"), file("b.md")];
        let full = build_review_checklist(&files);
        assert!(is_complete(&full, &files));

        let truncated = ReviewChecklist {
            files: full.files[..1].to_vec(),
            removals: Vec::new(),
            complete: false,
        };
        assert!(!is_complete(&truncated, &files));
    }
}
