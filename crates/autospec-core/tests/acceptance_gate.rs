//! The acceptance gate for conversion closure (issue #4274).
//!
//! The regression tests instantiate the configuration the bug required: an
//! issue with four acceptance criteria and a report that satisfies only
//! one. Against that shape, an ungated conversion fires `Closes #N` from a
//! patch that was never checked against the acceptance list; every test
//! below fails on the code that produced the incident.

use autospec_core::evidence_fidelity::{closure_authorized, ClosureVerdict};
use autospec_core::execution::acceptance_gate::{
    assess, assess_report, closes_authorized, closure_body, follow_up_body,
    parse_acceptance_criteria, parse_criterion_statuses, verdict_line, CriterionStatus, Verdict,
};

/// The issue body the incident ran on: four individually checkable
/// criteria.
const FOUR_CRITERIA_ISSUE: &str = "## Goal\n\nShip the converter.\n\n## Acceptance criteria\n\n\
- [ ] the conversion pass runs in the CLI\n\
- [ ] a unit test under tests/unit/ covers the verdict table\n\
- [ ] the doc row is present in docs/cli-reference.md\n\
- [ ] the smoke command prints SMOKE_OK\n";

/// The report the incident ran on: one criterion met, the rest unmarked.
const ONE_MET_REPORT: &str = "status=PASS build_rc=0 test_rc=0 criteria=met\n";

/// A complete assessment of the four-criterion issue: criterion one met,
/// the rest partial.
const PARTIAL_REPORT: &str =
    "status=PASS build_rc=0 test_rc=0 criteria=met,partial,partial,partial\n";

#[test]
fn a_spec_with_criteria_never_closes_from_a_report_without_statuses() {
    let verdict = assess_report(FOUR_CRITERIA_ISSUE, "status=PASS build_rc=0 test_rc=0").unwrap();
    assert_eq!(
        verdict,
        Verdict::Unassessed {
            missing: vec![1, 2, 3, 4]
        }
    );
    let error = closure_body(&verdict, 310, "converted the patch").unwrap_err();
    assert!(error.contains("310"), "{error}");
    assert!(error.contains("1, 2, 3, 4"), "{error}");
}

#[test]
fn the_incident_report_is_unassessed_and_does_not_close() {
    // One recorded status for four criteria is not a partial close: the
    // report never assessed three of the list, so the hold names them.
    let verdict = assess_report(FOUR_CRITERIA_ISSUE, ONE_MET_REPORT).unwrap();
    assert_eq!(
        verdict,
        Verdict::Unassessed {
            missing: vec![2, 3, 4]
        }
    );
    let error = closure_body(&verdict, 310, "one of four done").unwrap_err();
    assert!(error.contains("310"), "{error}");
    assert!(!closes_authorized(&verdict, 310));
}

#[test]
fn a_partially_met_report_does_not_close() {
    let verdict = assess_report(FOUR_CRITERIA_ISSUE, PARTIAL_REPORT).unwrap();
    assert_eq!(
        verdict,
        Verdict::PartiallyMet {
            remainder: vec![2, 3, 4]
        }
    );
    let body = closure_body(&verdict, 310, "one of four done").unwrap();
    // The close keyword is gone; the partial-fix marker is in its place.
    assert!(
        matches!(
            closure_authorized(&body, 310),
            ClosureVerdict::NotAuthorized
        ),
        "partially-met body must not close the issue: {body:?}"
    );
    assert!(body.contains("#310"), "{body:?}");
    let line = verdict_line(&verdict, 4);
    assert!(line.contains("1/4"), "{line}");
    assert!(line.contains("2, 3, 4"), "{line}");
    assert!(line.contains("follow-up"), "{line}");
}

#[test]
fn the_remainder_becomes_a_follow_up_issue_with_its_own_criteria() {
    let verdict = assess_report(FOUR_CRITERIA_ISSUE, PARTIAL_REPORT).unwrap();
    let criteria = parse_acceptance_criteria(FOUR_CRITERIA_ISSUE);
    let follow_up = follow_up_body(310, &criteria, &verdict).expect("partial has a follow-up");
    // The follow-up's acceptance section is the remainder itself — three
    // items, individually checkable, in order.
    let follow_up_criteria = parse_acceptance_criteria(&follow_up);
    assert_eq!(
        follow_up_criteria,
        vec![
            "a unit test under tests/unit/ covers the verdict table",
            "the doc row is present in docs/cli-reference.md",
            "the smoke command prints SMOKE_OK",
        ]
    );
    // Closing the follow-up completes the parent's list: the met item is
    // gone and the remainder is all that is left to do.
    let follow_up_verdict = Verdict::Complete;
    assert_eq!(
        assess(&follow_up_criteria, &[CriterionStatus::Met; 3]),
        follow_up_verdict
    );
    let body = closure_body(&follow_up_verdict, 310, "remainder done").unwrap();
    assert!(body.contains("Closes #310"), "{body:?}");
}

#[test]
fn an_all_met_report_closes() {
    let verdict = assess_report(
        FOUR_CRITERIA_ISSUE,
        "status=PASS build_rc=0 test_rc=0 criteria=met,met,met,met",
    )
    .unwrap();
    assert_eq!(verdict, Verdict::Complete);
    let body = closure_body(&verdict, 310, "all four done").unwrap();
    assert!(
        matches!(
            closure_authorized(&body, 310),
            ClosureVerdict::Authorized { .. }
        ),
        "complete body must close the issue: {body:?}"
    );
    assert!(closes_authorized(&verdict, 310));
}

#[test]
fn deferred_criteria_join_the_remainder() {
    let verdict = assess_report(
        FOUR_CRITERIA_ISSUE,
        "status=PASS build_rc=0 test_rc=0 criteria=met,deferred,met,deferred",
    )
    .unwrap();
    assert_eq!(
        verdict,
        Verdict::PartiallyMet {
            remainder: vec![2, 4]
        }
    );
    let line = verdict_line(&verdict, 4);
    assert!(line.contains("2, 4"), "{line}");
}

#[test]
fn a_short_report_is_unassessed_not_partial() {
    // Three of four marked is not a partial close; the unmarked fourth is
    // unassessed and the hold names it.
    let verdict = assess_report(
        FOUR_CRITERIA_ISSUE,
        "status=PASS build_rc=0 test_rc=0 criteria=met,met,partial",
    )
    .unwrap();
    assert_eq!(verdict, Verdict::Unassessed { missing: vec![4] });
    assert!(!closes_authorized(&verdict, 310));
}

#[test]
fn an_issue_without_criteria_is_not_required() {
    let body = "## Goal\n\nA docs-only tweak.\n\n## Files touched\n\n- docs/notes.md\n";
    let verdict = assess_report(body, "status=PASS build_rc=0").unwrap();
    assert_eq!(verdict, Verdict::NotRequired);
    assert_eq!(
        verdict_line(&verdict, 0),
        "ACCEPTANCE: not required — the issue declares no acceptance criteria"
    );
}

#[test]
fn a_report_a_gate_cannot_read_fails_the_conversion() {
    let error =
        assess_report(FOUR_CRITERIA_ISSUE, "status=PASS criteria=met,done,met,met").unwrap_err();
    assert!(error.contains("`done`"), "{error}");
}

#[test]
fn the_fleet_shape_report_is_read_the_same_way() {
    let fleet_report =
        "status: PASS\nbuild_rc: 0\ntest_rc: 0\ncriteria: met partial deferred met\n";
    let verdict = assess_report(FOUR_CRITERIA_ISSUE, fleet_report).unwrap();
    assert_eq!(
        assess(
            &parse_acceptance_criteria(FOUR_CRITERIA_ISSUE),
            &parse_criterion_statuses(fleet_report).unwrap().unwrap()
        ),
        verdict
    );
    assert_eq!(
        verdict,
        Verdict::PartiallyMet {
            remainder: vec![2, 3]
        }
    );
}
