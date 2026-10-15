use super::*;

fn report(
    status: Option<&str>,
    build_rc: Option<i32>,
    test_rc: Option<i32>,
    fmt_rc: Option<i32>,
    fmt_files: Option<usize>,
) -> AgentReport {
    AgentReport {
        status: status.map(str::to_string),
        build_rc,
        test_rc,
        fmt_rc,
        fmt_files,
        ..AgentReport::default()
    }
}

// --- parsing -----------------------------------------------------

#[test]
fn parses_the_gate_shape_single_line() {
    let parsed = parse_agent_report("status=PASS build_rc=0 test_rc=0 fmt_rc=0").unwrap();
    assert_eq!(
        parsed,
        report(Some("PASS"), Some(0), Some(0), Some(0), None)
    );
}

#[test]
fn parses_the_fleet_shape_multi_line() {
    let content = "\
# agent gate record
status: TIMEOUT
build_rc: 0
test_rc: 0
fmt_rc: 1

worker: gpu-4090-03
";
    let parsed = parse_agent_report(content).unwrap();
    assert_eq!(
        parsed,
        report(Some("TIMEOUT"), Some(0), Some(0), Some(1), None)
    );
}

#[test]
fn parses_the_fmt_files_sidecar_count() {
    let parsed = parse_agent_report("fmt-files.txt: 32 entries").unwrap();
    assert_eq!(parsed.fmt_files, Some(32));
    let bare = parse_agent_report("fmt-files.txt: 7").unwrap();
    assert_eq!(bare.fmt_files, Some(7));
}

#[test]
fn ignores_unknown_keys_comments_and_blanks() {
    let parsed = parse_agent_report("status: PASS\nagent_secs: 60\n# note\n\ngpu: 4090\n").unwrap();
    assert_eq!(parsed.status.as_deref(), Some("PASS"));
    assert!(parsed.build_rc.is_none());
    assert!(parsed.fmt_files.is_none());
}

#[test]
fn duplicate_keys_take_the_last_value() {
    let parsed = parse_agent_report("status: TIMEOUT\nstatus: PASS").unwrap();
    assert_eq!(parsed.status.as_deref(), Some("PASS"));
}

#[test]
fn mixes_both_shapes_across_lines() {
    // The issue's own example mixes `key=value` and `key: value`.
    let parsed = parse_agent_report("status=TIMEOUT\nfmt_rc=1\nfmt-files.txt: 32 entries").unwrap();
    assert_eq!(
        parsed,
        report(Some("TIMEOUT"), None, None, Some(1), Some(32))
    );
}

#[test]
fn a_non_integer_rc_fails_the_file_naming_the_line() {
    let error = parse_agent_report("status: PASS\nbuild_rc: abc").unwrap_err();
    assert!(error.contains("line 2"), "{error}");
    assert!(error.contains("build_rc"), "{error}");
}

#[test]
fn an_empty_value_fails_the_file() {
    let error = parse_agent_report("status:").unwrap_err();
    assert!(error.contains("line 1"), "{error}");
}

#[test]
fn a_line_without_any_separator_fails_the_file() {
    let error = parse_agent_report("status=PASS\ngarbage").unwrap_err();
    assert!(error.contains("line 2"), "{error}");
}

#[test]
fn a_gate_line_with_a_bare_token_fails_the_file() {
    let error = parse_agent_report("status=PASS garbage").unwrap_err();
    assert!(error.contains("line 1"), "{error}");
}

#[test]
fn an_empty_file_is_a_report_not_an_error() {
    let parsed = parse_agent_report("").unwrap();
    assert!(parsed.is_empty());
}

// --- triage ------------------------------------------------------

#[test]
fn a_timeout_re_dispatches() {
    assert_eq!(
        triage(&report(Some("TIMEOUT"), None, None, None, None)),
        TriageDecision::Redispatch {
            status: "TIMEOUT".to_string()
        }
    );
}

#[test]
fn a_timeout_with_no_output_raises_for_review() {
    assert_eq!(
        triage(&report(Some("TIMEOUT-NO-OUTPUT"), None, None, None, None)),
        TriageDecision::RaiseForReview {
            status: "TIMEOUT-NO-OUTPUT".to_string()
        }
    );
}

#[test]
fn a_timeout_with_no_output_outranks_every_other_signal() {
    let noisy = report(
        Some("TIMEOUT-NO-OUTPUT"),
        Some(1),
        Some(1),
        Some(1),
        Some(32),
    );
    assert_eq!(
        triage(&noisy),
        TriageDecision::RaiseForReview {
            status: "TIMEOUT-NO-OUTPUT".to_string()
        }
    );
}

#[test]
fn the_raise_for_review_decision_line_names_the_status_and_reason() {
    let report = report(Some("TIMEOUT-NO-OUTPUT"), None, None, None, None);
    let decision = triage(&report);
    let line = decision_line(&decision, &report);
    assert!(line.starts_with("RAISE-FOR-REVIEW:"), "{line}");
    assert!(line.contains("status=TIMEOUT-NO-OUTPUT"), "{line}");
    assert!(line.contains("no output"), "{line}");
}

#[test]
fn a_timeout_outranks_every_other_signal() {
    let noisy = report(Some("TIMEOUT"), Some(1), Some(1), Some(1), Some(32));
    assert_eq!(
        triage(&noisy),
        TriageDecision::Redispatch {
            status: "TIMEOUT".to_string()
        }
    );
}

#[test]
fn a_fmt_only_failure_formats_and_rechecks() {
    // The recorded verdict does not short-circuit the local check:
    // the pass formats and re-checks before judging (#4099).
    assert_eq!(
        triage(&report(None, Some(0), Some(0), Some(1), Some(32))),
        TriageDecision::FormatAndRecheck
    );
    // The status alone says the same thing.
    assert_eq!(
        triage(&report(Some("FMT-DIRTY"), Some(0), Some(0), None, None)),
        TriageDecision::FormatAndRecheck
    );
}

#[test]
fn a_fmt_failure_with_a_build_failure_still_holds() {
    // A formatter cannot repair a broken build: the same report
    // holds, with the fmt evidence first, as before.
    assert_eq!(
        triage(&report(None, Some(101), Some(0), Some(1), Some(32))),
        TriageDecision::Hold {
            reason: AgentHoldReason::Unformatted
        }
    );
    assert_eq!(
        triage(&report(Some("FMT-DIRTY"), Some(101), None, Some(1), None)),
        TriageDecision::Hold {
            reason: AgentHoldReason::Unformatted
        }
    );
}

#[test]
fn a_recorded_fmt_dirty_verdict_yields_to_the_local_recheck() {
    // AC3 (a): the run recorded FMT-DIRTY and the local re-check
    // passes without widening the changed-file set — the local
    // result is authoritative; the patch goes to the gate.
    let report = report(Some("FMT-DIRTY"), Some(0), Some(0), Some(1), Some(32));
    assert_eq!(triage(&report), TriageDecision::FormatAndRecheck);
    assert!(format_scope_check(&["src/a.rs".into()], &["src/a.rs".into()]).is_ok());
    assert_eq!(
        post_format_decision(0, Vec::new()),
        PostFormatDecision::Gate
    );
}

#[test]
fn a_recorded_fmt_dirty_verdict_holds_when_formatting_widens() {
    // AC3 (b): the run recorded FMT-DIRTY and the local re-check
    // passes, but the formatter touched a file the patch never
    // changed — the patch is held, naming the widened file.
    let report = report(Some("FMT-DIRTY"), Some(0), Some(0), Some(1), Some(32));
    assert_eq!(triage(&report), TriageDecision::FormatAndRecheck);
    let widened = format_scope_check(
        &["src/a.rs".into()],
        &["src/a.rs".into(), "src/b.rs".into()],
    )
    .unwrap_err();
    assert_eq!(widened, vec!["src/b.rs".to_string()]);
    assert_eq!(
        post_format_decision(0, widened),
        PostFormatDecision::HoldWidened {
            widened: vec!["src/b.rs".to_string()]
        }
    );
}

#[test]
fn a_local_fmt_failure_after_formatting_holds_still_dirty() {
    // The formatter did not clean the tree: the local re-check
    // still fails, and the patch is held.
    assert_eq!(
        post_format_decision(1, Vec::new()),
        PostFormatDecision::HoldStillDirty
    );
    // A still-dirty tree holds even if it also widened — the
    // formatter failure is the primary defect.
    assert_eq!(
        post_format_decision(1, vec!["src/b.rs".to_string()]),
        PostFormatDecision::HoldStillDirty
    );
}

#[test]
fn the_format_scope_check_is_a_set_inclusion() {
    let before = vec!["src/b.rs".to_string(), "src/a.rs".to_string()];
    // A subset in any order passes.
    assert!(format_scope_check(&before, &["src/a.rs".to_string()]).is_ok());
    // Duplicates on either side do not matter.
    assert!(format_scope_check(
        &before,
        &[
            "src/a.rs".to_string(),
            "src/a.rs".to_string(),
            "src/b.rs".to_string()
        ]
    )
    .is_ok());
    // Any file the formatter introduced fails, sorted and deduplicated.
    let widened = format_scope_check(
        &before,
        &["src/c.rs".into(), "src/c.rs".into(), "src/a.rs".into()],
    )
    .unwrap_err();
    assert_eq!(widened, vec!["src/c.rs".to_string()]);
    let widened = format_scope_check(&before, &["src/d.rs".into(), "src/c.rs".into()]).unwrap_err();
    assert_eq!(
        widened,
        vec!["src/c.rs".to_string(), "src/d.rs".to_string()]
    );
    // An empty after-set passes: formatting can only narrow.
    assert!(format_scope_check(&before, &[]).is_ok());
}

#[test]
fn a_zero_fmt_rc_does_not_hold() {
    assert_eq!(
        triage(&report(Some("PASS"), Some(0), Some(0), Some(0), None)),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentGreen
        }
    );
}

#[test]
fn a_build_failure_holds_unbuilt() {
    assert_eq!(
        triage(&report(None, Some(101), Some(0), Some(0), None)),
        TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt
        }
    );
    // The status alone says the same thing.
    assert_eq!(
        triage(&report(Some("BUILD-FAILED"), None, None, None, None)),
        TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt
        }
    );
    assert_eq!(
        triage(&report(
            Some("TESTS-DO-NOT-COMPILE"),
            None,
            None,
            None,
            None
        )),
        TriageDecision::Hold {
            reason: AgentHoldReason::Unbuilt
        }
    );
}

#[test]
fn fmt_outranks_build_when_both_failed() {
    let report = report(None, Some(101), None, Some(1), None);
    assert_eq!(
        triage(&report),
        TriageDecision::Hold {
            reason: AgentHoldReason::Unformatted
        }
    );
}

#[test]
fn a_test_failure_re_verifies_never_holds() {
    assert_eq!(
        triage(&report(None, Some(0), Some(101), Some(0), None)),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure { status: None }
        }
    );
    assert_eq!(
        triage(&report(Some("NEW-TEST-FAILURES"), None, None, None, None)),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentReportedTestFailure {
                status: Some("NEW-TEST-FAILURES".to_string())
            }
        }
    );
}

#[test]
fn no_baseline_gates_locally() {
    assert_eq!(
        triage(&report(Some("UNKNOWN-NO-BASELINE"), None, None, None, None)),
        TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline
        }
    );
}

#[test]
fn no_baseline_outranks_the_test_failure_it_implies() {
    let report = report(Some("UNKNOWN-NO-BASELINE"), None, Some(101), None, None);
    assert_eq!(
        triage(&report),
        TriageDecision::GateLocally {
            basis: GateBasis::NoBaseline
        }
    );
}

#[test]
fn green_gates_locally_to_confirm() {
    assert_eq!(
        triage(&report(Some("PASS"), Some(0), Some(0), Some(0), None)),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentGreen
        }
    );
}

#[test]
fn an_unconsumed_status_still_gates_locally() {
    // A fleet terminal status this triage does not act on: confirm
    // against the current main, the pass's default job.
    assert_eq!(
        triage(&report(Some("VERIFIED"), None, None, None, None)),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentGreen
        }
    );
}

#[test]
fn an_empty_report_fails_closed() {
    assert_eq!(
        triage(&AgentReport::default()),
        TriageDecision::GateLocally {
            basis: GateBasis::ReportUnreadable {
                detail: "status file is empty".to_string()
            }
        }
    );
}

// --- triage_report ------------------------------------------------

#[test]
fn triage_report_parses_and_triages() {
    assert_eq!(
        triage_report("status=TIMEOUT fmt_rc=1"),
        TriageDecision::Redispatch {
            status: "TIMEOUT".to_string()
        }
    );
    assert_eq!(
        triage_report("status: PASS\nbuild_rc: 0\ntest_rc: 0\nfmt_rc: 0"),
        TriageDecision::GateLocally {
            basis: GateBasis::AgentGreen
        }
    );
}

#[test]
fn an_unparseable_file_fails_closed_to_a_local_gate() {
    let decision = triage_report("build_rc: abc");
    match decision {
        TriageDecision::GateLocally {
            basis: GateBasis::ReportUnreadable { detail },
        } => {
            assert!(detail.contains("line 1"), "{detail}");
            assert!(detail.contains("build_rc"), "{detail}");
        }
        other => panic!("expected a failed-closed local gate, got {other:?}"),
    }
}

// --- rendering -----------------------------------------------------

#[test]
fn the_held_line_carries_the_agents_own_evidence() {
    let report = report(Some("TIMEOUT"), None, None, Some(1), Some(32));
    assert_eq!(
        held_line(&report, AgentHoldReason::Unformatted),
        "HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT"
    );
}

#[test]
fn the_held_line_degrades_gracefully_when_fields_are_missing() {
    assert_eq!(
        held_line(
            &report(None, None, None, Some(1), None),
            AgentHoldReason::Unformatted
        ),
        "HELD: agent reported fmt_rc=1"
    );
    assert_eq!(
        held_line(
            &report(None, None, None, None, Some(32)),
            AgentHoldReason::Unformatted
        ),
        "HELD: agent reported fmt_files=32"
    );
    assert_eq!(
        held_line(
            &report(None, None, None, None, None),
            AgentHoldReason::Unformatted
        ),
        "HELD: agent reported fmt failure"
    );
    assert_eq!(
        held_line(
            &report(Some("TESTS-DO-NOT-COMPILE"), None, None, None, None),
            AgentHoldReason::Unbuilt
        ),
        "HELD: agent reported build failure, status=TESTS-DO-NOT-COMPILE"
    );
    assert_eq!(
        held_line(
            &report(Some("BUILD-FAILED"), Some(101), None, None, None),
            AgentHoldReason::Unbuilt
        ),
        "HELD: agent reported build_rc=101, status=BUILD-FAILED"
    );
}

#[test]
fn the_decision_line_renders_every_branch() {
    let timeout = report(Some("TIMEOUT"), None, None, None, None);
    assert_eq!(
        decision_line(
            &TriageDecision::Redispatch {
                status: "TIMEOUT".to_string()
            },
            &timeout
        ),
        "RE-DISPATCH: agent reported status=TIMEOUT; the run never reached the gate"
    );

    let unformatted = report(Some("TIMEOUT"), None, None, Some(1), Some(32));
    assert_eq!(
        decision_line(
            &TriageDecision::Hold {
                reason: AgentHoldReason::Unformatted
            },
            &unformatted
        ),
        "HELD: agent reported fmt_rc=1 (32 files), status=TIMEOUT"
    );

    let fmt_dirty = report(Some("FMT-DIRTY"), Some(0), Some(0), Some(1), Some(32));
    assert_eq!(
        decision_line(&TriageDecision::FormatAndRecheck, &fmt_dirty),
        "FORMAT-AND-RECHECK: agent reported fmt_rc=1 (32 files), status=FMT-DIRTY; formatting and re-checking locally before judging"
    );
    assert_eq!(
        decision_line(
            &TriageDecision::FormatAndRecheck,
            &report(None, Some(0), Some(0), Some(1), None)
        ),
        "FORMAT-AND-RECHECK: agent reported fmt_rc=1; formatting and re-checking locally before judging"
    );

    assert_eq!(
        decision_line(
            &TriageDecision::GateLocally {
                basis: GateBasis::AgentReportedTestFailure {
                    status: Some("NEW-TEST-FAILURES".to_string())
                }
            },
            &report(Some("NEW-TEST-FAILURES"), None, None, None, None)
        ),
        "GATE-LOCALLY: agent reported test failure (status=NEW-TEST-FAILURES); re-verifying against current main"
    );
    assert_eq!(
        decision_line(
            &TriageDecision::GateLocally {
                basis: GateBasis::AgentReportedTestFailure { status: None }
            },
            &report(None, None, Some(101), None, None)
        ),
        "GATE-LOCALLY: agent reported test failure (test_rc=101); re-verifying against current main"
    );
    assert_eq!(
        decision_line(
            &TriageDecision::GateLocally {
                basis: GateBasis::NoBaseline
            },
            &report(Some("UNKNOWN-NO-BASELINE"), None, None, None, None)
        ),
        "GATE-LOCALLY: status=UNKNOWN-NO-BASELINE; no baseline to attribute the failures — gating against current main"
    );
    assert_eq!(
        decision_line(
            &TriageDecision::GateLocally {
                basis: GateBasis::AgentGreen
            },
            &report(Some("PASS"), Some(0), Some(0), Some(0), None)
        ),
        "GATE-LOCALLY: agent reported green; confirming against current main"
    );
    assert_eq!(
        decision_line(
            &TriageDecision::GateLocally {
                basis: GateBasis::ReportUnreadable {
                    detail: "line 2: build_rc expects an integer exit code, got abc"
                        .to_string()
                }
            },
            &AgentReport::default()
        ),
        "GATE-LOCALLY: status file unreadable (line 2: build_rc expects an integer exit code, got abc); gating against current main"
    );
}

#[test]
fn the_hold_reason_tokens_are_the_recorded_strings() {
    assert_eq!(
        AgentHoldReason::Unformatted.as_str(),
        "AGENT-REPORTED-UNFORMATTED"
    );
    assert_eq!(AgentHoldReason::Unbuilt.as_str(), "AGENT-REPORTED-UNBUILT");
}

// --- AGENT-NEVER-STARTED evidence (#4745) -------------------------------

#[test]
fn parses_the_never_started_evidence_keys() {
    let parsed =
        parse_agent_report("status=VERIFIED agent_rc=0 session_recorded=false assistant_turns=0")
            .unwrap();
    assert_eq!(parsed.session_recorded, Some(false));
    assert_eq!(parsed.assistant_turns, Some(0));
    let fleet = parse_agent_report("session_recorded: true\nassistant_turns: 7\n").unwrap();
    assert_eq!(fleet.session_recorded, Some(true));
    assert_eq!(fleet.assistant_turns, Some(7));
}

#[test]
fn an_unreadable_evidence_value_fails_the_file_naming_the_line() {
    let error = parse_agent_report("session_recorded=maybe").unwrap_err();
    assert!(error.contains("line 1"), "{error}");
    assert!(error.contains("session_recorded"), "{error}");
    let error = parse_agent_report("status=PASS\nassistant_turns=-1").unwrap_err();
    assert!(error.contains("line 2"), "{error}");
    assert!(error.contains("assistant_turns"), "{error}");
}

#[test]
fn evidence_alone_is_not_an_empty_report() {
    let parsed = parse_agent_report("assistant_turns=0").unwrap();
    assert!(!parsed.is_empty());
}

#[test]
fn zero_assistant_turns_outranks_a_verified_label() {
    let decision = triage_report("status=VERIFIED agent_rc=0 assistant_turns=0");
    assert_eq!(
        decision,
        TriageDecision::RaiseForReview {
            status: AGENT_NEVER_STARTED_STATUS.to_string()
        }
    );
}

#[test]
fn no_session_record_outranks_a_verified_label() {
    let decision = triage_report("status=VERIFIED agent_rc=0 session_recorded=false");
    assert_eq!(
        decision,
        TriageDecision::RaiseForReview {
            status: AGENT_NEVER_STARTED_STATUS.to_string()
        }
    );
}

#[test]
fn never_started_outranks_clean_and_dirty_stage_fields() {
    for stages in [
        "fmt_rc=1 build_rc=0",
        "build_rc=1 fmt_rc=0",
        "fmt_rc=0 build_rc=0 test_rc=0",
    ] {
        let body = format!("status=VERIFIED agent_rc=0 assistant_turns=0 {stages}");
        assert_eq!(
            triage_report(&body),
            TriageDecision::RaiseForReview {
                status: AGENT_NEVER_STARTED_STATUS.to_string()
            },
            "{body}"
        );
    }
}

#[test]
fn a_verified_report_without_evidence_still_gates_locally() {
    let decision = triage_report("status=VERIFIED agent_rc=0");
    assert!(
        matches!(
            decision,
            TriageDecision::GateLocally {
                basis: GateBasis::AgentGreen
            }
        ),
        "{decision:?}"
    );
}

#[test]
fn nonzero_exit_with_negative_evidence_is_not_never_started() {
    let decision = triage_report("status=VERIFIED agent_rc=1 assistant_turns=0");
    assert_ne!(
        decision,
        TriageDecision::RaiseForReview {
            status: AGENT_NEVER_STARTED_STATUS.to_string()
        }
    );
}

#[test]
fn a_signalled_run_with_evidence_stays_signalled() {
    let decision = triage_report("agent_rc=143 assistant_turns=0");
    assert!(
        matches!(decision, TriageDecision::Signalled { .. }),
        "{decision:?}"
    );
}

#[test]
fn the_never_started_decision_line_does_not_claim_a_timeout() {
    let body = "status=VERIFIED agent_rc=0 session_recorded=false";
    let report = parse_agent_report(body).unwrap();
    let line = decision_line(&triage(&report), &report);
    assert!(line.contains("AGENT-NEVER-STARTED"), "{line}");
    assert!(!line.contains("no output"), "{line}");
}
