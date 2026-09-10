//! A review checklist for a set of changed files.
//!
//! The checklist exists because a change that is *only* checked for what it
//! added still ships the three defects this module was built from (issue #231):
//! a config key that does not exist in the schema, a build step that aborts the
//! build, and a workflow gate that needs a job it cannot reach. A reviewer who
//! is handed a checklist — one row per changed file, the checks that apply to
//! that file's type, and the comments and config values the change deleted from
//! under it — has the defects in front of them before they merge.
//!
//! Two invariants hold for every checklist this module produces:
//!
//! - **It is complete.** [`build_review_checklist`] emits an entry for *every*
//!   changed file, in the order given, and sets [`ReviewChecklist::complete`]
//!   from that. Finding a defect in one file does not stop the walk, so the
//!   checklist never ends at the first bad file.
//! - **It surfaces what was removed.** Every removed comment and config value
//!   is carried in [`ReviewChecklist::removals`], each flagged for whether it
//!   sat next to an added line, so a deletion is seen whether or not it is
//!   adjacent to the new code.
//!
//! The module is pure: it reads the diff model and returns the report. It owns
//! no I/O, holds no state, and never fails — a total function over a well-formed
//! [`ChangedFile`] list.
//!
//! # Layout
//!
//! - [`diff`] — the unified-diff model and a small total parser.
//! - [`classify`] — file-type classification and the per-type checks table.
//! - [`removals`] — removed comment / config-value surfacing and adjacency.
//! - [`report`] — the checklist report and its JSON / text rendering.

mod classify;
mod diff;
mod removals;
mod report;

pub use classify::{checks_for, classify_file_type, has_no_applicable_check, FileType};
pub use diff::{parse_unified_diff, ChangedFile, DiffLine, LineKind};
pub use removals::{
    classify_removal, collect_removals, is_comment_line, is_config_value, RemovalItem, RemovalKind,
    ADJACENCY_WINDOW,
};
pub use report::{is_complete, FileCheckEntry, ReviewChecklist};

/// Build the review checklist for the given changed files.
///
/// The result carries one entry per changed file (in input order) plus the
/// removed comments and config values across all of them. It is always
/// [`ReviewChecklist::complete`], because a file is skipped only if it is not in
/// the input.
pub fn build_review_checklist(changed_files: &[ChangedFile]) -> ReviewChecklist {
    let files: Vec<FileCheckEntry> = changed_files
        .iter()
        .map(|file| {
            let file_type = classify_file_type(&file.path);
            FileCheckEntry {
                path: file.path.clone(),
                file_type,
                checks: checks_for(file_type)
                    .iter()
                    .map(|c| c.to_string())
                    .collect(),
                no_applicable_check: has_no_applicable_check(file_type),
            }
        })
        .collect();

    ReviewChecklist {
        files,
        removals: collect_removals(changed_files),
        complete: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_checklist::diff::DiffLine;
    use crate::review_checklist::removals::{RemovalItem, RemovalKind};

    fn file(path: &str) -> ChangedFile {
        ChangedFile {
            path: path.to_string(),
            lines: Vec::new(),
        }
    }

    #[test]
    fn every_changed_file_gets_an_entry_in_input_order() {
        let files = vec![
            file("src/config.ts"),
            file("app/Dockerfile"),
            file(".github/workflows/ci.yml"),
            file("notes.md"),
        ];
        let checklist = build_review_checklist(&files);
        assert_eq!(checklist.files.len(), files.len());
        assert!(checklist.complete);
        assert_eq!(
            checklist
                .files
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "src/config.ts",
                "app/Dockerfile",
                ".github/workflows/ci.yml",
                "notes.md"
            ]
        );
    }

    #[test]
    fn type_specific_checks_are_attached_to_their_files() {
        let files = vec![file("src/config.ts"), file("app/Dockerfile")];
        let checklist = build_review_checklist(&files);
        assert_eq!(checklist.files[0].file_type, FileType::TypeScript);
        assert_eq!(checklist.files[1].file_type, FileType::Containerfile);
        assert!(!checklist.files[0].checks.is_empty());
        assert!(!checklist.files[1].checks.is_empty());
    }

    #[test]
    fn uncheckable_files_are_flagged_not_dropped() {
        let files = vec![file("notes.md")];
        let checklist = build_review_checklist(&files);
        assert_eq!(checklist.files.len(), 1);
        assert!(checklist.files[0].no_applicable_check);
        assert!(checklist.files[0].checks.is_empty());
        assert!(checklist.complete);
    }

    #[test]
    fn finding_a_defect_in_one_file_does_not_end_the_walk() {
        // A file that is plain prose (no checks) sits between two that have
        // them; the checklist still carries all three.
        let files = vec![file("a.rs"), file("readme.md"), file("b.py")];
        let checklist = build_review_checklist(&files);
        assert_eq!(checklist.files.len(), 3);
        assert!(checklist.complete);
    }

    #[test]
    fn removals_are_carried_into_the_report() {
        let file = ChangedFile {
            path: "settings.yaml".to_string(),
            lines: vec![
                DiffLine {
                    kind: LineKind::Removed,
                    content: "timeout: 30".to_string(),
                    old_line: Some(2),
                },
                DiffLine {
                    kind: LineKind::Added,
                    content: "timeout: 60".to_string(),
                    old_line: None,
                },
            ],
        };
        let checklist = build_review_checklist(&[file]);
        assert_eq!(checklist.removals.len(), 1);
        assert_eq!(checklist.removals[0].kind, RemovalKind::ConfigValue);
        assert!(checklist.removals[0].adjacent_to_change);
    }

    #[test]
    fn empty_input_yields_an_empty_but_complete_checklist() {
        let checklist = build_review_checklist(&[]);
        assert!(checklist.files.is_empty());
        assert!(checklist.removals.is_empty());
        assert!(checklist.complete);
    }

    #[test]
    fn a_report_can_be_built_without_the_builder() {
        // is_complete is a property of the report vs the inputs, so a hand-built
        // report can be checked the same way.
        let files = vec![file("a.rs")];
        let checklist = ReviewChecklist {
            files: vec![FileCheckEntry {
                path: "a.rs".to_string(),
                file_type: FileType::Rust,
                checks: Vec::new(),
                no_applicable_check: false,
            }],
            removals: vec![RemovalItem {
                path: "a.rs".to_string(),
                old_line: None,
                kind: RemovalKind::Comment,
                content: "// gone".to_string(),
                adjacent_to_change: true,
            }],
            complete: true,
        };
        assert!(is_complete(&checklist, &files));
    }
}
