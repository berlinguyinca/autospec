use autospec_core::autonomous::audit::{classify_failure, FailureClass};

#[test]
fn autonomous_audit_classifies_dry_promotion_zero_filed_as_no_work() {
    let class = classify_failure("dry promotion cycles filed=0 and then repeated");

    assert_eq!(class, FailureClass::NoWork);
}

#[test]
fn autonomous_audit_classifies_dry_run_live_mutation_as_guideline_violation() {
    let class =
        classify_failure("autospec-autonomous --dry-run silently runs live and mutates GitHub");

    assert_eq!(class, FailureClass::GuidelineViolation);
}

#[test]
fn autonomous_audit_classifies_generated_cache_reports_as_false_positive() {
    let class =
        classify_failure("gitleaks reported generated .next and node_modules cache findings");

    assert_eq!(class, FailureClass::FalsePositive);
}

#[test]
fn autonomous_audit_classifies_validation_failures_as_validation_blocked() {
    let class =
        classify_failure("validation failed: CI checks are failing and main-health pending");

    assert_eq!(class, FailureClass::ValidationBlocked);
}

#[test]
fn autonomous_audit_classifies_watchdog_liveness_failures_as_stuck() {
    let class = classify_failure(
        "crashed conductor blocks relaunch until the watchdog reclaims the heartbeat lock",
    );

    assert_eq!(class, FailureClass::Stuck);
}
