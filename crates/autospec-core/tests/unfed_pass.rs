//! A pass given no work and a pass with no work (issue #4296).
//!
//! The regression tests run in the configuration the incident required:
//! `convpass.sh` taking its work positionally, run bare, while a separate
//! selection tool reported four waiting candidates. The bare run's summary
//! line was byte-identical to a healthy idle run's, and the exit trap
//! re-stated counters the run never populated.

use autospec_core::unfed_pass::bare_invocation_finding;
use autospec_core::unfed_pass::{
    examined_line, identical_summary_finding, missing_examined_finding, plan_invocation,
    refuse_line, trap_line, trap_line_findings, unfed_line, BareDefault, ExitPath, InvocationPlan,
    PassCounters,
};

const TOOL: &str = "convpass";
const SCRIPT: &str = "convpass.sh";
const SELECTOR: &str = "convselect.sh";

/// The line the incident's pass printed both unfed and idle — byte-identical,
/// which is the signal.
const INCIDENT_SUMMARY: &str = "######## convpass: converted=0 held=0 skipped=0 ########";

/// The exit-trap line the incident's pass printed after the guard: it
/// re-stated counters the run never populated.
const INCIDENT_TRAP: &str =
    "######## convpass: TERMINATED rc=0 (converted=0 held=0 skipped=0) ########";

/// The guard line `iwconv.sh` already had and the fix added to `convpass.sh`.
const GUARD_LINE: &str =
    "######## convpass: nothing to convert (no issues given -- run convpass.sh $(convselect.sh)) ########";

/// The four candidates the selector reported while the loop logged a clean
/// idle line: `considered=443 finished_patches=443 have_pr=333
/// closed_issue=284 attempted=250 changed=334 -> candidates=4`.
const INCIDENT_CANDIDATES: [&str; 4] = ["4065", "4251", "4257", "4282"];

fn incident_candidates() -> Vec<String> {
    INCIDENT_CANDIDATES.iter().map(|s| s.to_string()).collect()
}

fn zero() -> PassCounters {
    PassCounters::default()
}

// --- Invariant 1: unfed and idle print different lines --------------------

#[test]
fn incident_summary_line_cannot_tell_unfed_from_idle() {
    // The incident: one line, byte-identical, for a pass given no work and a
    // pass that examined everything and found nothing.
    let findings = identical_summary_finding(INCIDENT_SUMMARY, INCIDENT_SUMMARY);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("UNFED_SUMMARY_IDENTICAL:"));
}

#[test]
fn unfed_and_idle_lines_differ() {
    let unfed = unfed_line(TOOL, SCRIPT, SELECTOR);
    let idle = examined_line(TOOL, &zero());
    assert_ne!(unfed, idle);
    assert!(identical_summary_finding(&unfed, &idle).is_empty());
    // The idle line is not a counter-only line: it carries examined=0.
    assert!(idle.contains("examined=0"));
}

#[test]
fn unfed_line_is_the_sibling_fix() {
    // The guard the sibling file `iwconv.sh` already had: names the absence
    // and the selector that would feed the pass.
    assert_eq!(unfed_line(TOOL, SCRIPT, SELECTOR), GUARD_LINE);
}

// --- Invariant 2: selection is wired into execution by default ------------

#[test]
fn bare_invocation_incident_shape_is_a_finding() {
    // The incident: a bare invocation ran the loop with zero candidates.
    let plan = InvocationPlan::Run(Vec::new());
    let findings = bare_invocation_finding(&[], BareDefault::Refuse, &plan, SELECTOR);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("BARE_RUN_UNFEDED:"));
}

#[test]
fn bare_invocation_refuses_naming_the_selector() {
    let plan = plan_invocation(
        TOOL,
        SCRIPT,
        SELECTOR,
        &[],
        &incident_candidates(),
        BareDefault::Refuse,
    );
    match &plan {
        InvocationPlan::Refuse(line) => {
            assert!(line.contains(SELECTOR));
            assert!(line.contains("no issues"));
            assert!(bare_invocation_finding(&[], BareDefault::Refuse, &plan, SELECTOR).is_empty());
        }
        other => panic!("bare invocation under Refuse must refuse, got {other:?}"),
    }
    assert_eq!(refuse_line(TOOL, SCRIPT, SELECTOR), "error: convpass given no issues: run convpass.sh $(convselect.sh), or pass issue numbers positionally");
}

#[test]
fn bare_invocation_refusal_without_selector_is_a_finding() {
    let plan = InvocationPlan::Refuse("error: no issues given".to_string());
    let findings = bare_invocation_finding(&[], BareDefault::Refuse, &plan, SELECTOR);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("REFUSE_MISSING_SELECTOR:"));
}

#[test]
fn bare_invocation_runs_the_selector_by_default() {
    // Invariant 2, the designed-in half: the pass runs the selector itself,
    // so the four waiting candidates are examined instead of sat behind.
    let plan = plan_invocation(
        TOOL,
        SCRIPT,
        SELECTOR,
        &[],
        &incident_candidates(),
        BareDefault::RunSelector,
    );
    match &plan {
        InvocationPlan::Run(candidates) => {
            let given: Vec<&str> = INCIDENT_CANDIDATES.to_vec();
            assert_eq!(candidates, &incident_candidates());
            // A fed run has no bare-invocation finding.
            assert!(
                bare_invocation_finding(&given, BareDefault::RunSelector, &plan, SELECTOR)
                    .is_empty()
            );
        }
        other => panic!("bare invocation under RunSelector must run, got {other:?}"),
    }
}

#[test]
fn empty_selector_result_is_a_true_idle_not_a_finding() {
    // The selector ran and found nothing: examined=0 is a true idle, and the
    // pass knew it had been fed — no finding, unlike the bare-Run case.
    let plan = plan_invocation(TOOL, SCRIPT, SELECTOR, &[], &[], BareDefault::RunSelector);
    match &plan {
        InvocationPlan::Run(candidates) => {
            assert!(candidates.is_empty());
            assert!(
                bare_invocation_finding(&[], BareDefault::RunSelector, &plan, SELECTOR).is_empty()
            );
        }
        other => panic!("RunSelector with an empty selector result must run idle, got {other:?}"),
    }
}

#[test]
fn given_candidates_run_positionally() {
    // The normal form is unchanged: work handed positionally is run.
    let given: Vec<&str> = INCIDENT_CANDIDATES.to_vec();
    let plan = plan_invocation(TOOL, SCRIPT, SELECTOR, &given, &[], BareDefault::Refuse);
    assert_eq!(plan, InvocationPlan::Run(incident_candidates()));
    assert!(bare_invocation_finding(&given, BareDefault::Refuse, &plan, SELECTOR).is_empty());
}

// --- Invariant 3: the trap reflects the exit path --------------------------

#[test]
fn incident_trap_restates_unpopulated_counters() {
    // The incident: the guard line was followed by a trap line re-stating
    // counters that were never populated — the diagnostic overwritten with
    // the thing it was correcting.
    let findings = trap_line_findings(&ExitPath::Guarded, INCIDENT_TRAP);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("TRAP_RESTATES_COUNTERS:"));
    assert!(findings[0].contains("converted="));
}

#[test]
fn guarded_trap_line_names_the_guard_and_no_counters() {
    let line = trap_line(TOOL, 0, &ExitPath::Guarded);
    assert!(line.contains("on guard"));
    assert!(!line.contains("converted="));
    assert!(!line.contains("held="));
    assert!(!line.contains("skipped="));
    assert!(trap_line_findings(&ExitPath::Guarded, &line).is_empty());
}

#[test]
fn completed_trap_line_carries_populated_counters() {
    let counters = PassCounters {
        examined: 4,
        converted: 4,
        held: 0,
        skipped: 0,
    };
    let line = trap_line(TOOL, 0, &ExitPath::Completed(counters));
    assert!(line.contains("examined=4"));
    assert!(line.contains("converted=4"));
    assert!(trap_line_findings(&ExitPath::Completed(counters), &line).is_empty());
}

// --- Invariant 4: examined= is reported alongside the action counters -----

#[test]
fn incident_summary_line_missing_examined() {
    let findings = missing_examined_finding(INCIDENT_SUMMARY);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("EXAMINED_MISSING:"));
    // The fixed line reports the size of the input.
    let fixed = examined_line(TOOL, &zero());
    assert!(missing_examined_finding(&fixed).is_empty());
}

#[test]
fn counters_must_reconcile() {
    // A pass that acted on more than it examined is a state that cannot
    // exist.
    assert!(!PassCounters {
        examined: 0,
        converted: 1,
        held: 0,
        skipped: 0,
    }
    .reconciles());
    assert!(!PassCounters {
        examined: 2,
        converted: 1,
        held: 1,
        skipped: 1,
    }
    .reconciles());
    assert!(PassCounters {
        examined: 4,
        converted: 4,
        held: 0,
        skipped: 0,
    }
    .reconciles());
    assert!(zero().reconciles());
}

// --- The fixed pass, end to end -------------------------------------------

#[test]
fn fixed_pass_end_to_end() {
    // The fixed shape: a bare invocation runs the selector by default, the
    // four waiting candidates are examined, and every line the pass prints
    // tells which state it is in.
    let plan = plan_invocation(
        TOOL,
        SCRIPT,
        SELECTOR,
        &[],
        &incident_candidates(),
        BareDefault::RunSelector,
    );
    let candidates = match plan {
        InvocationPlan::Run(candidates) => candidates,
        other => panic!("expected a run, got {other:?}"),
    };
    assert_eq!(candidates.len(), 4);
    let counters = PassCounters {
        examined: 4,
        converted: 4,
        held: 0,
        skipped: 0,
    };
    assert!(counters.reconciles());
    let summary = examined_line(TOOL, &counters);
    let unfed = unfed_line(TOOL, SCRIPT, SELECTOR);
    assert!(identical_summary_finding(&unfed, &summary).is_empty());
    assert!(missing_examined_finding(&summary).is_empty());
    let trap = trap_line(TOOL, 0, &ExitPath::Completed(counters));
    assert!(trap_line_findings(&ExitPath::Completed(counters), &trap).is_empty());

    // And the unfed branch, when it is actually reached: different line,
    // guard-named trap, no re-stated counters.
    assert_ne!(unfed, summary);
    assert!(
        trap_line_findings(&ExitPath::Guarded, &trap_line(TOOL, 0, &ExitPath::Guarded)).is_empty()
    );
}
