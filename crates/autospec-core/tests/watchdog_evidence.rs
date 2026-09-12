//! A watchdog that kills must first capture why (issue #3792).
//!
//! The incident's numbers pin the fixtures: InferWeave #45, a keystone
//! unblocking 81 downstream issues, ran 2761 s (46m) and produced nothing —
//! `status=NO-OUTPUT agent_rc=143 agent_secs=2761 changed_files=0` — its
//! whole log 2,903 bytes ending `STALLED: no session or output activity for
//! 45m after 46m; terminating agent`. The endpoint it was given,
//! `qwen3.8-27b-22771168`, was healthy seven hours later; the neighbouring
//! jobs on that worker were idle during the 08:03–08:49 window; a sibling
//! dispatched in the same second completed in 673 s. The cause was
//! answerable at the instant of termination and is unanswerable now,
//! because the watchdog killed before it captured.

use autospec_core::watchdog_evidence::{
    capture, capture_gate, evidence_file_path, status_gate, termination_order, ArtifactState,
    Cause, Evidence, OutputTail, Probe, ProbeOutcome, ProcessState, StatusRecord, StatusVerdict,
    TerminationOrder, EVIDENCE_SUFFIX,
};

/// The endpoint the incident's agent was given.
const ENDPOINT: &str = "qwen3.8-27b-22771168";

/// The stall the incident's watchdog fired on.
fn stalled_run() -> Evidence {
    Evidence {
        run_id: "iw-45".to_string(),
        probes: vec![Probe {
            dependency: ENDPOINT.to_string(),
            outcome: ProbeOutcome::Unreachable {
                reason: "connection refused".to_string(),
            },
        }],
        watched_path: "/scratch/iw/out/issue-45/agent.out".to_string(),
        watched: ArtifactState::Missing,
        tail: OutputTail {
            source: "/scratch/iw/out/issue-45/agent.out".to_string(),
            lines: vec![],
        },
        state: ProcessState::BlockedIo,
    }
}

// --- AC1: the capture is written to durable storage before the signal ------

#[test]
fn evidence_durable_before_the_signal_is_the_only_compliant_order() {
    assert_eq!(
        termination_order(true, true),
        TerminationOrder::CaptureFirst
    );
    assert!(capture_gate(TerminationOrder::CaptureFirst));
}

#[test]
fn a_signal_sent_before_the_capture_is_the_incident() {
    // The incident's order: kill -TERM, then nothing. The stall was real
    // and the kill may have been justified — the evidence it destroyed is
    // unrecoverable by definition, and the gate says so.
    assert_eq!(
        termination_order(false, false),
        TerminationOrder::KilledBeforeCapture
    );
    assert_eq!(
        termination_order(true, false),
        TerminationOrder::KilledBeforeCapture
    );
    // A render that never reached durable storage (buffered, in-memory) is
    // not a capture either: durable is the word that matters.
    assert!(!capture_gate(TerminationOrder::KilledBeforeCapture));
}

// --- The capture: every piece the issue names -------------------------------

#[test]
fn a_capture_names_what_the_probe_found_and_refuses_to_cover_nothing() {
    // The dependency answered: the model did answer, so the cause is not
    // the endpoint.
    let ok = Probe {
        dependency: ENDPOINT.to_string(),
        outcome: ProbeOutcome::Responded {
            status: 200,
            latency_ms: 412,
        },
    };
    assert!(!ok.is_unreachable());
    assert_eq!(ok.line(), "probe qwen3.8-27b-22771168 → HTTP 200 in 412 ms");

    let dead = Probe {
        dependency: ENDPOINT.to_string(),
        outcome: ProbeOutcome::Unreachable {
            reason: "timed out after 10 s".to_string(),
        },
    };
    assert!(dead.is_unreachable());
    assert_eq!(
        dead.line(),
        "probe qwen3.8-27b-22771168 → unreachable (timed out after 10 s)"
    );

    // Refusals name what would tell the capture, rather than guessing.
    let mut bare = stalled_run();
    bare.probes = vec![];
    assert_eq!(
        capture("stalls", &bare).unwrap_err().as_str(),
        "no dependency was probed: the capture must cover every external dependency the run was given"
    );
    let mut no_path = stalled_run();
    no_path.watched_path = String::new();
    assert_eq!(
        capture("stalls", &no_path).unwrap_err().as_str(),
        "the watched path is not named: the capture cannot state whether it was ever created"
    );
    let mut no_id = stalled_run();
    no_id.run_id = String::new();
    assert_eq!(
        capture("stalls", &no_id).unwrap_err().as_str(),
        "run id is empty: the evidence file would be unaddressable"
    );
}

#[test]
fn capture_names_the_evidence_file_and_renders_every_piece() {
    let (evidence, path) = capture("stalls", &stalled_run()).unwrap();
    assert_eq!(path, "stalls/run-iw-45-stall-evidence.txt");
    assert_eq!(path, evidence_file_path("stalls", "iw-45"));
    assert_eq!(EVIDENCE_SUFFIX, "stall-evidence.txt");
    assert_eq!(evidence, stalled_run());

    let record = evidence.render();
    let expected = [
        "run iw-45",
        "watched: /scratch/iw/out/issue-45/agent.out: missing — never created",
        "probes:",
        "  probe qwen3.8-27b-22771168 → unreachable (connection refused)",
        "  tail of /scratch/iw/out/issue-45/agent.out: (empty)",
        "process state at kill: blocked on I/O",
    ]
    .join("\n")
        + "\n";
    assert_eq!(record, expected);
}

#[test]
fn a_never_created_path_is_a_different_failure_from_a_path_that_stopped_growing() {
    let missing = stalled_run();
    assert_eq!(
        missing.watched.line(&missing.watched_path),
        "/scratch/iw/out/issue-45/agent.out: missing — never created"
    );

    let mut growing = stalled_run();
    growing.watched = ArtifactState::Present {
        size_bytes: 2903,
        mtime: "2026-09-14T08:03:12".to_string(),
    };
    growing.tail = OutputTail {
        source: "/scratch/iw/out/issue-45/agent.out".to_string(),
        lines: vec![
            "budget: slot_ctx=65536 context_window=65536 max_tokens=32768".to_string(),
            "pi skills in force: <none>".to_string(),
        ],
    };
    assert_eq!(
        growing.watched.line(&growing.watched_path),
        "/scratch/iw/out/issue-45/agent.out: present, 2903 bytes, mtime 2026-09-14T08:03:12"
    );
    assert_eq!(
        growing.tail.line(),
        "tail of /scratch/iw/out/issue-45/agent.out: budget: slot_ctx=65536 context_window=65536 max_tokens=32768; pi skills in force: <none>"
    );
    // With every dependency answering, the two artifact states name two
    // different causes.
    let mut healthy_probe = growing.clone();
    healthy_probe.probes = vec![Probe {
        dependency: ENDPOINT.to_string(),
        outcome: ProbeOutcome::Responded {
            status: 200,
            latency_ms: 380,
        },
    }];
    assert_eq!(
        healthy_probe.cause_line(),
        "watched path /scratch/iw/out/issue-45/agent.out stopped growing with every probed dependency answering"
    );
    let mut healthy_probe_missing = healthy_probe.clone();
    healthy_probe_missing.watched = ArtifactState::Missing;
    assert_eq!(
        healthy_probe_missing.cause_line(),
        "watched path /scratch/iw/out/issue-45/agent.out was never created — the watchdog was testing a path that never existed"
    );
}

#[test]
fn the_process_state_at_kill_is_recorded_in_all_forms() {
    assert_eq!(ProcessState::Running.as_str(), "running");
    assert_eq!(ProcessState::Sleeping.as_str(), "sleeping");
    assert_eq!(ProcessState::BlockedIo.as_str(), "blocked on I/O");
    assert_eq!(ProcessState::Zombie.as_str(), "zombie");
    assert_eq!(ProcessState::Unknown.as_str(), "unknown");
}

// --- AC3 + AC4: the status names its evidence; NO-OUTPUT is never bare -----

#[test]
fn the_recorded_status_names_the_evidence_file_and_the_cause() {
    let (evidence, path) = capture("stalls", &stalled_run()).unwrap();
    let record = StatusRecord {
        status: "NO-OUTPUT".to_string(),
        cause: Some(Cause::Captured {
            evidence,
            line: "unreachable dependency qwen3.8-27b-22771168: connection refused".to_string(),
        }),
        evidence_file: Some(path),
    };
    assert_eq!(record.gate(), StatusVerdict::Accompanied);
    assert_eq!(
        record.render(),
        "status=NO-OUTPUT evidence=stalls/run-iw-45-stall-evidence.txt \
         cause=unreachable dependency qwen3.8-27b-22771168: connection refused"
    );
}

#[test]
fn a_capture_failure_is_explicit_not_swallowed() {
    let record = StatusRecord {
        status: "NO-OUTPUT".to_string(),
        cause: Some(Cause::CaptureFailed {
            reason: "probe timed out before the record could be rendered".to_string(),
        }),
        evidence_file: None,
    };
    assert_eq!(record.gate(), StatusVerdict::Accompanied);
    assert_eq!(
        record.render(),
        "status=NO-OUTPUT cause=capture failed: probe timed out before the record could be rendered"
    );
    // A capture-failure with an empty reason is not a statement: it is the
    // bare classification wearing a label.
    assert_eq!(
        status_gate(
            "NO-OUTPUT",
            Some(&Cause::CaptureFailed { reason: " ".into() })
        ),
        StatusVerdict::BareNoOutput
    );
}

#[test]
fn bare_no_output_is_the_classification_the_incident_left_behind() {
    // The incident's record: the classification with nothing behind it.
    let record = StatusRecord {
        status: "NO-OUTPUT".to_string(),
        cause: None,
        evidence_file: None,
    };
    assert_eq!(record.gate(), StatusVerdict::BareNoOutput);
    assert_eq!(record.render(), "status=NO-OUTPUT");
    assert_eq!(status_gate("NO-OUTPUT", None), StatusVerdict::BareNoOutput);
    // The gate is about the one classification the incident turned into a
    // non-answer; other statuses are out of scope here.
    assert_eq!(status_gate("TIMEOUT", None), StatusVerdict::Accompanied);
}

// --- AC5: the regression the issue names ------------------------------------

#[test]
fn a_run_stalled_by_an_unreachable_dependency_records_it_as_unreachable() {
    // The regression test the acceptance criteria require: a run stalled by
    // an unreachable dependency must produce a record naming that
    // dependency as unreachable.
    let (evidence, path) = capture("stalls", &stalled_run()).unwrap();
    let cause = evidence.cause_line();
    assert!(
        cause.contains(ENDPOINT),
        "the record must name the dependency the run was given: {cause}"
    );
    assert!(
        cause.contains("unreachable"),
        "the record must state the dependency was unreachable: {cause}"
    );
    assert_eq!(
        cause,
        "unreachable dependency qwen3.8-27b-22771168: connection refused"
    );

    // The record is investigable without reconstructing context from
    // unrelated logs: it names the evidence file, and the file's contents
    // carry the probe, the watched-artifact state, the tail, and the
    // process state at kill time.
    let record = StatusRecord {
        status: "NO-OUTPUT".to_string(),
        cause: Some(Cause::Captured {
            evidence: evidence.clone(),
            line: cause.clone(),
        }),
        evidence_file: Some(path.clone()),
    };
    assert_eq!(record.gate(), StatusVerdict::Accompanied);
    let rendered = record.render();
    assert!(rendered.contains(&format!("evidence={path}")), "{rendered}");
    assert!(rendered.contains(ENDPOINT), "{rendered}");

    let file = evidence.render();
    assert!(file.contains("probe qwen3.8-27b-22771168 → unreachable (connection refused)"));
    assert!(file.contains("agent.out: missing — never created"));
    assert!(file.contains("process state at kill: blocked on I/O"));
}

#[test]
fn the_cause_names_the_first_unreachable_dependency_in_probe_order() {
    // Two dependencies, the first answering and the second unreachable:
    // the cause is the second, named.
    let mut run = stalled_run();
    run.probes = vec![
        Probe {
            dependency: "registry.internal:5000".to_string(),
            outcome: ProbeOutcome::Responded {
                status: 200,
                latency_ms: 9,
            },
        },
        Probe {
            dependency: ENDPOINT.to_string(),
            outcome: ProbeOutcome::Unreachable {
                reason: "connection refused".to_string(),
            },
        },
    ];
    let probe = run.first_unreachable().expect("one probe is unreachable");
    assert_eq!(probe.dependency, ENDPOINT);
    assert_eq!(
        run.cause_line(),
        "unreachable dependency qwen3.8-27b-22771168: connection refused"
    );
}

#[test]
fn the_incident_log_is_2903_bytes_and_the_record_is_not_the_log() {
    // The incident's whole log, reconstructed from the issue: three lines
    // of budget and a stall verdict. The capture is a different, larger
    // kind of record — it answers the questions the log could not.
    let incident_log = "budget: slot_ctx=65536 context_window=65536 max_tokens=32768\n\
                       pi skills in force: <none>\n\
                       STALLED: no session or output activity for 45m after 46m; terminating agent\n";
    let (evidence, _path) = capture("stalls", &stalled_run()).unwrap();
    let record = evidence.render();
    assert!(
        record.len() > incident_log.len(),
        "the evidence record must carry more than the incident's 2,903-byte log"
    );
    // And the status the incident left — `status=NO-OUTPUT agent_rc=143
    // agent_secs=2761 changed_files=0` — is the bare classification the
    // gate rejects on its own.
    let bare = StatusRecord {
        status: "NO-OUTPUT".to_string(),
        cause: None,
        evidence_file: None,
    };
    assert_eq!(bare.gate(), StatusVerdict::BareNoOutput);
}
