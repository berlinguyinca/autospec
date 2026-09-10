//! Issue #3768: a timed-out gate reported "fixes: <all seven checks>".
//!
//! An empty result set is not "nothing failed"; it is "no verdict". The
//! two are identical to every set operation and opposite in meaning.

use std::collections::BTreeSet;

use autospec_core::gate_verdict::{
    check_timeout_budget, diff_failed_checks, BudgetFinding, Completion, GateDiffVerdict,
    MeasuredRun,
};

fn baseline_seven() -> BTreeSet<String> {
    (1..=7).map(|n| format!("check_{n}")).collect()
}

/// The merged PR's claim, reproduced: `timeout 900` killed the gate against
/// a command whose measured wall-clock is 741–1189s, `$now` is empty, and a
/// bare `comm -23 baseline now` reads "everything fixed".
#[test]
fn the_incident_gate_holds_with_a_reason_and_claims_no_fix() {
    let verdict = diff_failed_checks(&Completion::TimedOut, &baseline_seven(), &BTreeSet::new());

    // Rule 2: a timed-out gate produces a hold with the reason — never a
    // pass and never a "fixed" claim.
    assert!(verdict.is_no_verdict(), "{verdict:?}");
    assert!(verdict.holds(), "{verdict:?}");
    assert!(!verdict.passes(), "{verdict:?}");
    assert_eq!(verdict.fixed(), None, "{verdict:?}");

    let line = verdict.render("validate");
    assert!(line.contains("NO-VERDICT"), "{line}");
    assert!(line.contains("timeout"), "{line}");
    assert!(!line.contains("fixes:"), "{line}");
    assert!(!line.contains("no new failures"), "{line}");
}

/// The pre-fix report line is unreachable: no completion state renders a
/// fixes clause that a bare set difference would have produced.
#[test]
fn no_completion_state_can_render_the_incident_claim() {
    let baseline = baseline_seven();
    let empty: BTreeSet<String> = BTreeSet::new();
    for completion in [
        Completion::TimedOut,
        Completion::Errored {
            reason: "spawn failed".to_string(),
        },
    ] {
        let verdict = diff_failed_checks(&completion, &baseline, &empty);
        assert!(
            !verdict.render("validate").contains("fixes:"),
            "{verdict:?}"
        );
        assert_eq!(verdict.fixed(), None, "{verdict:?}");
    }
}

/// The one state that may claim a fix is a completed run: a green
/// `validate` (exit 0) with an empty failing set really did fix the
/// baseline.
#[test]
fn only_a_completed_run_may_claim_the_fix() {
    let verdict = diff_failed_checks(
        &Completion::Ran { exit_code: 0 },
        &baseline_seven(),
        &BTreeSet::new(),
    );
    let GateDiffVerdict::Judged {
        new_failures,
        fixed,
    } = &verdict
    else {
        panic!("expected Judged, got {verdict:?}");
    };
    assert!(new_failures.is_empty());
    assert_eq!(fixed.len(), 7, "{fixed:?}");
    assert!(verdict.passes());
}

/// Rule 3: the incident's 900s budget does not exceed the measured
/// wall-clock (741–1189s), and a budget with no recorded measurement is a
/// finding, not an OK.
#[test]
fn every_timeout_budget_exceeds_the_recorded_measurement() {
    let measurements = [
        MeasuredRun {
            secs: 741,
            source: "measured on main, run A".to_string(),
        },
        MeasuredRun {
            secs: 1189,
            source: "measured on main, run B".to_string(),
        },
    ];

    let finding =
        check_timeout_budget(900, &measurements).expect("the incident budget must be a finding");
    assert!(matches!(finding, BudgetFinding::BudgetBelowMeasured { .. }));

    assert_eq!(
        check_timeout_budget(1200, &measurements),
        None,
        "a budget above the longest measurement validates"
    );

    let missing =
        check_timeout_budget(900, &[]).expect("a budget with no measurement must be a finding");
    assert!(matches!(missing, BudgetFinding::MissingMeasurement { .. }));
}
