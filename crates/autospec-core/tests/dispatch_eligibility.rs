use autospec_core::coordination::RemoteIssue;
use autospec_core::coordination::{
    evaluate_dispatch_eligibility, is_dispatch_eligible, reconcile, DispatchEligibilityPolicy,
    EligibilityVerdict, ReconcileError, ReconcileErrorKind, ReconcileInput,
    DISPATCH_ELIGIBILITY_LABEL,
};

/// The retired selector from the #3771 incident: the fleet's old
/// `llm-ready` vocabulary, replaced by `auto-implement`.
const RETIRED: &str = "llm-ready";

fn migrating_policy() -> DispatchEligibilityPolicy {
    DispatchEligibilityPolicy::new(DISPATCH_ELIGIBILITY_LABEL, vec![RETIRED.to_string()])
        .expect("auto-implement is not retired here")
}

fn open(number: u64, labels: &[&str]) -> RemoteIssue {
    RemoteIssue::open(
        number,
        format!("issue-{number}"),
        String::new(),
        labels.iter().map(|label| (*label).to_string()).collect(),
        "agent",
    )
}

fn input(tracker: Vec<RemoteIssue>, worklist: Vec<u64>) -> ReconcileInput {
    ReconcileInput {
        tracker_issues: tracker,
        worklist,
        policy: migrating_policy(),
    }
}

#[test]
fn eligibility_label_is_named_in_exactly_one_place() {
    // The dispatch path names the selector once; the constant is the source
    // of truth for what "dispatchable" means.
    assert_eq!(DISPATCH_ELIGIBILITY_LABEL, "auto-implement");
    let policy = DispatchEligibilityPolicy::default();
    assert_eq!(policy.current(), "auto-implement");
    assert!(policy.retired().is_empty());
}

#[test]
fn predicate_grants_eligibility_only_to_open_issues_with_the_current_label() {
    let policy = migrating_policy();

    assert!(is_dispatch_eligible(&open(1, &["auto-implement"]), &policy));
    // Migrated issue still carrying the retired label: eligible, not stranded.
    assert!(is_dispatch_eligible(
        &open(2, &["llm-ready", "auto-implement"]),
        &policy
    ));

    assert!(!is_dispatch_eligible(&open(3, &["llm-ready"]), &policy));
    assert!(!is_dispatch_eligible(
        &open(4, &["safety:reviewed"]),
        &policy
    ));
    assert!(!is_dispatch_eligible(
        &RemoteIssue::closed(5, "done", "", vec!["auto-implement".to_string()], "agent"),
        &policy
    ));

    assert_eq!(
        evaluate_dispatch_eligibility(&open(1, &["auto-implement"]), &policy),
        EligibilityVerdict::Eligible
    );
    assert_eq!(
        evaluate_dispatch_eligibility(&open(4, &["safety:reviewed"]), &policy),
        EligibilityVerdict::Ineligible
    );
}

#[test]
fn retired_only_issue_surfaces_as_error_not_silently_skipped() {
    // Regression for #3771: an issue carrying only the retired selector must
    // fail the reconcile loudly, listing its number — not vanish from the
    // worklist with no trace.
    let input = input(vec![open(3322, &[RETIRED])], Vec::new());

    let error = reconcile(&input).expect_err("retired-only issue must fail loudly");
    match &error.kind {
        ReconcileErrorKind::StrandedIssues { issues } => {
            assert_eq!(issues, &[3322]);
        }
        other => panic!("expected stranded issues, got {other:?}"),
    }
    let message = error.to_string();
    assert!(
        message.contains("#3322"),
        "error must list issue numbers: {message}"
    );
    assert!(message.contains("ERROR"), "error must be loud: {message}");
}

#[test]
fn stranded_error_lists_every_retired_only_issue_number() {
    let input = input(
        vec![
            open(11, &[RETIRED]),
            open(12, &["auto-implement"]),
            open(13, &[RETIRED]),
        ],
        vec![12],
    );

    let error = reconcile(&input).expect_err("multiple retired-only issues");
    match &error.kind {
        ReconcileErrorKind::StrandedIssues { issues } => assert_eq!(issues, &[11, 13]),
        other => panic!("expected stranded issues, got {other:?}"),
    }
    let message = error.to_string();
    assert!(message.contains("#11") && message.contains("#13"));
    // The failed run still reports what it would have changed.
    assert!(
        message.contains("no changes"),
        "failed run still reports changes: {message}"
    );
    assert!(error.report.is_some());
}

#[test]
fn reconcile_adds_eligible_and_removes_ineligible() {
    let tracker = vec![
        open(10, &["auto-implement"]),  // eligible, missing from worklist
        open(20, &["safety:reviewed"]), // in worklist, no current label
        open(40, &["auto-implement"]),  // in worklist, eligible
        open(50, &[RETIRED, "auto-implement"]), // migrated: eligible, in worklist
        RemoteIssue::closed(30, "done", "", vec!["auto-implement".to_string()], "agent"),
        // in worklist, closed
    ];
    let report = reconcile(&input(tracker, vec![20, 30, 40, 50, 99]))
        .expect("no retired-only issues in this feed");

    assert_eq!(report.added, vec![10]);
    assert_eq!(report.removed, vec![20, 30, 99]);
    assert_eq!(report.kept, vec![40, 50]);
    assert_eq!(report.updated_worklist, vec![40, 50, 10]);
    assert!(report.changed());
    assert_eq!(
        report.summary(),
        "dispatch-reconciler: added #10, removed #20 #30 #99, kept 2"
    );
}

#[test]
fn reconcile_is_idempotent_and_reports_no_changes_explicitly() {
    let tracker = vec![open(40, &["auto-implement"]), open(50, &["auto-implement"])];
    let first = reconcile(&input(tracker.clone(), vec![40, 50])).expect("consistent feed");
    assert!(!first.changed());
    assert_eq!(first.summary(), "dispatch-reconciler: no changes; kept 2");

    // Feed the first run's output back in: a scheduled second run is a no-op.
    let second = reconcile(&input(tracker, first.updated_worklist)).expect("idempotent second run");
    assert!(!second.changed());
    assert_eq!(second.kept, vec![40, 50]);
    assert_eq!(second.updated_worklist, vec![40, 50]);
}

#[test]
fn policy_rejects_current_selector_among_retired_and_empty_current() {
    let error = DispatchEligibilityPolicy::new(
        "auto-implement",
        vec!["LLM-Ready".to_string(), "auto-implement".to_string()],
    )
    .expect_err("current and retired overlap");
    assert!(matches!(
        error.kind,
        ReconcileErrorKind::InvalidPolicy { .. }
    ));
    assert!(error.report.is_none());
    assert!(error.to_string().contains("ERROR"));

    assert!(matches!(
        DispatchEligibilityPolicy::new("", Vec::<String>::new())
            .expect_err("empty current selector"),
        ReconcileError {
            kind: ReconcileErrorKind::InvalidPolicy { .. },
            ..
        }
    ));
}

#[test]
fn duplicate_worklist_entries_and_tracker_rows_are_collapsed() {
    let tracker = vec![open(10, &["auto-implement"]), open(10, &["auto-implement"])];
    let report = reconcile(&input(tracker, vec![10, 10])).expect("clean feed");
    assert!(!report.changed());
    assert_eq!(report.updated_worklist, vec![10]);
}
