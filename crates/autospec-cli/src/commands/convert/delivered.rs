//! The already-delivered residue (issue #4501).
//!
//! Eighteen delivered issues sat open in the conversion backlog for weeks:
//! the PR that merged them put the issue number in the **title**, not a
//! closing keyword in the body, so the tracker never closed them. Every
//! pass then re-fetched the patch, applied it, ran a gate, and reached
//! "no change" — for work that had already landed. The patch that yields an
//! empty diff against the base is the residue's signature, and it is cheap
//! to test: `git apply --reverse --check` succeeds exactly when the tree
//! already carries the patch's changes.
//!
//! The invariant: **a merge must close the issue it delivers, by a
//! mechanism the tracker enforces, not by a convention in a title.** This
//! module is the detection half of the fix — the pass reports the residue
//! as its own category (never offered, never gated) so the backlog number
//! means what a reader assumes it means, and the PR the pass opens carries
//! the closing keyword so the residue stops accumulating.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::git::run_git_in;
/// The candidates whose patch yields no change against the base.
///
/// One shared worktree at the base for the whole pass — not one per patch —
/// and a read-only check per patch: `git apply --reverse --check` succeeds
/// exactly when the tree already carries the patch's changes, so the check
/// is the test. Any check failure reads as *not delivered* (fail-closed:
/// the candidate goes on to the gate, which measures it the usual way); a
/// check can never read a patch as delivered that the base does not carry.
///
/// `None` from the setup (no fetch, no worktree) means the detection ran at
/// all: the caller reports no delivered candidates, exactly the state
/// before this detection existed.
pub(super) fn detect_delivered(
    base_ref: &str,
    patches: &[(u64, std::path::PathBuf)],
) -> BTreeSet<u64> {
    // PID-suffixed: a pass is one process, and concurrent passes must not
    // share the check worktree.
    let worktree = std::env::temp_dir().join(format!(
        "autospec-conv-delivered-check-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&worktree);
    if let Err(error) = run_git_in(
        &std::env::current_dir().ok().unwrap_or_default(),
        &["worktree", "add", "--detach", worktree.to_str().unwrap_or_default(), base_ref],
    ) {
        eprintln!(
            "WARN: delivered-patch detection skipped: could not add the base worktree ({error})"
        );
        return BTreeSet::new();
    }
    let mut delivered = BTreeSet::new();
    for (issue, path) in patches {
        let output = Command::new("git")
            .args(["apply", "--reverse", "--check"])
            .arg(path)
            .current_dir(&worktree)
            .output()
            .ok();
        if matches!(output, Some(o) if o.status.success()) {
            delivered.insert(*issue);
        }
    }
    super::git::teardown_worktree(&worktree);
    delivered
}

/// The one-off archive of a delivered patch: move it under the issue's
/// `superseded/` directory (archival, never deletion — the same path the
/// base-superseded apply uses), so the next pass no longer enumerates it
/// and the queue entry it held is released. Returns `true` when the patch
/// left disk.
pub(super) fn archive_patch(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let _ = fs::create_dir_all(parent.join("superseded"));
    let archive = autospec_core::stored_output::superseded_archive_path(
        parent,
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    fs::rename(path, &archive).is_ok()
}

/// The body of the PR the pass opens for a converted patch. The closing
/// keyword is the mechanism the tracker enforces (#4501): a number in the
/// title is documentation, not a link, and seventeen merged PRs left their
/// issues open on exactly that convention.
pub(super) fn pr_body(
    issue: u64,
    scope: &[String],
    resolutions: &[super::conflict::ResolutionRecord],
) -> String {
    format!(
        "Converted from the agent patch for issue #{}.\n\nGate scope: {} \
         (derived from the patch's touched crates).\n\nSource spec: n/a \
         (patch-to-PR conversion pass).{}\n\nCloses #{}.",
        issue,
        scope.join(" "),
        super::conflict::pr_section(resolutions),
        issue
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pr_body_carries_the_closing_keyword() {
        let body = pr_body(3246, &["-p".to_string(), "autospec-core".to_string()], &[]);
        assert!(
            body.contains("Closes #3246."),
            "the tracker closes on the body keyword, never on a title: {body}"
        );
        assert!(body.contains("Gate scope: -p autospec-core"), "{body}");
    }

    #[test]
    fn the_pr_body_keeps_the_conflict_section_and_the_keyword() {
        // Even with an auto-resolution section, the closing keyword is
        // present — the section is review information, the keyword is the
        // close.
        let body = pr_body(4291, &["--workspace".to_string()], &[]);
        assert!(body.ends_with("Closes #4291."), "{body}");
    }

    #[test]
    fn archive_moves_the_patch_under_superseded() {
        let dir =
            std::env::temp_dir().join(format!("convert-delivered-archive-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let patch = dir.join("changes.patch");
        fs::write(&patch, "diff --git a/x b/x\n").unwrap();
        assert!(archive_patch(&patch), "the patch leaves disk");
        assert!(!patch.exists(), "the original is gone, not copied");
        let moved = fs::read_dir(dir.join("superseded"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(moved.len(), 1, "one archived file");
        assert!(
            moved[0].starts_with("changes-") && moved[0].ends_with(".patch"),
            "the archived name keeps the patch identity: {moved:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
