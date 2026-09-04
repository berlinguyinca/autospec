use super::*;

pub(super) fn is_accountability_issue(issue: &RemoteIssue) -> bool {
    issue_has_label(issue, "autospec:run-accountability")
}

pub(super) fn reviewable_issue(issue: &RemoteIssue) -> bool {
    reviewable_issue_with_recheck(issue, false)
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
pub(super) fn reviewable_issue_with_recheck(issue: &RemoteIssue, recheck: bool) -> bool {
    !issue.closed
        && issue_has_label(issue, "auto-implement")
        && !issue_has_label(issue, "needs-classify")
        && !issue_has_label(issue, "autospec:needs-human")
        && (recheck || !issue_has_label(issue, "security:quarantined"))
        && !is_accountability_issue(issue)
}
