//! Triage: what the conversion pass does with a run's recorded report
//! (#3715), and the signalled run it used to swallow (#4651).
//!
//! The routing lives here rather than in `run_status.rs` because the two
//! questions are different: that file asks whether a status *name* means
//! what the vocabulary says it means, this one asks what the pass *does*
//! once the name is resolved. It matters most for the case #4651 added,
//! where the name in the record is not the fact that decides — a killed
//! agent is identified by its exit code, and the code outranks the label.

use autospec_core::execution::status_triage::{
    decision_line, triage, AgentHoldReason, AgentReport, GateBasis, TriageDecision,
};
use autospec_core::run_status::emitted;

fn report(
    status: Option<&str>,
    build_rc: Option<i32>,
    test_rc: Option<i32>,
    fmt_rc: Option<i32>,
) -> AgentReport {
    AgentReport {
        status: status.map(str::to_string),
        build_rc,
        test_rc,
        fmt_rc,
        fmt_files: None,
        ..AgentReport::default()
    }
}

#[test]
fn triage_routes_the_statuses_the_runner_actually_writes() {
    // Each of these names was previously unknown to triage: the run fell through
    // to the green arm, which ran the local gate over an unbuilt tree.
    assert_eq!(
        triage(&report(Some("BUILD-FAIL"), Some(1), None, Some(0))),
        TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt
        }
    );
    // FMT-DIRTY with a clean build: the recorded verdict is not trusted over a
    // local repair the stage can run itself (#4099). The caller formats and
    // re-checks before judging.
    assert_eq!(
        triage(&report(Some("FMT-DIRTY"), Some(0), Some(0), Some(1))),
        TriageDecision::FormatAndRecheck
    );
    assert_eq!(
        triage(&report(Some("NO-OUTPUT"), None, None, None)),
        TriageDecision::RaiseForReview {
            status: "NO-OUTPUT".to_string()
        }
    );
    assert_eq!(
        triage(&report(Some("TEST-TIMEOUT"), Some(0), None, Some(0))),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure {
                status: Some("TEST-TIMEOUT".to_string())
            }
        }
    );
}

#[test]
fn triage_reaches_a_legacy_spelling_through_the_vocabulary() {
    // The old name must route to the same decision as the runner's own.
    let legacy = triage(&report(Some("BUILD-FAILED"), None, None, None));
    let current = triage(&report(Some("BUILD-FAIL"), None, None, None));
    assert_eq!(legacy, current);
    assert_eq!(
        legacy,
        TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt
        }
    );
    assert_eq!(
        triage(&report(Some("UNKNOWN-NO-FMT-BASELINE"), None, None, None)),
        TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline
        }
    );
}

#[test]
fn a_verified_report_with_a_failing_test_rc_triages_on_the_code() {
    // The word loses to the exit code: a gate is run, the run is not held.
    let d = triage(&report(Some("VERIFIED"), Some(0), Some(101), Some(0)));
    assert_eq!(
        d,
        TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure {
                status: Some("VERIFIED".to_string())
            }
        }
    );
    // A no-baseline verdict keeps its own rule even beside a failing test_rc.
    assert_eq!(
        triage(&report(
            Some("UNKNOWN-NO-BASELINE"),
            Some(0),
            Some(1),
            Some(0)
        )),
        TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline
        }
    );
    // Truly green still goes to the local gate.
    assert_eq!(
        triage(&report(Some("VERIFIED"), Some(0), Some(0), Some(0))),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentGreen
        }
    );
}

#[test]
fn every_emitted_status_has_a_triage_route() {
    // No status may fall through to a default: the fall-through is what made the
    // unbuilt runs look convertible.
    for name in emitted() {
        let d = triage(&report(Some(name), None, None, None));
        match d {
            TriageDecision::Redispatch { .. }
            | TriageDecision::RaiseForReview { .. }
            | TriageDecision::Hold { .. }
            | TriageDecision::FormatAndRecheck
            | TriageDecision::Signalled { .. }
            | TriageDecision::GateLocally { .. } => {}
        }
    }
}

// ---------------------------------------------------------------------------
// A signalled agent is refused on its own status (#4651).
// ---------------------------------------------------------------------------

#[test]
fn a_signalled_run_is_refused_not_gated_and_not_held() {
    // The pass must not spend a gate on a tree whose agent is gone, and must
    // not hold it as a deterministic property of the patch: a killed agent
    // established neither.
    let d = triage(&report(Some("SIGNALLED"), None, None, None));
    assert_eq!(
        d,
        TriageDecision::Signalled {
            signal: None,
            status: "SIGNALLED".to_string()
        }
    );
}

#[test]
fn the_exit_code_outranks_a_label_that_says_otherwise() {
    // iw-87 died mid-edit and its record said `UNKNOWN-NO-BASELINE` — the
    // value a healthy run gets when no baseline is measurable (#4644). The
    // exit code is the field that cannot be wrong about how it died.
    let killed = AgentReport {
        status: Some("UNKNOWN-NO-BASELINE".to_string()),
        agent_rc: Some(143),
        ..AgentReport::default()
    };
    assert_eq!(
        triage(&killed),
        TriageDecision::Signalled {
            signal: None,
            status: "SIGNALLED".to_string()
        }
    );
    // The same label with no signal keeps its own rule: a genuinely
    // unmeasurable baseline is still the converter's job.
    assert_eq!(
        triage(&report(Some("UNKNOWN-NO-BASELINE"), None, None, None)),
        TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline
        }
    );
}

#[test]
fn a_killed_agent_never_becomes_a_repairable_fmt_failure() {
    // The concrete harm: a 2000-line half-applied edit carrying fmt_rc=1 and
    // test_rc=101 triaged as a formatting defect the pass should repair, so
    // the pass formatted it and offered it to the conversion queue.
    let half_applied = AgentReport {
        status: Some("UNKNOWN-NO-BASELINE".to_string()),
        build_rc: Some(0),
        test_rc: Some(101),
        fmt_rc: Some(1),
        agent_rc: Some(143),
        signal: Some("SIGTERM".to_string()),
        ..AgentReport::default()
    };
    let d = triage(&half_applied);
    assert!(matches!(d, TriageDecision::Signalled { .. }), "{d:?}");
    assert_ne!(d, TriageDecision::FormatAndRecheck);
    assert_ne!(
        d,
        TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure { status: None }
        }
    );
}

#[test]
fn the_runners_own_timeout_is_still_a_timeout_not_a_signal() {
    // 124 is the runner ending its own run: re-dispatch, which is what the
    // timeout rule has always done. Conflating the two would turn every
    // limit expiry into an unattributed kill.
    let own = AgentReport {
        status: Some("TIMEOUT".to_string()),
        agent_rc: Some(124),
        ..AgentReport::default()
    };
    assert_eq!(
        triage(&own),
        TriageDecision::Redispatch {
            status: "TIMEOUT".to_string()
        }
    );
    // And a runner that *asserts* its own limit fired keeps that assertion
    // even when its code is ambiguous: a harness reporting the child's
    // death-signal writes 143 where `timeout` itself exits 124, so the claim
    // outranks the inference (#4651 AC3 asks the runner to say which).
    let asserted = AgentReport {
        status: Some("TIMEOUT".to_string()),
        agent_rc: Some(143),
        ..AgentReport::default()
    };
    assert_eq!(
        triage(&asserted),
        TriageDecision::Redispatch {
            status: "TIMEOUT".to_string()
        }
    );
}

#[test]
fn a_recorded_signal_names_itself_in_the_refusal() {
    let report = AgentReport {
        status: Some("SIGNALLED".to_string()),
        agent_rc: Some(143),
        signal: Some("SIGTERM".to_string()),
        ..AgentReport::default()
    };
    let d = triage(&report);
    assert_eq!(
        d,
        TriageDecision::Signalled {
            signal: Some("SIGTERM".to_string()),
            status: "SIGNALLED".to_string()
        }
    );
    // The line the operator reads carries the attribution: which signal, and
    // the fact that the runner's own timeout did not fire (#4651 AC3).
    let line = decision_line(&d, &report);
    assert!(line.contains("REFUSED"), "{line}");
    assert!(line.contains("SIGTERM"), "{line}");
    assert!(line.contains("124"), "{line}");
    assert!(line.contains("unattributed"), "{line}");
}

#[test]
fn a_record_that_named_only_a_signal_is_still_signalled() {
    // A runner killed before it wrote its label leaves `signal=` and nothing
    // else; absence of a status is not absence of a termination.
    let report = AgentReport {
        signal: Some("SIGKILL".to_string()),
        ..AgentReport::default()
    };
    assert!(
        matches!(triage(&report), TriageDecision::Signalled { .. }),
        "a lone signal must not fall through to the green arm"
    );
}

#[test]
fn signalling_a_run_changes_nothing_about_an_ordinary_record() {
    // The gate must not start seeing signals where it never saw one: every
    // status the runner writes still routes exactly as before #4651, so long
    // as the record names no termination.
    for name in emitted() {
        if name == "SIGNALLED" {
            continue;
        }
        let d = triage(&report(Some(name), None, None, None));
        assert!(
            !matches!(d, TriageDecision::Signalled { .. }),
            "{name} must not read as a kill"
        );
    }
}
