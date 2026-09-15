//! The candidate list is `(base_sha, entries)`, not `entries` (issue #4512).
//!
//! A classification is a fact about the base it was derived against, and it
//! expires silently when trunk moves: a patch measured `fresh` at the start
//! of a pass can be already-delivered by the time the pass acts on it, if a
//! merge landed in between. The pass therefore (1) records the base its
//! classification was derived against and reports it, and (2) revalidates the
//! classification against the current base immediately before mutating — a
//! stale entry produces a reported reclassification (`STALE` line), never a
//! silent act on the old state. This is the same discipline
//! `hold_memo::HoldRecord` applies to holds: the hold and the classification
//! are both facts about a pair.

use std::collections::BTreeSet;
use std::process::Command;

use autospec_core::conversion_pass::{select_fresh, Attempt, PatchCandidate};
use autospec_core::hold_memo::HoldRecord;

use super::delivered;
use super::git::run_git_capture;
use super::language;
use super::{base_changed_files, attempt_state, prefetch_attempt_index, PatchLocation, ConvertPlan};
use autospec_core::hold_memo::re_gate;

/// Whether the issue is closed, asked of the tracker (issue #4626).
///
/// `None` when the state could not be read — no repo, no `gh`, or a failed
/// call — and the caller treats the candidate as not closed: a hold is a
/// claim that work is pending, and a claim is only lifted by the fact, never
/// by the absence of one (unknown never authorises acting).
///
/// Only the two states the tracker has are acted on; anything else is
/// `None`.
pub(super) fn issue_is_closed(repo: Option<&str>, issue: u64) -> Option<bool> {
    let repo = repo?;
    let output = Command::new("gh")
        .args([
            "api",
            &format!("repos/{repo}/issues/{issue}"),
            "--jq",
            ".state",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&output.stdout).trim() {
        "closed" => Some(true),
        "open" => Some(false),
        _ => None,
    }
}

/// The base the classification was derived against: the tip of `base_ref`.
/// `None` when the ref cannot be resolved (the caller reports what it can).
pub(super) fn classify_base(base_ref: &str) -> Option<String> {
    run_git_capture(&["rev-parse", base_ref]).ok()
}

/// The base recorded on the plan, for reporting: `origin/<base>#<sha8>`.
pub(super) fn base_label(opts_base: &str, base_ref: &str, classified_base: Option<&str>) -> String {
    match classified_base {
        Some(sha) => format!("{base_ref}#{}", &sha[..sha.len().min(8)]),
        None => format!("{base_ref} (unresolved)"),
    }
}

/// One candidate whose state changed when the base moved between the plan's
/// classification and the revalidation.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Flip {
    pub issue: u64,
    pub from: &'static str,
    pub to: &'static str,
}

fn state_name(attempt: &Attempt, held_recorded: bool, delivered: bool) -> &'static str {
    if delivered {
        "delivered"
    } else if held_recorded {
        "held"
    } else {
        match attempt {
            Attempt::Fresh => "fresh",
            Attempt::Interrupted => "interrupted",
            Attempt::Live => "attempted",
            Attempt::Unknown => "attempted",
        }
    }
}

/// The flips between two classifications of the same patches, in issue order.
/// A candidate present on only one side is a flip to/from `absent` — the
/// enumeration itself changed under the pass.
pub(super) fn flips(old: &[PatchCandidate], new: &[PatchCandidate]) -> Vec<Flip> {
    let mut out = Vec::new();
    for (o, n) in old.iter().zip(new.iter()) {
        if o.issue != n.issue {
            continue;
        }
        let from = state_name(&o.attempt, o.held_recorded, o.delivered);
        let to = state_name(&n.attempt, n.held_recorded, n.delivered);
        if from != to {
            out.push(Flip { issue: o.issue, from, to });
        }
    }
    for c in old.iter().chain(new.iter()) {
        let in_old = old.iter().any(|o| o.issue == c.issue);
        let in_new = new.iter().any(|o| o.issue == c.issue);
        if in_old != in_new {
            out.push(Flip {
                issue: c.issue,
                from: if in_old { "present" } else { "absent" },
                to: if in_new { "present" } else { "absent" },
            });
        }
    }
    out.sort_by_key(|f| f.issue);
    out
}

/// Classify the examined patches against the current tip of `base_ref`:
/// attempt state (branch liveness), recorded holds (re-gated against the
/// base), and the already-delivered residue. The caller has already fetched.
pub(super) fn classify(
    examined: &[PatchLocation],
    repo: Option<&str>,
    base_ref: &str,
    branch_prefix: &str,
    held: &std::collections::BTreeMap<u64, HoldRecord>,
) -> Vec<PatchCandidate> {
    let patches: Vec<_> = examined.iter().map(|p| (p.issue, p.path.clone())).collect();
    let delivered_issues: BTreeSet<u64> = repo
        .map(|_| delivered::detect_delivered(base_ref, &patches))
        .unwrap_or_default();
    let attempt_index = repo
        .and_then(|repo| prefetch_attempt_index(repo, branch_prefix));

    let mut candidates = Vec::new();
    for patch in examined {
        let branch = format!("{branch_prefix}{}", patch.issue);
        let attempt = attempt_state(repo, &branch, attempt_index.as_ref());
        let held_recorded = match held.get(&patch.issue) {
            Some(record) => {
                let changed = base_changed_files(&record.base_sha, base_ref)
                    .unwrap_or_else(|| vec!["<base: unknown — over-re-gate>".to_string()]);
                re_gate(record, &patch.patch_key, &changed).is_still_held()
            }
            None => false,
        };
        // Issue state is checked before the patch is selected (#4626): a
        // hold on a closed issue is a claim of pending work that does not
        // exist, and re-gating it forever is the measured waste. Asked only
        // for candidates with a recorded hold — the ledger is small, the
        // fresh backlog is not, and a fresh patch for a closed issue comes
        // from a closed queue entry, which the dispatch side evicts.
        let closed = held_recorded
            .then(|| issue_is_closed(repo, patch.issue))
            .flatten()
            .unwrap_or(false);
        candidates.push(PatchCandidate {
            issue: patch.issue,
            patch_key: patch.patch_key.clone(),
            attempt,
            held_recorded,
            delivered: delivered_issues.contains(&patch.issue),
            closed,
            language: language::candidate_language(patch),
        });
    }
    candidates
}

/// Revalidate the plan's classification against the current base.
///
/// Base unchanged: the plan's candidates, and no flips. Base moved: the
/// classification is recomputed and the changes returned as flips — the
/// caller reports them and acts on the new candidates. The base cannot be
/// resolved: fail closed — keep the plan's classification and report the
/// gap, so a missing ref never silently authorises acting on a stale state.
/// The revalidation decision: has the base moved since the classification?
///
/// - recorded, now different: moved.
/// - never recorded, now resolvable: the base is known to have changed
///   under the pass — revalidate.
/// - the current base cannot be resolved: fail closed — keep the recorded
///   state and report the gap; a missing ref never authorises acting on a
///   reclassification the pass could not compute.
pub(super) fn base_moved(classified: Option<&str>, current: Option<&str>) -> bool {
    match (classified, current) {
        (Some(old), Some(new)) => old != new,
        (None, Some(_)) => true,
        (_, None) => false,
    }
}

pub(super) fn revalidate(
    plan: &ConvertPlan,
    base_ref: &str,
) -> (Vec<PatchCandidate>, Vec<Flip>, Option<String>) {
    let current = classify_base(base_ref);
    let current_sha = current.clone();
    if !base_moved(plan.classified_base.as_deref(), current.as_deref()) {
        return (plan.candidates.clone(), Vec::new(), current_sha);
    }
    let repo = plan.opts.repo.clone().or_else(super::infer_repo);
    let fresh = classify(
        &plan.examined,
        repo.as_deref(),
        base_ref,
        &plan.opts.branch_prefix,
        &plan.held,
    );
    let flipped = flips(&plan.candidates, &fresh);
    (fresh, flipped, current_sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(issue: u64, attempt: Attempt, held: bool, delivered: bool) -> PatchCandidate {
        PatchCandidate {
            issue,
            patch_key: format!("pk{issue}"),
            attempt,
            held_recorded: held,
            delivered,
            closed: false,
            language: Default::default(),
        }
    }

    #[test]
    fn no_change_no_flips() {
        let a = vec![cand(1, Attempt::Fresh, false, false)];
        let b = vec![cand(1, Attempt::Fresh, false, false)];
        assert!(flips(&a, &b).is_empty());
    }

    #[test]
    fn a_moved_base_flips_fresh_to_delivered() {
        let a = vec![cand(1, Attempt::Fresh, false, false)];
        let b = vec![cand(1, Attempt::Fresh, false, true)];
        assert_eq!(
            flips(&a, &b),
            vec![Flip { issue: 1, from: "fresh", to: "delivered" }]
        );
    }

    #[test]
    fn a_moved_base_can_unflip_delivered_back_to_fresh() {
        // The issue's own scenario in the other direction: classified
        // already-in-main, then a merge touched the files and the patch no
        // longer applies — the state must be re-measured, not assumed.
        let a = vec![cand(1, Attempt::Fresh, false, true)];
        let b = vec![cand(1, Attempt::Fresh, false, false)];
        assert_eq!(
            flips(&a, &b),
            vec![Flip { issue: 1, from: "delivered", to: "fresh" }]
        );
    }

    #[test]
    fn state_names_rank_delivered_over_held_over_attempt() {
        assert_eq!(state_name(&Attempt::Live, true, true), "delivered");
        assert_eq!(state_name(&Attempt::Live, true, false), "held");
        assert_eq!(state_name(&Attempt::Live, false, false), "attempted");
        assert_eq!(state_name(&Attempt::Unknown, false, false), "attempted");
        assert_eq!(state_name(&Attempt::Interrupted, false, false), "interrupted");
        assert_eq!(state_name(&Attempt::Fresh, false, false), "fresh");
    }

    #[test]
    #[test]
    fn the_move_decision_fails_closed_on_an_unresolvable_base() {
        assert!(base_moved(Some("a"), Some("b")));
        assert!(!base_moved(Some("a"), Some("a")));
        assert!(base_moved(None, Some("a")));
        assert!(!base_moved(Some("a"), None));
        assert!(!base_moved(None, None));
    }

    fn base_labels_truncate_to_eight() {
        assert_eq!(
            base_label("main", "origin/main", Some("0123456789abcdef")),
            "origin/main#01234567"
        );
        assert_eq!(
            base_label("main", "origin/main", None),
            "origin/main (unresolved)"
        );
    }

    #[test]
    fn selection_of_revalidated_candidates_uses_the_new_states() {
        // The point of revalidating is that the downstream selection — what
        // the pass offers, archives, and gates — follows the new states.
        let old = vec![cand(1, Attempt::Fresh, false, false)];
        let new = vec![cand(1, Attempt::Fresh, false, true)];
        let selection = select_fresh(&new);
        assert_eq!(selection.delivered.len(), 1);
        assert!(selection.fresh.is_empty());
        let _ = &old;
    }
}
