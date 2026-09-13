//! The conversion pass's language gate (issue #4559): the CLI surface for
//! holding patches the Rust gate cannot evaluate. The verdict is decided in
//! `autospec_core::patch_language` before any branch exists — a gate that
//! cannot fail a patch must not be reported as passing it — and this module
//! renders the holds, archives them on `--apply`, and keeps the pass's
//! counters honest.
//!
//! This module exists as a child of the convert module rather than as
//! additions to `convert.rs`: the pass's command file is already past the
//! size ratchet's threshold, and an oversized file may be edited and
//! shrunk but not grown.

use std::fs;

use autospec_core::conversion_pass::{select_fresh, LanguageHold, PassOutcome, PatchCandidate};
use autospec_core::patch_language::{self, PatchLanguage};
use autospec_core::unfed_pass::PassCounters;
use serde_json::{json, Value};

use super::{patch_files, ConvertPlan, PatchLocation};

/// The patch's language class (issue #4559), decided before any branch
/// exists: the Rust gate cannot fail on a shell-only or a neither patch, so
/// a green gate there would mean the gate did not read the patch. An
/// unreadable patch yields an empty file list, which classifies as neither
/// and is held — fail-closed.
pub(crate) fn candidate_language(patch: &PatchLocation) -> PatchLanguage {
    let text = fs::read_to_string(&patch.path).unwrap_or_default();
    patch_language::classify(&patch_files(&text))
}

/// The plan-mode outcome: a pass that examined patches and offered none
/// must not print the idle `converted=0 held=0 skipped=0` line (issue
/// #4559, the skip-counting bug found while testing the language gate) —
/// the skips are the disqualified plus the language-held patches.
pub(crate) fn plan_outcome(candidates: &[PatchCandidate]) -> PassOutcome {
    let selection = select_fresh(candidates);
    PassOutcome::Examined(PassCounters {
        examined: candidates.len(),
        converted: 0,
        held: 0,
        skipped: selection.disqualified.len() + selection.language_held.len(),
    })
}

/// The `--json` array for the language-held patches: the reason names the
/// deciding files, so an operator can triage the hold without the patch.
pub(crate) fn held_json(plan: &ConvertPlan, holds: &[LanguageHold]) -> Vec<Value> {
    holds
        .iter()
        .map(|hold| {
            json!({
                "issue": hold.candidate.issue,
                "patch_key": hold.candidate.patch_key,
                "language": hold.language.as_str(),
                "reason": language_hold_reason(plan, hold),
            })
        })
        .collect()
}

/// The plan-mode `HOLD` lines, one per language-held patch.
pub(crate) fn render_holds(plan: &ConvertPlan, holds: &[LanguageHold]) {
    for hold in holds {
        // Flushed like every other decision line: a hold is a verdict, and a
        // verdict must not sit in the block buffer (#4572).
        super::progress::report_line(&format!(
            "  HOLD  #{issue} ({reason}) {patch_key}",
            issue = hold.candidate.issue,
            reason = language_hold_reason(plan, hold),
            patch_key = hold.candidate.patch_key
        ));
    }
}

/// The hold reason for a language-held candidate, from the files of the
/// patch on disk: the reason names the deciding files (the shell files for
/// a mixed hold — the prompt signal — or the whole list for a neither
/// hold), and those live with the patch.
pub(crate) fn language_hold_reason(plan: &ConvertPlan, hold: &LanguageHold) -> String {
    let files = plan
        .examined
        .iter()
        .find(|p| p.issue == hold.candidate.issue)
        .map(|p| patch_files(&fs::read_to_string(&p.path).unwrap_or_default()))
        .unwrap_or_default();
    patch_language::hold_reason(hold.language, &files)
}

/// Archive the language-held patches, folding their counts into the run's
/// counters and the run's archive count (issue #4559). They are terminal,
/// not a re-gate queue: they will never convert, so archiving them frees
/// their queue entries and they are never re-offered — the archive
/// directory, not a HELD ledger line, is the record (a ledger line would be
/// re-offered when the base next moves).
pub(crate) fn archive_held(
    plan: &ConvertPlan,
    holds: &[LanguageHold],
    counters: &mut PassCounters,
    archived: &mut usize,
) {
    for hold in holds {
        if let Some(patch) = plan
            .examined
            .iter()
            .find(|p| p.issue == hold.candidate.issue)
        {
            // The reason is computed while the patch is still on disk: it
            // names the deciding files, which leave the buffer with the move.
            let reason = language_hold_reason(plan, hold);
            archive_language_held(patch);
            counters.skipped += 1;
            *archived += 1;
            println!(
                "  ARCHIVED #{issue} (language: {reason})",
                issue = patch.issue,
            );
        }
    }
}

/// Move a language-held patch into its issue dir's `language-held/`
/// archive — terminal, unlike the `superseded/` queue (issue #4559). Once
/// moved, the file no longer matches the enumeration path
/// (`out/issue-*/changes.patch`), so the pass never sees it again: these
/// patches never convert, and their queue entry is freed with the move.
fn archive_language_held(patch: &PatchLocation) {
    let issue_dir = match patch.path.parent() {
        Some(dir) => dir,
        None => return,
    };
    let _ = fs::create_dir_all(issue_dir.join("language-held"));
    let archive = autospec_core::stored_output::language_held_archive_path(
        issue_dir,
        patch
            .path
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    let _ = fs::rename(&patch.path, &archive);
}
