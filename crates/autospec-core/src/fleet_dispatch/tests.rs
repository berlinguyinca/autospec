use super::*;

fn policy() -> FleetDispatchPolicy {
    FleetDispatchPolicy::default()
}

fn run(issue: &str, duration_secs: u64, transcript: &str) -> RunRecord {
    RunRecord {
        issue: issue.to_string(),
        duration_secs,
        transcript: transcript.to_string(),
        agent_rc: None,
        session_recorded: None,
        assistant_turns: None,
    }
}

fn run_with_rc(issue: &str, duration_secs: u64, transcript: &str, agent_rc: i32) -> RunRecord {
    RunRecord {
        issue: issue.to_string(),
        duration_secs,
        transcript: transcript.to_string(),
        agent_rc: Some(agent_rc),
        session_recorded: None,
        assistant_turns: None,
    }
}

/// The acceptance test from the issue: a bad-credential dispatch must be
/// `INFRA-FAIL`, never `NO-OUTPUT`, and must not consume an attempt.
#[test]
fn bad_credential_is_infra_fail_not_no_output() {
    let record = classify_run(
        &run(
            "51",
            40,
            "POST /v1/chat/completions -> 401 Unauthorized: invalid api key",
        ),
        &policy(),
    );
    assert_eq!(record.status, RunStatus::InfraFail);
    assert_ne!(record.status, RunStatus::NoOutput);
    assert!(!record.consumes_attempt);
    assert_eq!(
        record.failure_signature.as_deref(),
        Some("INFRA-FAIL/auth/401 unauthorized")
    );

    let mut ledger = AttemptLedger::new();
    ledger.apply(std::slice::from_ref(&record));
    assert_eq!(
        ledger.attempts("51"),
        0,
        "INFRA-FAIL must not consume an attempt"
    );
}

#[test]
fn short_run_below_floor_is_launch_fail_regardless_of_transcript() {
    // A 2-second run with an infra-looking transcript is still a launch
    // failure: it ended before it could have executed the spec.
    let record = classify_run(
        &run("52", 2, "connection refused while starting the session"),
        &policy(),
    );
    assert_eq!(record.status, RunStatus::LaunchFail);
    assert!(!record.consumes_attempt);
    assert_eq!(record.failure_signature, None);

    // A 2-second run that printed a healthy-looking transcript is the
    // same: below the floor, nothing ran.
    let healthy = classify_run(
        &run("53", 1, "loaded worktree, applied 3 edits, tests green"),
        &policy(),
    );
    assert_eq!(healthy.status, RunStatus::LaunchFail);
}

#[test]
fn floor_is_strictly_less_than() {
    let at_floor = classify_run(&run("7", 30, "did the work"), &policy());
    assert_eq!(at_floor.status, RunStatus::Ok);
    let below_floor = classify_run(&run("7", 29, "did the work"), &policy());
    assert_eq!(below_floor.status, RunStatus::LaunchFail);
}

#[test]
fn empty_transcript_at_duration_is_no_output() {
    let record = classify_run(&run("60", 120, "   \n  "), &policy());
    assert_eq!(record.status, RunStatus::NoOutput);
    assert!(
        record.consumes_attempt,
        "a run that ran but changed nothing does consume an attempt"
    );
    assert_eq!(record.failure_signature, None);
}

#[test]
fn long_run_with_output_is_ok() {
    let record = classify_run(
        &run("61", 300, "patch generated: 2 files changed"),
        &policy(),
    );
    assert_eq!(record.status, RunStatus::Ok);
    assert!(record.consumes_attempt);
}

#[test]
fn endpoint_failure_is_infra_fail() {
    let record = classify_run(
        &run(
            "54",
            90,
            "error: failed to connect to api.internal: connection refused",
        ),
        &policy(),
    );
    assert_eq!(record.status, RunStatus::InfraFail);
    assert_eq!(
        record.failure_signature.as_deref(),
        Some("INFRA-FAIL/endpoint/connection refused")
    );
}

#[test]
fn context_failure_is_infra_fail() {
    let record = classify_run(
        &run(
            "55",
            200,
            "API Error: prompt is too long: 210000 tokens > 200000 maximum",
        ),
        &policy(),
    );
    assert_eq!(record.status, RunStatus::InfraFail);
    assert_eq!(
        record.failure_signature.as_deref(),
        Some("INFRA-FAIL/context/prompt is too long")
    );
}

#[test]
fn auth_category_wins_over_later_categories() {
    let record = classify_run(
        &run(
            "56",
            100,
            "401 Unauthorized; also connection refused on retry",
        ),
        &policy(),
    );
    assert_eq!(record.status, RunStatus::InfraFail);
    assert!(record
        .failure_signature
        .as_deref()
        .unwrap()
        .starts_with("INFRA-FAIL/auth/"));
}

#[test]
fn short_transcript_is_quoted_verbatim() {
    let transcript = "401 Unauthorized: bad token\nat fetch (line 12)\ttabbed";
    let record = classify_run(&run("57", 45, transcript), &policy());
    assert_eq!(record.quoted_transcript.as_deref(), Some(transcript));
    assert_eq!(record.transcript_bytes, transcript.len());
}

#[test]
fn long_transcript_is_not_quoted_but_counted() {
    let transcript = "x".repeat(5000);
    let record = classify_run(&run("58", 60, &transcript), &policy());
    assert_eq!(record.quoted_transcript, None);
    assert_eq!(record.transcript_bytes, 5000);
}

#[test]
fn quote_boundary_is_inclusive() {
    let transcript = "y".repeat(DEFAULT_TRANSCRIPT_QUOTE_BYTES);
    let at_threshold = classify_run(&run("59", 60, &transcript), &policy());
    assert_eq!(
        at_threshold.quoted_transcript.as_deref(),
        Some(transcript.as_str())
    );

    let over = classify_run(
        &run("59", 60, &"y".repeat(DEFAULT_TRANSCRIPT_QUOTE_BYTES + 1)),
        &policy(),
    );
    assert_eq!(over.quoted_transcript, None);
}

#[test]
fn tsv_line_escapes_control_characters() {
    let record = classify_run(&run("62", 45, "line one\nline two\ttabbed"), &policy());
    let line = record.tsv_line();
    let columns = line.split('\t').count();
    assert_eq!(
        columns, 7,
        "a quoted transcript must not add TSV columns: {line:?}"
    );
    assert!(line.contains("line one\\nline two\\ttabbed"));
}

#[test]
fn batch_summary_counts_and_raises_fault_on_repetition() {
    let records = vec![
        classify_run(&run("51", 40, "401 Unauthorized"), &policy()),
        classify_run(&run("52", 38, "401 Unauthorized"), &policy()),
        classify_run(
            &run("53", 41, "401 Unauthorized: invalid api key"),
            &policy(),
        ),
        classify_run(&run("60", 120, ""), &policy()),
        classify_run(&run("61", 300, "did the work"), &policy()),
    ];
    let summary = summarize_batch(&records, &policy());
    assert_eq!(
        (
            summary.ok,
            summary.no_output,
            summary.infra_fail,
            summary.launch_fail
        ),
        (1, 1, 3, 0)
    );
    assert_eq!(summary.total(), 5);
    assert_eq!(summary.faults.len(), 1);
    let fault = &summary.faults[0];
    assert_eq!(fault.code, REPEATED_IDENTICAL_FAILURE);
    assert_eq!(fault.status, RunStatus::InfraFail);
    assert_eq!(fault.count, 3);
    assert_eq!(
        fault.issues,
        vec!["51".to_string(), "52".to_string(), "53".to_string()]
    );

    let lines = summary.lines();
    assert_eq!(
        lines[0],
        "BATCH-SUMMARY ok=1 no-output=1 infra-fail=3 launch-fail=0 total=5"
    );
    assert!(lines[1].starts_with("FLEET-FAULT code=REPEATED_IDENTICAL_FAILURE"));
}

#[test]
fn below_threshold_repetition_is_not_a_fault() {
    let records = vec![
        classify_run(&run("51", 40, "401 Unauthorized"), &policy()),
        classify_run(&run("52", 38, "401 Unauthorized"), &policy()),
    ];
    let summary = summarize_batch(&records, &policy());
    assert!(summary.faults.is_empty());
}

#[test]
fn distinct_signatures_do_not_group() {
    let records = vec![
        classify_run(&run("51", 40, "401 Unauthorized"), &policy()),
        classify_run(&run("52", 38, "connection refused"), &policy()),
        classify_run(&run("53", 41, "prompt is too long"), &policy()),
    ];
    let summary = summarize_batch(&records, &policy());
    assert_eq!(summary.infra_fail, 3);
    assert!(
        summary.faults.is_empty(),
        "three different infrastructure failures are not one repeated failure"
    );
}

/// The zero-output contradiction from issue #3749: an agent that exited
/// 0 while writing nothing must be flagged, not read as a quiet success.
#[test]
fn zero_output_with_rc_zero_is_flagged_contradiction() {
    let record = classify_run(&run_with_rc("71", 300, "", 0), &policy());
    assert_eq!(record.status, RunStatus::NoOutput);
    assert!(record.consumes_attempt);
    assert_eq!(record.failure_signature.as_deref(), Some(NO_OUTPUT_RC_ZERO));
}

/// A nonzero exit with zero output is a plausible failure and stays a
/// plain unsigned NO-OUTPUT.
#[test]
fn zero_output_with_nonzero_rc_is_not_flagged() {
    for rc in [1, 2, 127] {
        let record = classify_run(&run_with_rc("72", 300, "", rc), &policy());
        assert_eq!(record.status, RunStatus::NoOutput);
        assert_eq!(
            record.failure_signature, None,
            "rc={rc} with no output is a plausible failure, not a contradiction"
        );
    }
}

/// Without a reported exit code there is no contradiction to flag.
#[test]
fn zero_output_without_exit_code_is_not_flagged() {
    let record = classify_run(&run("73", 300, "   "), &policy());
    assert_eq!(record.status, RunStatus::NoOutput);
    assert_eq!(record.failure_signature, None);
}

/// N identical zero-output-rc-zero runs in one batch raise the same
/// fleet fault as N identical infrastructure failures.
#[test]
fn repeated_zero_output_rc_zero_is_a_fleet_fault() {
    let records = (1..=3)
        .map(|n| classify_run(&run_with_rc(&n.to_string(), 300, "", 0), &policy()))
        .collect::<Vec<_>>();
    let summary = summarize_batch(&records, &policy());
    assert_eq!(summary.no_output, 3);
    assert_eq!(summary.faults.len(), 1);
    let fault = &summary.faults[0];
    assert_eq!(fault.code, REPEATED_IDENTICAL_FAILURE);
    assert_eq!(fault.status, RunStatus::NoOutput);
    assert_eq!(fault.signature, NO_OUTPUT_RC_ZERO);
    assert_eq!(fault.count, 3);
    assert!(summary.lines()[1].contains("signature=NO-OUTPUT-RC-ZERO"));
}

/// rc-zero contradictions and bare zero-output failures are distinct
/// identities and must not group into one fault.
#[test]
fn rc_zero_and_bare_no_output_do_not_mingle() {
    let records = vec![
        classify_run(&run_with_rc("81", 300, "", 0), &policy()),
        classify_run(&run("82", 300, ""), &policy()),
        classify_run(&run("83", 300, "  \n "), &policy()),
    ];
    let summary = summarize_batch(&records, &policy());
    assert_eq!(summary.no_output, 3);
    assert!(
        summary.faults.is_empty(),
        "one signed plus two unsigned no-output records are two distinct identities of 1 and 2, neither at threshold"
    );
}

#[test]
fn idle_subfleets_are_reported() {
    let states = vec![
        SubfleetState {
            name: "gw-issue-51-53".to_string(),
            running_agents: 0,
            open_eligible: 7,
        },
        SubfleetState {
            name: "busy".to_string(),
            running_agents: 2,
            open_eligible: 9,
        },
        SubfleetState {
            name: "empty".to_string(),
            running_agents: 0,
            open_eligible: 0,
        },
    ];
    let lines = idle_subfleet_lines(&states);
    assert_eq!(lines.len(), 1);
    assert!(lines[0]
        .starts_with("SUBFLEET-IDLE subfleet=gw-issue-51-53 running-agents=0 open-eligible=7"));
}

#[test]
fn attempt_ledger_counts_only_consumed_runs() {
    let records = vec![
        classify_run(&run("51", 2, "401 Unauthorized"), &policy()), // LAUNCH-FAIL: no attempt
        classify_run(&run("51", 40, "401 Unauthorized"), &policy()), // INFRA-FAIL: no attempt
        classify_run(&run("51", 120, ""), &policy()),               // NO-OUTPUT: attempt
        classify_run(&run("52", 300, "did it"), &policy()),         // OK: attempt
    ];
    let mut ledger = AttemptLedger::new();
    ledger.apply(&records);
    assert_eq!(ledger.attempts("51"), 1);
    assert_eq!(ledger.attempts("52"), 1);
    assert_eq!(ledger.attempts("99"), 0);
}

#[test]
fn status_serialization_uses_hyphenated_strings() {
    let record = classify_run(&run("51", 40, "401 Unauthorized"), &policy());
    let json = serde_json::to_string(&record).expect("record serializes");
    assert!(json.contains("\"INFRA-FAIL\""), "got {json}");
    assert!(json.contains("\"INFRA-FAIL/auth/401 unauthorized\""));

    let roundtrip: RunStatusRecord = serde_json::from_str(&json).expect("record deserializes");
    assert_eq!(roundtrip, record);

    // The underscored legacy form still parses.
    assert_eq!(RunStatus::parse("NO_OUTPUT"), Some(RunStatus::NoOutput));
    assert_eq!(RunStatus::parse("bogus"), None);
}

#[test]
fn policy_requires_positive_knobs() {
    assert!(FleetDispatchPolicy::new(30, 4096, 3).is_some());
    assert!(FleetDispatchPolicy::new(0, 4096, 3).is_none());
    assert!(FleetDispatchPolicy::new(30, 0, 3).is_none());
    assert!(FleetDispatchPolicy::new(30, 4096, 0).is_none());
}

#[test]
fn signature_priority_is_first_listed_match() {
    // "context length exceeded" contains "context length"; the more
    // specific signature must win.
    let found = infra_signature("Error: context length exceeded for model").unwrap();
    assert_eq!(found, (InfraCategory::Context, "context length exceeded"));
}

#[test]
fn infra_match_is_case_insensitive() {
    assert_eq!(
        infra_signature("HTTP 401 UNAUTHORIZED from the gateway").map(|(c, _)| c),
        Some(InfraCategory::Auth)
    );
}

// ---- issue #4211: reconciler memory ----

#[test]
fn consecutive_failures_suspend_at_threshold() {
    let mut memory = ReconcilerMemory::new(3);
    // The first two failures are recorded but do not suspend.
    assert_eq!(
        memory.record(
            "glm-5.3-flash",
            &StartOutcome::Failed {
                reason: "unknown model".to_string(),
            }
        ),
        RecordEffect::Submitted
    );
    assert_eq!(
        memory.record(
            "glm-5.3-flash",
            &StartOutcome::Failed {
                reason: "unknown model".to_string(),
            }
        ),
        RecordEffect::Submitted
    );
    assert!(!memory.is_suspended("glm-5.3-flash"));

    // The third failure crosses the threshold and suspends.
    let effect = memory.record(
        "glm-5.3-flash",
        &StartOutcome::Failed {
            reason: "unknown model".to_string(),
        },
    );
    assert_eq!(
        effect,
        RecordEffect::Suspended {
            suspended: SuspendedClaim {
                claim: "glm-5.3-flash".to_string(),
                consecutive_failures: 3,
                last_failure: "unknown model".to_string(),
            }
        }
    );
    assert!(memory.is_suspended("glm-5.3-flash"));
    assert_eq!(memory.consecutive_failures("glm-5.3-flash"), 3);
    assert_eq!(memory.last_failure("glm-5.3-flash"), Some("unknown model"));
}

/// The incident at full scale: 102 identical start failures. The counter
/// keeps counting past the threshold (the reconciler is expected to stop
/// submitting once `is_suspended` is true), the claim stays suspended,
/// and the status line renders as a sentence, not `want=1 up=0`.
#[test]
fn the_incident_102_identical_failures_stays_suspended_and_is_reported() {
    let mut memory = ReconcilerMemory::new(DEFAULT_CONSECUTIVE_FAILURE_THRESHOLD);
    let mut suspensions = Vec::new();
    for _ in 0..102 {
        if let RecordEffect::Suspended { suspended } = memory.record(
            "glm-5.3-flash",
            &StartOutcome::Failed {
                reason: "unknown model".to_string(),
            },
        ) {
            suspensions.push(suspended);
        }
    }
    // Only the first crossing of the threshold is reported as new.
    assert_eq!(suspensions.len(), 1);
    assert!(memory.is_suspended("glm-5.3-flash"));
    assert_eq!(memory.consecutive_failures("glm-5.3-flash"), 102);
    assert_eq!(memory.last_failure("glm-5.3-flash"), Some("unknown model"));

    let line = memory.status_line("glm-5.3-flash", 1, 0, 0);
    assert!(
        line.starts_with(
            "want=1 up=0 pending=0 (102 consecutive start failures, last: unknown model"
        ),
        "the never-converged line must carry the count and the last failure: {line}"
    );
    assert!(line.contains("(suspended)"), "{line}");
}

#[test]
fn never_converged_is_distinct_from_converging() {
    let mut memory = ReconcilerMemory::new(3);
    // No failures yet: a plain converging line.
    assert_eq!(
        memory.status_line("glm-5.3-flash", 1, 0, 0),
        "want=1 up=0 pending=0"
    );

    memory.record(
        "glm-5.3-flash",
        &StartOutcome::Failed {
            reason: "unknown model".to_string(),
        },
    );
    let line = memory.status_line("glm-5.3-flash", 1, 0, 0);
    assert!(
        line.starts_with("want=1 up=0 pending=0 (1 consecutive start failures"),
        "{line}"
    );
    assert_ne!(line, "want=1 up=0 pending=0");
}

#[test]
fn started_resets_the_counter_and_releases_a_suspension() {
    let mut memory = ReconcilerMemory::new(2);
    memory.record(
        "claim-1",
        &StartOutcome::Failed {
            reason: "bad key".to_string(),
        },
    );
    memory.record(
        "claim-1",
        &StartOutcome::Failed {
            reason: "bad key".to_string(),
        },
    );
    assert!(memory.is_suspended("claim-1"));

    // The worker finally comes up: the counter resets and the suspension
    // is released because the claim converged after all.
    assert_eq!(
        memory.record("claim-1", &StartOutcome::Started),
        RecordEffect::Submitted
    );
    assert!(!memory.is_suspended("claim-1"));
    assert_eq!(memory.consecutive_failures("claim-1"), 0);
    assert_eq!(memory.last_failure("claim-1"), None);
    assert_eq!(
        memory.status_line("claim-1", 1, 1, 0),
        "want=1 up=1 pending=0"
    );
}

#[test]
fn cancelled_does_not_advance_the_counter() {
    let mut memory = ReconcilerMemory::new(3);
    memory.record(
        "claim-2",
        &StartOutcome::Failed {
            reason: "unknown model".to_string(),
        },
    );
    // A cancelled submission is neither a start nor a failure: it must
    // not push the claim over the edge.
    assert_eq!(
        memory.record("claim-2", &StartOutcome::Cancelled),
        RecordEffect::Submitted
    );
    memory.record(
        "claim-2",
        &StartOutcome::Failed {
            reason: "unknown model".to_string(),
        },
    );
    assert_eq!(memory.consecutive_failures("claim-2"), 2);
    assert!(!memory.is_suspended("claim-2"));
}

#[test]
fn clear_releases_a_suspended_claim() {
    let mut memory = ReconcilerMemory::new(3);
    memory.record(
        "claim-3",
        &StartOutcome::Failed {
            reason: "unknown model".to_string(),
        },
    );
    memory.record(
        "claim-3",
        &StartOutcome::Failed {
            reason: "unknown model".to_string(),
        },
    );
    memory.record(
        "claim-3",
        &StartOutcome::Failed {
            reason: "unknown model".to_string(),
        },
    );
    assert!(memory.is_suspended("claim-3"));
    assert!(memory.clear("claim-3"));
    assert!(!memory.is_suspended("claim-3"));
    assert_eq!(memory.consecutive_failures("claim-3"), 0);
    // Clearing an unsuspended claim is a no-op that reports no release.
    assert!(!memory.clear("claim-3"));
}

#[test]
fn start_outcome_maps_classified_run_statuses() {
    let policy = policy();
    // A cheap death below the floor is a start failure.
    let launch = classify_run(&run("51", 2, "unknown model"), &policy);
    assert_eq!(
        launch.start_outcome(),
        StartOutcome::Failed {
            reason: "LAUNCH-FAIL".to_string(),
        }
    );
    // A bad credential is a start failure with its signature as the reason.
    let infra = classify_run(&run("52", 40, "401 Unauthorized"), &policy);
    assert_eq!(
        infra.start_outcome(),
        StartOutcome::Failed {
            reason: "INFRA-FAIL/auth/401 unauthorized".to_string(),
        }
    );
    // A run that happened — even a no-output one — is a start.
    assert_eq!(
        classify_run(&run("53", 300, "did it"), &policy).start_outcome(),
        StartOutcome::Started
    );
    assert_eq!(
        classify_run(&run("54", 300, ""), &policy).start_outcome(),
        StartOutcome::Started
    );
}

/// The rate threshold (per-claim consecutive failures) is what makes the
/// per-attempt cost floor ([`FleetDispatchPolicy::duration_floor_secs`])
/// safe: a batch of cheap `LAUNCH-FAIL` runs that never consume an
/// attempt still accumulates into a suspension.
#[test]
fn cheap_launch_failures_still_accumulate_into_a_suspension() {
    let policy = FleetDispatchPolicy::new(30, 4096, 3).unwrap();
    // Three runs that die in under a second — the exact cheapness that
    // made the incident survivable, and invisible to an attempt ledger
    // because LAUNCH-FAIL never consumes an attempt.
    let runs = (1..=3)
        .map(|_| classify_run(&run("glm-5.3-flash", 0, "unknown model"), &policy))
        .collect::<Vec<_>>();

    // The attempt ledger sees nothing — LAUNCH-FAIL never consumes an
    // attempt, so the per-attempt cost floor alone would miss this.
    let mut attempts = AttemptLedger::new();
    attempts.apply(&runs);
    assert_eq!(attempts.attempts("glm-5.3-flash"), 0);

    // But the reconciler's per-claim rate threshold accumulates the
    // cheap failures into a suspension and reports it.
    let mut memory = ReconcilerMemory::new(DEFAULT_CONSECUTIVE_FAILURE_THRESHOLD);
    let suspensions = memory.apply_runs(&runs);
    assert_eq!(suspensions.len(), 1);
    assert!(memory.is_suspended("glm-5.3-flash"));
    assert_eq!(memory.suspended_lines().len(), 1);
    let line = &memory.suspended_lines()[0];
    assert!(!line.contains('\n'));
    assert!(
        line.starts_with("CLAIM-SUSPENDED claim=glm-5.3-flash"),
        "{line}"
    );
}

#[test]
fn apply_runs_reports_only_newly_suspended_claims() {
    let policy = policy();
    let mut memory = ReconcilerMemory::new(3);
    // Claim A accumulates two failures across batches, B one.
    memory.apply_runs(&[
        classify_run(&run("A", 0, "boom"), &policy),
        classify_run(&run("B", 0, "boom"), &policy),
    ]);
    // A reaches three here (two more failures) and is reported; B stays
    // below.
    let newly = memory.apply_runs(&[
        classify_run(&run("A", 0, "boom"), &policy),
        classify_run(&run("A", 0, "boom"), &policy),
    ]);
    assert_eq!(newly.len(), 1);
    assert_eq!(newly[0].claim, "A");
    assert!(memory.is_suspended("A"));
    assert!(!memory.is_suspended("B"));
}

#[test]
fn zero_threshold_falls_back_to_default() {
    assert_eq!(
        ReconcilerMemory::new(0).threshold(),
        DEFAULT_CONSECUTIVE_FAILURE_THRESHOLD
    );
    assert_eq!(ReconcilerMemory::new(5).threshold(), 5);
}

fn evidence_run(
    agent_rc: i32,
    session_recorded: Option<bool>,
    assistant_turns: Option<u32>,
    transcript: &str,
) -> RunRecord {
    RunRecord {
        session_recorded,
        assistant_turns,
        ..run_with_rc("90", 300, transcript, agent_rc)
    }
}

/// Exit 0 with no session record never ran the agent, whatever it printed.
#[test]
fn exit_zero_without_session_is_agent_never_started() {
    let record = classify_run(
        &evidence_run(0, Some(false), None, "a banner and a goodbye"),
        &policy(),
    );
    assert_eq!(record.status, RunStatus::AgentNeverStarted);
    assert_eq!(record.status.as_str(), AGENT_NEVER_STARTED_STATUS);
    assert_eq!(AGENT_NEVER_STARTED_STATUS, "AGENT-NEVER-STARTED");
    assert!(!record.consumes_attempt);
}

/// Exit 0 with zero completed assistant turns never started either, and
/// must not burn an attempt.
#[test]
fn exit_zero_with_zero_assistant_turns_is_agent_never_started() {
    let record = classify_run(&evidence_run(0, None, Some(0), ""), &policy());
    assert_eq!(record.status, RunStatus::AgentNeverStarted);
    assert!(!record.consumes_attempt);
    assert_eq!(
        record.failure_signature.as_deref(),
        Some(AGENT_NEVER_STARTED_STATUS)
    );
    assert!(record.line().contains("no-attempt"));
}

/// A nonzero exit is a different failure; the evidence rule is exit-0 only.
#[test]
fn nonzero_exit_is_never_agent_never_started() {
    for rc in [1, 2, 143] {
        let record = classify_run(&evidence_run(rc, Some(false), Some(0), "died"), &policy());
        assert_ne!(record.status, RunStatus::AgentNeverStarted, "rc={rc}");
    }
}

/// Positive evidence (a session and turns) is not a never-started run.
#[test]
fn exit_zero_with_session_and_turns_is_not_never_started() {
    let record = classify_run(&evidence_run(0, Some(true), Some(3), "did work"), &policy());
    assert_eq!(record.status, RunStatus::Ok);
    assert!(record.consumes_attempt);
}

/// Legacy records carry neither new field and keep their old signature.
#[test]
fn legacy_record_without_evidence_keeps_no_output_rc_zero() {
    let record = classify_run(&run_with_rc("91", 300, "", 0), &policy());
    assert_eq!(record.status, RunStatus::NoOutput);
    assert_eq!(record.failure_signature.as_deref(), Some(NO_OUTPUT_RC_ZERO));
    let legacy: RunRecord =
        serde_json::from_str(r#"{"issue":"1","duration_secs":300,"transcript":"","agent_rc":0}"#)
            .expect("legacy record without evidence fields deserializes");
    assert_eq!(legacy.session_recorded, None);
    assert_eq!(legacy.assistant_turns, None);
}

/// Infrastructure failures and the duration floor outrank the new status.
#[test]
fn infra_and_launch_fail_outrank_never_started() {
    let infra = classify_run(
        &evidence_run(0, None, Some(0), "401 Unauthorized: invalid api key"),
        &policy(),
    );
    assert_eq!(infra.status, RunStatus::InfraFail);
    let mut short = evidence_run(0, Some(false), Some(0), "");
    short.duration_secs = 2;
    assert_eq!(
        classify_run(&short, &policy()).status,
        RunStatus::LaunchFail
    );
}

/// A never-started run is a start failure for the reconciler.
#[test]
fn never_started_start_outcome_is_failed() {
    let record = classify_run(&evidence_run(0, Some(false), None, "x"), &policy());
    assert_eq!(
        record.start_outcome(),
        StartOutcome::Failed {
            reason: AGENT_NEVER_STARTED_STATUS.to_string(),
        }
    );
}

/// The pure rule, including its non-triggering shapes.
#[test]
fn never_started_rule_truth_table() {
    assert!(never_started(Some(0), Some(false), None));
    assert!(never_started(Some(0), None, Some(0)));
    assert!(never_started(Some(0), Some(true), Some(0)));
    assert!(!never_started(Some(0), Some(true), Some(2)));
    assert!(!never_started(Some(0), None, None));
    assert!(!never_started(Some(1), Some(false), Some(0)));
    assert!(!never_started(None, Some(false), Some(0)));
}

/// The batch summary counts the new status instead of dropping it.
#[test]
fn batch_summary_counts_never_started() {
    let records: Vec<_> = (0..3)
        .map(|_| classify_run(&evidence_run(0, Some(false), None, "x"), &policy()))
        .collect();
    let summary = summarize_batch(&records, &policy());
    assert_eq!(summary.never_started, 3);
    assert_eq!(summary.total(), 3);
    assert!(summary.lines()[0].contains("never-started=3"));
    assert_eq!(summary.faults.len(), 1);
    assert_eq!(summary.faults[0].status, RunStatus::AgentNeverStarted);
}
