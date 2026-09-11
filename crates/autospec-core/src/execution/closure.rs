//! Conversion-PR closure semantics and tracker reconciliation (#4044).
//!
//! GitHub only closes an issue when the PR that merges carries a closing
//! keyword — `Closes`, `Fixes`, or `Resolves`. `Refs` does nothing. The
//! conversion pass used to end every PR body with `Refs #N`, which left 51
//! delivered issues open on the tracker; `refresh-queue.sh` had to exclude
//! them by hand, so "issues open" and "work remaining" quietly diverged.
//!
//! Invariants:
//! 1. **The conversion pass writes a closing keyword.**
//!    [`conversion_pr_body`] ends with `Closes #N`; a test asserts
//!    [`closure_authorized`] on the generated body for every issue number.
//! 2. **Not closing is a stated decision, not an accident.** A partial-fix
//!    PR carries the explicit marker from [`partial_fix_pr_body`]
//!    (`Refs #N (does not close it)`); a held-for-review PR carries the
//!    marker from [`held_for_review_pr_body`] (`Refs #N (NOT closing: held
//!    for supervisor review; ...)`); a bare `Refs #N` is indistinguishable
//!    from a missing `Closes` and is reported.
//! 3. **Reconciliation reports, it does not absorb.**
//!    [`reconcile_tracker`] names every open issue a merged PR delivers and
//!    every merged PR whose issue is open.
//! 4. **Counts feed decisions only after reconciliation.**
//!    [`reconciled_open_count`] — not the raw tracker count — is the number
//!    capacity/throughput metrics may use, so a tracking lag cannot be
//!    misread as a backlog trend.

use crate::evidence_fidelity::closure_authorized;
use crate::execution::backlog::{PrRecord, PrState};
use std::collections::BTreeSet;

/// The body the conversion pass generates for a PR that fully implements
/// the issue: the summary plus a closing trailer. Only
/// Closes/Fixes/Resolves make GitHub close the issue on merge (#4044).
pub fn conversion_pr_body(issue_number: u64, summary: &str) -> String {
    format!("{summary}\n\nCloses #{issue_number}")
}

/// The exact phrase a PR uses to state that it deliberately does not close
/// its issue (a partial fix). `Refs` alone cannot say this: with no
/// closing keyword and no marker, the body reads as a defect (#4044).
pub const PARTIAL_FIX_MARKER: &str = "does not close it";

/// The body the conversion pass generates for a PR that only partially
/// addresses the issue: a `Refs` trailer that explicitly states the PR does
/// not close the issue.
pub fn partial_fix_pr_body(issue_number: u64, summary: &str) -> String {
    format!("{summary}\n\nRefs #{issue_number} ({PARTIAL_FIX_MARKER})")
}

/// The exact phrase a PR uses to state that it is held for supervisor
/// review: the patch converts but the issue stays open until the
/// acceptance criteria are verified.
pub const HELD_FOR_REVIEW_MARKER: &str =
    "NOT closing: held for supervisor review; close it only once the acceptance criteria are verified";

/// The body the conversion pass generates for a patch held for supervisor
/// review: a `Refs` trailer that explicitly states the issue stays open.
pub fn held_for_review_pr_body(issue_number: u64, summary: &str) -> String {
    format!("{summary}\n\nRefs #{issue_number} ({HELD_FOR_REVIEW_MARKER})")
}

/// Whether `text` carries the explicit non-closure marker for
/// `issue_number`: a `Refs #N` line stating the PR does not close the
/// issue. Case-insensitive on the keyword and the marker phrase.
pub fn has_partial_fix_marker(text: &str, issue_number: u64) -> bool {
    has_refs_marker(text, issue_number, PARTIAL_FIX_MARKER)
}

/// Whether `text` carries the explicit held-for-review marker for
/// `issue_number`: a `Refs #N` line stating the PR is held for supervisor
/// review. Case-insensitive on the keyword and the marker phrase.
pub fn has_held_for_review_marker(text: &str, issue_number: u64) -> bool {
    has_refs_marker(text, issue_number, HELD_FOR_REVIEW_MARKER)
}

/// Shared detection for `Refs #N (marker)` lines. Case-insensitive on the
/// keyword and the marker phrase.
fn has_refs_marker(text: &str, issue_number: u64, marker: &str) -> bool {
    let reference = format!("#{issue_number}");
    for line in text.lines() {
        let line = line.trim_start();
        if line.len() < 4 || !line[..4].eq_ignore_ascii_case("refs") {
            continue;
        }
        // "Refs" must be a whole word: a space (or end of line) follows.
        if line[4..].chars().next().is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let rest = line[4..].trim_start();
        let Some(after_ref) = rest.strip_prefix(reference.as_str()) else {
            continue;
        };
        let after_ref = after_ref.trim_start();
        let Some(inner) = after_ref
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
        else {
            continue;
        };
        if inner.eq_ignore_ascii_case(marker) {
            return true;
        }
    }
    false
}

/// One mismatch between the tracker and the merged work (#4044).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerDiscrepancy {
    /// The issue the mismatch is about.
    pub issue: u64,
    /// The merged PR's head branch (the report's PR identifier).
    pub pr_branch: String,
    /// Which mismatch this is.
    pub kind: TrackerDiscrepancyKind,
}

/// The two ways a merged PR and its issue can disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerDiscrepancyKind {
    /// The issue is open although a merged PR closes it: the tracker and
    /// the queue disagree about whether the issue is done.
    DeliveredButOpen,
    /// A merged PR references the issue with no closing keyword and no
    /// explicit non-closure marker (partial-fix or held-for-review): a
    /// bare `Refs` where a `Closes` was meant, indistinguishable from the
    /// #4044 defect.
    NoClosureDecision,
}

impl TrackerDiscrepancy {
    /// The action line the report renders for this mismatch.
    pub fn line(&self) -> String {
        match self.kind {
            TrackerDiscrepancyKind::DeliveredButOpen => format!(
                "issue #{} is open but merged PR {} delivers it (closing keyword present): reconcile the issue state",
                self.issue, self.pr_branch
            ),
            TrackerDiscrepancyKind::NoClosureDecision => format!(
                "issue #{} is open and merged PR {} references it with no closing keyword and no non-closure marker: state the decision (Closes, partial-fix, or held-for-review marker)",
                self.issue, self.pr_branch
            ),
        }
    }
}

/// Reconcile the tracker against the merged PRs.
///
/// For every **merged** PR with an authoritative issue that the tracker
/// still lists as open, exactly one of:
/// - the body carries a closing keyword for the issue →
///   [`TrackerDiscrepancyKind::DeliveredButOpen`] (the tracker should have
///   closed it on merge);
/// - the body carries an explicit non-closure marker (partial-fix or
///   held-for-review) → no discrepancy (not closing is a stated decision);
/// - otherwise → [`TrackerDiscrepancyKind::NoClosureDecision`].
///
/// Open and closed-unmerged PRs are work in progress or abandoned work,
/// not tracker lag, and are never reported.
pub fn reconcile_tracker(open_issues: &BTreeSet<u64>, prs: &[PrRecord]) -> Vec<TrackerDiscrepancy> {
    let mut found: Vec<TrackerDiscrepancy> = prs
        .iter()
        .filter(|pr| pr.state == PrState::Merged)
        .filter_map(|pr| {
            let issue = pr.authoritative_issue()?;
            if !open_issues.contains(&issue) {
                return None;
            }
            let kind = if closure_authorized(&pr.description, issue).is_authorized() {
                TrackerDiscrepancyKind::DeliveredButOpen
            } else if has_partial_fix_marker(&pr.description, issue)
                || has_held_for_review_marker(&pr.description, issue)
            {
                return None;
            } else {
                TrackerDiscrepancyKind::NoClosureDecision
            };
            Some(TrackerDiscrepancy {
                issue,
                pr_branch: pr.head_branch.clone(),
                kind,
            })
        })
        .collect();
    found.sort_by(|a, b| (a.issue, &a.pr_branch).cmp(&(b.issue, &b.pr_branch)));
    found
}

/// The open-issue count capacity/throughput metrics may use: the tracker
/// count minus the issues a merged PR has already delivered. Feeding the
/// raw tracker count into a trend is how a tracking lag looks like a
/// backlog (#4044).
pub fn reconciled_open_count(
    open_issues: &BTreeSet<u64>,
    discrepancies: &[TrackerDiscrepancy],
) -> usize {
    let delivered: BTreeSet<u64> = discrepancies
        .iter()
        .filter(|d| d.kind == TrackerDiscrepancyKind::DeliveredButOpen)
        .map(|d| d.issue)
        .collect();
    open_issues.difference(&delivered).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence_fidelity::ClosureVerdict;

    fn merged(branch: &str, linked: Option<u64>, body: &str) -> PrRecord {
        PrRecord {
            head_branch: branch.to_string(),
            linked_issue: linked,
            description: body.to_string(),
            state: PrState::Merged,
        }
    }

    fn open_pr(branch: &str, linked: Option<u64>, body: &str) -> PrRecord {
        PrRecord {
            head_branch: branch.to_string(),
            linked_issue: linked,
            description: body.to_string(),
            state: PrState::Open,
        }
    }

    fn set(values: &[u64]) -> BTreeSet<u64> {
        values.iter().copied().collect()
    }

    // ── AC1: every generated PR body carries a closing keyword ─────────

    #[test]
    fn generated_pr_body_closes_its_issue() {
        for issue in [7u64, 50, 3888, 4044, 123456] {
            let body = conversion_pr_body(issue, "converted the patch");
            assert!(body.contains(&format!("Closes #{issue}")), "{body:?}");
            assert_eq!(
                closure_authorized(&body, issue),
                ClosureVerdict::Authorized { verb: "closes" },
                "generated body for #{issue} must authorize closure: {body:?}"
            );
            // The trailer is one the backlog's authoritative parsing finds.
            assert_eq!(
                crate::execution::backlog::issue_number_from_trailer(&body),
                Some(issue)
            );
        }
    }

    // ── AC4: not closing is an explicit, detectable decision ───────────

    #[test]
    fn partial_fix_body_states_its_decision_and_does_not_authorize() {
        let body = partial_fix_pr_body(4044, "partial sweep");
        assert!(body.contains("Refs #4044"), "{body:?}");
        assert!(body.contains(PARTIAL_FIX_MARKER), "{body:?}");
        // A partial fix does NOT close: the keyword is deliberately absent.
        assert_eq!(
            closure_authorized(&body, 4044),
            ClosureVerdict::NotAuthorized
        );
        assert!(has_partial_fix_marker(&body, 4044));
    }

    #[test]
    fn marker_detection_requires_refs_the_issue_and_the_phrase() {
        assert!(has_partial_fix_marker("Refs #50 (does not close it)", 50));
        assert!(has_partial_fix_marker("refs #50 (DOES NOT CLOSE IT)", 50));
        assert!(has_partial_fix_marker(
            "part one\n\nRefs #50 (does not close it)",
            50
        ));
        // A bare Refs is the defect, not the decision.
        assert!(!has_partial_fix_marker("Refs #50", 50));
        // The marker is for the specific issue it references.
        assert!(!has_partial_fix_marker("Refs #5 (does not close it)", 50));
        assert!(!has_partial_fix_marker("Refs #50 (partial sweep)", 50));
        // "Refs" must be a whole word.
        assert!(!has_partial_fix_marker(
            "Refunds #50 (does not close it)",
            50
        ));
    }

    // ── AC2: reconciliation reports both directions of the mismatch ────

    #[test]
    fn delivered_but_open_issue_is_reported() {
        let found = reconcile_tracker(
            &set(&[3870]),
            &[merged("conv/3870-fix", None, "x\n\nCloses #3870")],
        );
        assert_eq!(
            found,
            vec![TrackerDiscrepancy {
                issue: 3870,
                pr_branch: "conv/3870-fix".to_string(),
                kind: TrackerDiscrepancyKind::DeliveredButOpen,
            }]
        );
        // The line names both sides of the mismatch: issue and PR.
        let line = found[0].line();
        assert!(line.contains("#3870"), "{line}");
        assert!(line.contains("conv/3870-fix"), "{line}");
    }

    #[test]
    fn merged_pr_with_open_issue_and_no_decision_is_reported() {
        // The exact #4044 signature: merged, `Refs #N`, no marker, issue open.
        let found = reconcile_tracker(
            &set(&[3888]),
            &[merged("conv/3888-retry-2", None, "Refs #3888")],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, TrackerDiscrepancyKind::NoClosureDecision);
        assert_eq!(found[0].issue, 3888);
        assert!(
            found[0].line().contains("conv/3888-retry-2"),
            "{:?}",
            found[0].line()
        );
    }

    #[test]
    fn linked_only_merged_pr_without_keyword_is_reported() {
        // A platform link is not a closing keyword: GitHub would not close
        // on it, so an open issue behind it is a missing decision.
        let found = reconcile_tracker(
            &set(&[3849]),
            &[merged("conv/3849-sweep", Some(3849), "no trailer")],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].kind, TrackerDiscrepancyKind::NoClosureDecision);
    }

    #[test]
    fn explicit_partial_fix_is_not_reported() {
        // AC5: no keyword but the explicit marker — a documented decision.
        let found = reconcile_tracker(
            &set(&[4101]),
            &[merged(
                "conv/4101-partial",
                None,
                "first slice\n\nRefs #4101 (does not close it)",
            )],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn non_merged_prs_and_closed_issues_are_never_reported() {
        let open = set(&[3905]); // 4200 is already closed in the tracker
        let found = reconcile_tracker(
            &open,
            &[
                // Open PR: work in progress, not tracker lag.
                open_pr("conv/3905-fix", Some(3905), "Closes #3905"),
                // Merged, but its issue is already closed: no mismatch.
                merged("conv/4200-fix", None, "Closes #4200"),
            ],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    // ── AC3: counts are computed after reconciliation ──────────────────

    #[test]
    fn reconciled_count_subtracts_only_delivered_issues() {
        let open = set(&[1, 2, 3, 4]);
        let discrepancies = vec![
            TrackerDiscrepancy {
                issue: 1,
                pr_branch: "a".into(),
                kind: TrackerDiscrepancyKind::DeliveredButOpen,
            },
            TrackerDiscrepancy {
                issue: 2,
                pr_branch: "b".into(),
                kind: TrackerDiscrepancyKind::NoClosureDecision,
            },
        ];
        // Only the delivered issue drops out; a no-decision issue still
        // needs work and stays in the count.
        assert_eq!(reconciled_open_count(&open, &discrepancies), 3);
        assert_eq!(reconciled_open_count(&open, &[]), 4);
    }

    // ── AC4b: held-for-review marker is an explicit, detectable decision ─

    #[test]
    fn held_for_review_body_states_its_decision_and_does_not_authorize() {
        let body = held_for_review_pr_body(4286, "converted the patch");
        assert!(body.contains("Refs #4286"), "{body:?}");
        assert!(body.contains(HELD_FOR_REVIEW_MARKER), "{body:?}");
        // Held for review does NOT close: the keyword is deliberately absent.
        assert_eq!(
            closure_authorized(&body, 4286),
            ClosureVerdict::NotAuthorized
        );
        assert!(has_held_for_review_marker(&body, 4286));
    }

    #[test]
    fn held_marker_detection_requires_refs_the_issue_and_the_phrase() {
        assert!(has_held_for_review_marker(
            "Refs #50 (NOT closing: held for supervisor review; close it only once the acceptance criteria are verified)",
            50
        ));
        assert!(has_held_for_review_marker(
            "refs #50 (not closing: HELD FOR SUPERVISOR REVIEW; CLOSE IT ONLY ONCE THE ACCEPTANCE CRITERIA ARE VERIFIED)",
            50
        ));
        // A bare Refs is the defect, not the decision.
        assert!(!has_held_for_review_marker("Refs #50", 50));
        // The marker is for the specific issue it references.
        assert!(!has_held_for_review_marker(
            "Refs #5 (NOT closing: held for supervisor review; close it only once the acceptance criteria are verified)",
            50
        ));
        // "Refs" must be a whole word.
        assert!(!has_held_for_review_marker(
            "Refunds #50 (NOT closing: held for supervisor review; close it only once the acceptance criteria are verified)",
            50
        ));
    }

    #[test]
    fn explicit_held_for_review_is_not_reported() {
        // No keyword but the explicit held-for-review marker — a documented
        // decision.
        let found = reconcile_tracker(
            &set(&[4286]),
            &[merged(
                "conv/4286-held",
                None,
                "converted the patch\n\nRefs #4286 (NOT closing: held for supervisor review; close it only once the acceptance criteria are verified)",
            )],
        );
        assert!(found.is_empty(), "{found:?}");
    }
}
