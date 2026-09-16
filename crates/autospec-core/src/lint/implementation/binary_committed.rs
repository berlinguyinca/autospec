//! `BINARY_COMMITTED`: reject a commit that adds an unreviewable blob.
//!
//! A build artefact committed as source is permanent (it survives in the pack
//! even after removal) and a trap (a stale binary a person can run). `go build
//! ./cmd/x` with no `-o` writes the executable into the tree, and `git add -A`
//! sweeps it in (#4645). This module lives in its own file so the
//! already-oversized `implementation.rs` is not grown (file-size ratchet).

use super::{FindingCollector, ImplementationLintRule};
use crate::lint::diff::{DiffFile, UnifiedDiff};

/// Reject a binary file (no reviewer can read a diff of an 8 MB executable),
/// or an executable (mode 100755) that is not a text script.
pub fn detect(diff: &UnifiedDiff, collector: &mut FindingCollector) {
    for file in &diff.files {
        if collector.stopped() {
            return;
        }
        if file.is_binary {
            collector.emit(
                ImplementationLintRule::BinaryCommitted,
                &file.path,
                None,
                "committed binary blob — add a .gitignore entry for it and stage source by name",
            );
            continue;
        }
        if file.is_executable() && !is_text_script(file) {
            collector.emit(
                ImplementationLintRule::BinaryCommitted,
                &file.path,
                None,
                "committed executable (mode 100755) that is not a text script — a build artefact is never source",
            );
        }
    }
}

/// A text script is an executable whose added lines open with a shebang
/// (`#!`). A compiled binary has no shebang, so executability plus no shebang
/// is the unreviewable-blob signature.
fn is_text_script(file: &DiffFile) -> bool {
    file.added_lines()
        .any(|line| line.content.trim_start().starts_with("#!"))
}
