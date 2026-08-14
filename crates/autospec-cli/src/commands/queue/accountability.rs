use super::*;

pub(super) fn is_accountability_issue(issue: &RemoteIssue) -> bool {
    issue_has_label(issue, "autospec:run-accountability")
}

pub(super) fn reviewable_issue(issue: &RemoteIssue) -> bool {
    !issue.closed
        && issue_has_label(issue, "auto-implement")
        && !issue_has_label(issue, "needs-classify")
        && !issue_has_label(issue, "autospec:needs-human")
        && !issue_has_label(issue, "security:quarantined")
        && !is_accountability_issue(issue)
}
