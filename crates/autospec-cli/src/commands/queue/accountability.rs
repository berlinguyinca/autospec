use super::*;

pub(super) fn is_accountability_issue(issue: &RemoteIssue) -> bool {
    issue_has_label(issue, "autospec:run-accountability")
}

/// How far a recheck pass may reach.
///
/// `recheck` alone lifts `security:quarantined`. Lifting `autospec:needs-human`
/// additionally requires `targeted` — an explicit `--issue` — because that label
/// is overloaded: the safety reviewer writes it for an AMBIGUOUS verdict, but
/// the orchestrator also writes it when an implementer has failed repeatedly.
/// Only the first is a verdict this command can re-derive. Requiring the
/// operator to name the issue keeps a bulk sweep from clearing the second kind.
#[derive(Clone, Copy, Default)]
pub(super) struct RecheckScope {
    pub(super) recheck: bool,
    pub(super) targeted: bool,
}

impl RecheckScope {
    pub(super) fn new(recheck: bool, targeted: bool) -> Self {
        Self { recheck, targeted }
    }

    pub(super) fn lifts_needs_human(self) -> bool {
        self.recheck && self.targeted
    }

    /// The stale safety labels this scope is allowed to lift off `issue`.
    pub(super) fn liftable_labels(self, issue: &RemoteIssue) -> Vec<&'static str> {
        let mut labels = Vec::new();
        if self.recheck && issue_has_label(issue, "security:quarantined") {
            labels.push("security:quarantined");
        }
        if self.lifts_needs_human() && issue_has_label(issue, "autospec:needs-human") {
            labels.push("autospec:needs-human");
        }
        labels
    }
}

/// Whether a safety label on this issue is exactly what the recheck exists to
/// re-derive. The deterministic screen allowing the issue now is the evidence
/// that the label is stale; without treating that as a reason to review, the
/// label has no path back off.
pub(super) fn stale_safety_label(issue: &RemoteIssue, scope: RecheckScope) -> bool {
    !scope.liftable_labels(issue).is_empty()
}

pub(super) fn reviewable_issue(issue: &RemoteIssue) -> bool {
    reviewable_issue_with_recheck(issue, RecheckScope::default())
}

/// `recheck` admits `security:quarantined` issues so the reviewer can re-derive
/// a verdict for them; every other gate is unchanged.
///
/// Without it a quarantine is terminal. The reviewer skips quarantined issues,
/// so a quarantine applied by a classifier defect survives the fix to that
/// defect, and the only way out is a human editing the security label by hand —
/// exactly the action a guarded environment refuses. That left
/// InferWeave/inferweave #1, #2, #5, #10, #50 and #123 permanently blocked by a
/// `ci-or-review-bypass` false positive, and with #1 the sole transitive root,
/// the entire 123-issue queue with them.
///
/// Sticky by default is still the right posture: a quarantine should not lift
/// itself on the next routine sweep. Recheck makes lifting it an explicit,
/// audited act performed by the same typed reviewer that applied it, rather than
/// label surgery no verdict backs.
pub(super) fn reviewable_issue_with_recheck(issue: &RemoteIssue, scope: RecheckScope) -> bool {
    !issue.closed
        && issue_has_label(issue, "auto-implement")
        && !issue_has_label(issue, "needs-classify")
        && (scope.lifts_needs_human() || !issue_has_label(issue, "autospec:needs-human"))
        && (scope.recheck || !issue_has_label(issue, "security:quarantined"))
        && !is_accountability_issue(issue)
}
