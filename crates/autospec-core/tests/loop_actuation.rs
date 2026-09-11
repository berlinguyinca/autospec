//! Regression tests for `autospec_core::loop_actuation` (issue #4268).
//!
//! The tests run in the configuration the bug required: a frontier
//! loop that reports decisions it cannot execute, over a precondition
//! no component satisfies, over a history of passes with zero
//! dispatches. On a loop that dispatches, every check agrees and the
//! audit is empty — the control case that proves the checks are not
//! false-positiving.

use std::collections::BTreeMap;

use autospec_core::loop_actuation::{
    actuation_findings, audit, precondition_findings, ActuatorStatus, EndToEndEvidence, LoopReport,
    Precondition,
};

fn blocked(counts: &[(&str, usize)]) -> BTreeMap<String, usize> {
    counts.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

/// The incident pass: 8 ready, 0 dispatched, all 8 blocked on a
/// precondition no component produced, and no actuator status in the
/// report.
fn incident() -> (LoopReport, Vec<Precondition>, EndToEndEvidence) {
    let report = LoopReport {
        ready: 8,
        dispatched: 0,
        blocked: blocked(&[("no staged spec", 8)]),
        actuator: None,
    };
    let preconditions = vec![Precondition {
        name: "staged spec at iw/issues/<n>.md".to_string(),
        satisfied: false,
        owner: None,
    }];
    (
        report,
        preconditions,
        EndToEndEvidence::NeverDispatched { passes: 40 },
    )
}

#[test]
fn incident_report_names_what_it_could_not_do() {
    let (report, preconditions, evidence) = incident();
    let line = report.line();
    assert!(line.contains("8 ready, 0 dispatched"), "{line}");
    assert!(line.contains("8 blocked: no staged spec"), "{line}");
    assert!(line.contains("actuator: not reported"), "{line}");

    let findings = audit(&report, &preconditions, evidence);
    assert!(
        findings
            .iter()
            .any(|f| f.starts_with("ACTUATION_NOT_REPORTED")),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.starts_with("OWNERLESS_PRECONDITION")),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.starts_with("NOT_OBSERVED_TO_ACT")),
        "{findings:?}"
    );
    assert!(!evidence.loop_working());
}

/// The durable fix: the staging step is called from the frontier loop,
/// so the actuator's precondition has an owner and holds, and the pass
/// dispatches.
#[test]
fn fixed_loop_dispatches_and_reports() {
    let report = LoopReport {
        ready: 8,
        dispatched: 8,
        blocked: BTreeMap::new(),
        actuator: Some(ActuatorStatus::CouldExecute),
    };
    assert_eq!(report.line(), "8 ready, 8 dispatched, could execute");
    assert!(report.reconciles());
    assert!(actuation_findings(&report).is_empty());

    let preconditions = vec![Precondition {
        name: "staged spec at iw/issues/<n>.md".to_string(),
        satisfied: true,
        owner: Some("iw-stage.sh, called from the frontier loop".to_string()),
    }];
    assert!(precondition_findings(&preconditions).is_empty());

    let evidence = EndToEndEvidence::Dispatched { passes: 1 };
    assert!(evidence.loop_working());
    assert!(audit(&report, &preconditions, evidence).is_empty());
}

#[test]
fn ownerless_unsatisfied_precondition_is_the_permanent_block() {
    let p = Precondition {
        name: "staged spec at iw/issues/<n>.md".to_string(),
        satisfied: false,
        owner: None,
    };
    let findings = precondition_findings(&[p.clone()]);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].starts_with("OWNERLESS_PRECONDITION"),
        "{findings:?}"
    );
    assert_eq!(
        p.hold_line(),
        "blocked: staged spec at iw/issues/<n>.md (no owner)"
    );

    // With a named owner the same unsatisfied precondition is a
    // temporary hold, not a finding — and the hold line names the
    // owner, so the block is visible as having a release path.
    let p = Precondition {
        name: p.name,
        satisfied: false,
        owner: Some("iw-stage.sh, called from the frontier loop".to_string()),
    };
    assert!(precondition_findings(&[p.clone()]).is_empty());
    assert!(
        p.hold_line().contains("owner: iw-stage.sh"),
        "{}",
        p.hold_line()
    );
}

#[test]
fn idle_loop_reports_no_decisions_and_owes_no_actuation() {
    let report = LoopReport {
        ready: 0,
        dispatched: 0,
        blocked: BTreeMap::new(),
        actuator: None,
    };
    // The control case: an idle loop made no decision, so the report
    // owes no actuator status and the findings are empty — the check
    // is not a false positive on quiet operation.
    assert!(actuation_findings(&report).is_empty());
    assert_eq!(report.line(), "0 ready, 0 dispatched");
}

#[test]
fn could_execute_but_not_dispatched_is_a_gap() {
    let report = LoopReport {
        ready: 8,
        dispatched: 3,
        blocked: BTreeMap::new(),
        actuator: Some(ActuatorStatus::CouldExecute),
    };
    let findings = actuation_findings(&report);
    assert!(
        findings.iter().any(|f| f.starts_with("ACTUATION_GAP")),
        "{findings:?}"
    );
}

#[test]
fn cannot_execute_fragment_names_owner_or_its_absence() {
    let with_owner = ActuatorStatus::CannotExecute {
        reason: "no staged spec".to_string(),
        owner: Some("iw-stage.sh".to_string()),
    };
    assert_eq!(
        with_owner.fragment(),
        "cannot execute: no staged spec (owner: iw-stage.sh)"
    );
    let without = ActuatorStatus::CannotExecute {
        reason: "no staged spec".to_string(),
        owner: None,
    };
    assert_eq!(
        without.fragment(),
        "cannot execute: no staged spec (no owner)"
    );

    // A pass that reports the hold with a status (even ownerless) is
    // not an ACTUATION_NOT_REPORTED finding — the loop stated what it
    // could do — but the line still names the missing owner.
    let report = LoopReport {
        ready: 8,
        dispatched: 0,
        blocked: blocked(&[("no staged spec", 8)]),
        actuator: Some(without),
    };
    assert!(actuation_findings(&report).is_empty());
    assert!(report.line().contains("(no owner)"), "{}", report.line());
}

#[test]
fn unreconciled_report_is_a_state_that_cannot_exist() {
    let report = LoopReport {
        ready: 4,
        dispatched: 3,
        blocked: blocked(&[("no staged spec", 3)]),
        actuator: Some(ActuatorStatus::CouldExecute),
    };
    assert!(!report.reconciles());
    let findings = actuation_findings(&report);
    assert!(
        findings
            .iter()
            .any(|f| f.starts_with("REPORT_DOES_NOT_RECONCILE")),
        "{findings:?}"
    );
}

#[test]
fn blocked_reasons_render_sorted() {
    let report = LoopReport {
        ready: 5,
        dispatched: 0,
        blocked: blocked(&[("no staged spec", 3), ("worker saturated", 2)]),
        actuator: Some(ActuatorStatus::CouldExecute),
    };
    let line = report.line();
    let first = line.find("3 blocked: no staged spec").unwrap();
    let second = line.find("2 blocked: worker saturated").unwrap();
    assert!(first < second, "{line}");
}

#[test]
fn evidence_line_distinguishes_working_from_compute_only() {
    let working = EndToEndEvidence::Dispatched { passes: 2 };
    assert!(working.loop_working());
    assert!(working.line().contains("working"), "{}", working.line());

    // Zero dispatches is compute-not-act even at zero passes — the
    // loop has simply not been observed to act yet, and the audit
    // says so rather than assuming.
    let stuck = EndToEndEvidence::NeverDispatched { passes: 0 };
    assert!(!stuck.loop_working());
    let findings = audit(
        &LoopReport {
            ready: 0,
            dispatched: 0,
            blocked: BTreeMap::new(),
            actuator: None,
        },
        &[],
        stuck,
    );
    assert!(
        findings
            .iter()
            .any(|f| f.starts_with("NOT_OBSERVED_TO_ACT")),
        "{findings:?}"
    );
}
