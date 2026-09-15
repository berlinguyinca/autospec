//! The terminal disposition of an unconvertible patch (issue #4637).
//!
//! A produced patch suppresses re-dispatch of its issue for as long as it
//! sits on disk: topup's "already produced" test is the patch's presence.
//! That is right while the patch is convertible — re-running a finished
//! issue wastes GPU. It is wrong when the patch is conflict-bound in a
//! shape the pass will never merge: then the file is evidence of work
//! against a base that no longer exists, and it holds its issue hostage
//! forever — 193 queue entries, 27 idle agent slots, measured.
//!
//! The invariant: **a produced patch is only evidence of completed work
//! while it is still convertible against the current base.** Once the pass
//! has proved it is not — every conflicted file a shape it refuses or
//! regenerates — the patch gets a terminal disposition: the disposition is
//! recorded where the patch lived, the patch is archived (never deleted),
//! and the issue re-enters dispatch, where an agent regenerates the patch
//! against current main.

use std::fs;

use super::{ApplyResult, ConvertPlan, PatchLocation};
use crate::commands::convert::progress;

/// The disposition file that stands in for an invalidated patch, in the
/// issue's own directory. It is the audit half of the disposition: the
/// archived patch says *what* the work was, this file says *why it left*
/// and *against which base the verdict was made* — the verdict is a fact
/// about a base, and it expires the same way a classification does (#4512).
fn record_disposition(patch_path: &std::path::Path, base_sha: &str, reason: &str) {
    let Some(dir) = patch_path.parent() else {
        return;
    };
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let body = format!(
        "status: invalidated\nreason: {reason}\nbase: {base_sha}\nat: {at}\n"
    );
    let _ = fs::write(dir.join("disposition.txt"), body);
}

/// The terminal disposition: record, report, archive.
///
/// Archiving is the mechanism that releases the issue: the patch leaves
/// the path topup reads, so the next pass re-dispatches the issue instead
/// of skipping it, and the work is regenerated against the current base.
/// The disposition is written *before* the move, so a run killed between
/// the two still leaves the reason at the old path.
pub(super) fn dispose(
    plan: &ConvertPlan,
    base_sha: &str,
    patch: &PatchLocation,
    reason: &str,
) -> ApplyResult {
    record_disposition(&patch.path, base_sha, reason);
    progress::invalidated(patch.issue, reason);
    super::delivered::archive_patch(&patch.path);
    ApplyResult::Invalidated
}

/// A conflict the pass could not resolve, settled: a structural refusal is
/// terminal — the patch is work against a base that no longer exists, and on
/// disk it suppresses re-dispatch forever, so it is invalidated, not held.
/// Anything the pass could not prove structural is an ordinary hold, the
/// patch untouched, re-offered next pass. The worktree is read (to decide
/// structural) and then torn down either way.
pub(super) fn settle_unresolved(
    plan: &ConvertPlan,
    base_sha: &str,
    patch: &PatchLocation,
    worktree: &std::path::Path,
    reason: &str,
) -> ApplyResult {
    let terminal = super::conflict::is_structural_refusal(worktree);
    super::git::teardown_worktree(worktree);
    if terminal {
        dispose(plan, base_sha, patch, reason)
    } else {
        super::record_held_and_result(plan, base_sha, patch, reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "convert-invalidated-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("issue dir");
        dir
    }

    #[test]
    fn the_disposition_records_status_reason_and_base() {
        let dir = issue_dir("record");
        let patch = dir.join("changes.patch");
        fs::write(&patch, "diff --git a/x b/x\n").unwrap();
        let reason = "conflict (refusing auto-resolution): src/a.rs: shape unknown";
        record_disposition(&patch, "0123456789abcdef", reason);
        let body = fs::read_to_string(dir.join("disposition.txt")).unwrap();
        assert!(body.starts_with("status: invalidated\n"), "{body}");
        assert!(body.contains(&format!("reason: {reason}\n")), "{body}");
        assert!(body.contains("base: 0123456789abcdef\n"), "{body}");
        assert!(body.contains("at: "), "{body}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_later_disposition_overwrites_the_earlier_one() {
        // The file is the *latest* terminal event, not an append-only log:
        // a second invalidation against a newer base must not leave the
        // reader two contradictory verdicts side by side.
        let dir = issue_dir("overwrite");
        let patch = dir.join("changes.patch");
        fs::write(&patch, "diff --git a/x b/x\n").unwrap();
        record_disposition(&patch, "aaaaaaaaaaaaaaaa", "first");
        record_disposition(&patch, "bbbbbbbbbbbbbbbb", "second");
        let body = fs::read_to_string(dir.join("disposition.txt")).unwrap();
        assert!(!body.contains("first"), "{body}");
        assert!(body.contains("base: bbbbbbbbbbbbbbbb\n"), "{body}");
        let _ = fs::remove_dir_all(&dir);
    }
}
