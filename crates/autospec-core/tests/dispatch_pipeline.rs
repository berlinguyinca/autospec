//! Filing-to-dispatch liveness (#3800).
//!
//! A filed issue reached an agent only while an interactive session happened to
//! be alive, and when that session died the queue simply stopped being
//! repopulated — the consumer read an untouched file and reported "no work".
//! These tests pin the four invariants that make that shape impossible:
//! population is a scheduled step, a stale artifact is a named failure rather
//! than steady state, every hop carries a liveness stamp, and the credential
//! topology is declared rather than assumed.

use autospec_core::dispatch_pipeline::{
    CredentialRequirement, DispatchOutcome, DispatchPipeline, DispatchTick, EntryState,
    FailureCode, FreshnessPolicy, HostKind, LifecycleLedger, LivenessLedger, LivenessVerdict,
    PipelineStep, PipelineTopology, QueueFile, SchedulingReconciliation, SkipReason, StepSchedule,
    TopologyViolation, DEFAULT_INTERVAL_SECS, DEFAULT_MAX_STALE_INTERVALS, QUEUE_ARTIFACT,
};

const NOW: u64 = 1_800_000_000;

fn step(
    name: &str,
    host: HostKind,
    credential: CredentialRequirement,
    schedule: StepSchedule,
    produces: Option<&str>,
    consumes: &[&str],
) -> PipelineStep {
    PipelineStep {
        name: name.to_string(),
        host,
        credential,
        schedule,
        produces: produces.map(str::to_string),
        consumes: consumes.iter().map(|name| name.to_string()).collect(),
        log: Some(format!("~/.autospec/logs/{name}.log")),
    }
}

fn scheduled(name: &str) -> PipelineStep {
    step(
        name,
        HostKind::Authenticated,
        CredentialRequirement::GitHubToken,
        StepSchedule::Scheduled {
            interval_secs: DEFAULT_INTERVAL_SECS,
        },
        Some(QUEUE_ARTIFACT),
        &[],
    )
}

fn pipeline(topology: PipelineTopology) -> DispatchPipeline {
    DispatchPipeline::new(
        topology,
        LivenessLedger::new(),
        FreshnessPolicy::new(DEFAULT_INTERVAL_SECS, DEFAULT_MAX_STALE_INTERVALS).expect("policy"),
    )
}

fn queue(stamped_at: Option<u64>, entries: &[u64]) -> QueueFile {
    QueueFile {
        entries: entries.to_vec(),
        refreshed_at: stamped_at,
        refreshed_by: stamped_at.map(|_| "refresh-queue".to_string()),
    }
}

// ── Invariant 2: staleness is a named failure, never "no work" ──────────────

#[test]
fn fresh_stamped_queue_with_entries_authorises_dispatch() {
    let pipeline = pipeline(PipelineTopology::reference());
    let outcome = pipeline.authorize_queue(Some(&queue(Some(NOW - 30), &[12, 13])), NOW);

    assert_eq!(
        outcome,
        DispatchOutcome::Proceed {
            entries: 2,
            age_secs: 30
        }
    );
    assert!(!outcome.held());
}

#[test]
fn fresh_empty_queue_reads_as_idle_and_still_exits_zero() {
    let pipeline = pipeline(PipelineTopology::reference());
    let outcome = pipeline.authorize_queue(Some(&queue(Some(NOW - 60), &[])), NOW);

    assert_eq!(outcome, DispatchOutcome::Idle { age_secs: 60 });
    assert!(!outcome.held());
    assert!(outcome.line().contains("no new issues were filed"));
}

#[test]
fn stale_stamp_holds_and_names_the_step_that_stopped_refreshing() {
    let pipeline = pipeline(PipelineTopology::reference());
    // 4 intervals of silence against a tolerance of 3.
    let outcome = pipeline.authorize_queue(Some(&queue(Some(NOW - 2_400), &[7])), NOW);

    let DispatchOutcome::Hold { failure } = &outcome else {
        panic!("a 4-interval-old queue must hold, got {outcome:?}");
    };
    assert_eq!(failure.code, FailureCode::StampNotRefreshed);
    assert_eq!(failure.step, "refresh-queue");
    assert_eq!(failure.artifact.as_deref(), Some(QUEUE_ARTIFACT));
    assert!(failure.message.contains("4 intervals"));
    assert!(failure.message.contains("refusing to read a stale queue"));
}

#[test]
fn unstamped_queue_holds_even_when_it_has_work() {
    let pipeline = pipeline(PipelineTopology::reference());
    // The #3800 shape in miniature: content the consumer cannot date.
    let outcome = pipeline.authorize_queue(Some(&queue(None, &[44])), NOW);

    let DispatchOutcome::Hold { failure } = &outcome else {
        panic!("an unstamped queue must never authorise dispatch, got {outcome:?}");
    };
    assert_eq!(failure.code, FailureCode::QueueUnstamped);
    assert_eq!(failure.step, "refresh-queue");
}

#[test]
fn empty_unstamped_queue_is_a_failure_not_the_absence_of_work() {
    let pipeline = pipeline(PipelineTopology::reference());
    let outcome = pipeline.authorize_queue(Some(&queue(None, &[])), NOW);

    assert!(outcome.held());
    assert!(!outcome.line().contains("no new issues were filed"));
}

#[test]
fn absent_queue_holds_with_its_own_code() {
    let pipeline = pipeline(PipelineTopology::reference());
    let outcome = pipeline.authorize_queue(None, NOW);

    let DispatchOutcome::Hold { failure } = &outcome else {
        panic!("a missing queue must hold, got {outcome:?}");
    };
    assert_eq!(failure.code, FailureCode::QueueMissing);
}

#[test]
fn future_stamp_holds_as_clock_rewind() {
    let pipeline = pipeline(PipelineTopology::reference());
    let outcome = pipeline.authorize_queue(Some(&queue(Some(NOW + 120), &[3])), NOW);

    let DispatchOutcome::Hold { failure } = &outcome else {
        panic!("a future stamp must hold, got {outcome:?}");
    };
    assert_eq!(failure.code, FailureCode::ClockRewind);
    assert!(failure.message.contains("120s in the future"));
}

#[test]
fn queue_age_is_measured_in_the_producers_own_interval() {
    // A step that runs every 60s is held far sooner than one that runs every hour.
    let fast = PipelineTopology::new(vec![PipelineStep {
        schedule: StepSchedule::Scheduled { interval_secs: 60 },
        ..scheduled("refresh-queue")
    }]);
    let outcome = pipeline(fast).authorize_queue(Some(&queue(Some(NOW - 400), &[1])), NOW);

    assert!(outcome.held(), "400s is 6.6 intervals of a 60s step");
}

// ── Invariant 1: population is a scheduled step with a failure signal ───────

#[test]
fn reference_topology_declares_no_defects() {
    assert_eq!(PipelineTopology::reference().audit(), Vec::new());
}

#[test]
fn session_scoped_queue_producer_is_the_defect_that_cause_3800() {
    let broken = step(
        "refresh-queue",
        HostKind::Authenticated,
        CredentialRequirement::GitHubToken,
        StepSchedule::SessionScoped,
        Some(QUEUE_ARTIFACT),
        &[],
    );
    let consumer = step(
        "topup",
        HostKind::Authenticated,
        CredentialRequirement::GitHubToken,
        StepSchedule::Scheduled {
            interval_secs: DEFAULT_INTERVAL_SECS,
        },
        None,
        &[QUEUE_ARTIFACT],
    );

    let violations = PipelineTopology::new(vec![broken, consumer]).audit();
    assert!(
        violations.contains(&TopologyViolation::ProducerNotScheduled {
            step: "refresh-queue".to_string(),
            artifact: QUEUE_ARTIFACT.to_string(),
        }),
        "a queue repopulated only from a session must be a declared defect: {violations:?}"
    );
}

#[test]
fn queue_producer_running_on_an_ephemeral_host_is_a_defect() {
    // A durable schedule on a host that dies with its session is the same
    // failure in slower motion: the queue goes stale once nobody is logged in.
    let ephemeral = PipelineStep {
        host: HostKind::EphemeralSession,
        ..scheduled("refresh-queue")
    };
    let consumer = step(
        "topup",
        HostKind::Authenticated,
        CredentialRequirement::GitHubToken,
        StepSchedule::Scheduled {
            interval_secs: DEFAULT_INTERVAL_SECS,
        },
        None,
        &[QUEUE_ARTIFACT],
    );

    let violations = PipelineTopology::new(vec![ephemeral, consumer]).audit();
    assert!(
        violations.contains(&TopologyViolation::ProducerHostNotDurable {
            step: "refresh-queue".to_string(),
            artifact: QUEUE_ARTIFACT.to_string(),
        }),
        "{violations:?}"
    );
}

#[test]
fn consumed_artifact_with_no_producer_is_a_defect() {
    let broken = PipelineTopology::new(vec![step(
        "topup",
        HostKind::Authenticated,
        CredentialRequirement::GitHubToken,
        StepSchedule::Scheduled {
            interval_secs: DEFAULT_INTERVAL_SECS,
        },
        None,
        &[QUEUE_ARTIFACT],
    )]);

    assert!(broken
        .audit()
        .contains(&TopologyViolation::UnknownProducer {
            artifact: QUEUE_ARTIFACT.to_string()
        }));
}

#[test]
fn step_without_a_log_has_no_failure_signal_and_is_a_defect() {
    let mute = PipelineStep {
        log: None,
        ..scheduled("refresh-queue")
    };

    assert_eq!(
        PipelineTopology::new(vec![mute]).audit(),
        vec![TopologyViolation::NoLog {
            step: "refresh-queue".to_string()
        }]
    );
}

#[test]
fn producer_defects_are_only_demanded_for_artifacts_somebody_waits_on() {
    // Nothing consumes `orphan.txt`, so a session-scoped producer of it is fine.
    let loose = step(
        "occasional-export",
        HostKind::Authenticated,
        CredentialRequirement::GitHubToken,
        StepSchedule::SessionScoped,
        Some("orphan.txt"),
        &[],
    );

    assert_eq!(PipelineTopology::new(vec![loose]).audit(), Vec::new());
}

// ── Invariant 4: the credential topology is declared, not assumed ───────────

#[test]
fn credential_on_cluster_shared_storage_is_a_defect() {
    let broken = PipelineTopology::new(vec![PipelineStep {
        host: HostKind::SharedCluster,
        ..scheduled("refresh-queue")
    }]);

    assert!(broken
        .audit()
        .contains(&TopologyViolation::CredentialOnSharedStorage {
            step: "refresh-queue".to_string()
        }));
}

#[test]
fn unattended_credential_step_on_an_ephemeral_host_is_a_defect() {
    let broken = PipelineTopology::new(vec![PipelineStep {
        host: HostKind::EphemeralSession,
        schedule: StepSchedule::Scheduled {
            interval_secs: DEFAULT_INTERVAL_SECS,
        },
        ..scheduled("refresh-queue")
    }]);

    assert!(broken
        .audit()
        .contains(&TopologyViolation::CredentialHostNotDurable {
            step: "refresh-queue".to_string()
        }));
}

#[test]
fn attended_credential_step_may_live_on_a_session_host() {
    let attended = step(
        "file-issue",
        HostKind::EphemeralSession,
        CredentialRequirement::GitHubToken,
        StepSchedule::SessionScoped,
        None,
        &[],
    );

    assert_eq!(PipelineTopology::new(vec![attended]).audit(), Vec::new());
}

#[test]
fn credential_steps_are_enumerable_for_the_report() {
    let topology = PipelineTopology::reference();
    let names: Vec<&str> = topology
        .credential_steps()
        .iter()
        .map(|step| step.name.as_str())
        .collect();

    assert_eq!(
        names,
        ["file-issue", "refresh-queue", "topup"],
        "dispatch-agent runs unauthenticated on the cluster"
    );
}

// ── Invariant 3: every hop carries a liveness stamp ─────────────────────────

#[test]
fn beats_are_monotonic_so_a_rewound_clock_cannot_erase_life() {
    let mut ledger = LivenessLedger::new();
    ledger.record("topup", 1_000);
    ledger.record("topup", 900);

    assert_eq!(ledger.last_beat("topup"), Some(1_000));
}

#[test]
fn hop_that_never_beat_fails_liveness() {
    let ledger = LivenessLedger::new();

    assert_eq!(
        ledger.assess(&scheduled("topup"), &FreshnessPolicy::default(), NOW),
        LivenessVerdict::NeverBeat
    );
}

#[test]
fn silent_hop_reports_the_intervals_it_missed() {
    let mut ledger = LivenessLedger::new();
    ledger.record("topup", NOW - 2_400);

    let verdict = ledger.assess(&scheduled("topup"), &FreshnessPolicy::default(), NOW);
    assert_eq!(
        verdict,
        LivenessVerdict::Silent {
            age_secs: 2_400,
            missed_intervals: 4,
            max_intervals: DEFAULT_MAX_STALE_INTERVALS,
        }
    );
    assert!(verdict.line("topup").contains("SILENT"));
    assert_eq!(
        verdict.failure("topup").map(|failure| failure.code),
        Some(FailureCode::StepSilent)
    );
}

#[test]
fn session_scoped_hop_is_not_expected_to_beat() {
    let ledger = LivenessLedger::new();
    let hop = step(
        "file-issue",
        HostKind::Authenticated,
        CredentialRequirement::GitHubToken,
        StepSchedule::SessionScoped,
        None,
        &[],
    );

    let verdict = ledger.assess(&hop, &FreshnessPolicy::default(), NOW);
    assert!(!verdict.failed());
    assert_eq!(
        verdict,
        LivenessVerdict::EventDriven {
            last_beat_age_secs: None
        }
    );
    assert!(verdict.line("file-issue").contains("EVENT-DRIVEN"));
}

#[test]
fn a_hand_written_topology_uses_the_spellings_the_report_prints() {
    // The topology file is written by a human copying a flag out of the report,
    // so the names read back in must be the names printed out.
    let json = r#"{"steps":[{"name":"refresh-queue","host":"authenticated","credential":"gh-token","schedule":{"scheduled":{"interval_secs":600}},"produces":"queue.txt","consumes":[],"log":"~/.autospec/logs/refresh-queue.log"}]}"#;

    let topology: PipelineTopology =
        serde_json::from_str(json).expect("hand-written topology parses");
    let step = &topology.steps()[0];
    assert_eq!(step.host.as_str(), "authenticated");
    assert_eq!(step.credential.as_str(), "gh-token");

    let written: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&topology).expect("prints")).expect("reparses");
    let read: serde_json::Value = serde_json::from_str(json).expect("parses");
    assert_eq!(written, read);
}

#[test]
fn ledger_survives_a_json_roundtrip() {
    let mut ledger = LivenessLedger::new();
    ledger.record("topup", 42);
    let text = ledger.to_json();

    assert_eq!(LivenessLedger::from_json(&text).expect("parses"), ledger);
    assert!(LivenessLedger::from_json("not json").is_err());
}

// ── The combined report ────────────────────────────────────────────────────

#[test]
fn report_is_healthy_only_with_fresh_queue_live_hops_and_clean_topology() {
    let live = {
        let mut ledger = LivenessLedger::new();
        for name in ["refresh-queue", "topup", "dispatch-agent"] {
            ledger.record(name, NOW);
        }
        ledger
    };
    let policy = FreshnessPolicy::default();
    let healthy = DispatchPipeline::new(PipelineTopology::reference(), live.clone(), policy);

    let report = healthy.report(Some(&queue(Some(NOW - 5), &[9])), NOW);
    assert!(report.healthy(), "{}", report.lines().join("\n"));
    assert_eq!(report.hops.len(), 4);

    // A silent hop alone is enough to fail it. Built from scratch rather than
    // by re-recording, because a beat is monotonic and cannot be unwound.
    let mut stalled = LivenessLedger::new();
    stalled.record("refresh-queue", NOW);
    stalled.record("topup", NOW);
    stalled.record("dispatch-agent", NOW - 10_000);
    let silent = DispatchPipeline::new(PipelineTopology::reference(), stalled, policy);
    let report = silent.report(Some(&queue(Some(NOW - 5), &[9])), NOW);
    assert!(!report.healthy());
    assert_eq!(report.failures()[0].code, FailureCode::StepSilent);

    // A stale artifact alone is enough to fail it.
    let stale = healthy.report(Some(&queue(Some(NOW - 10_000), &[9])), NOW);
    assert!(!stale.healthy());
    assert!(stale
        .failures()
        .iter()
        .any(|failure| failure.code == FailureCode::StampNotRefreshed));

    // A declared topology defect alone is enough to fail it.
    let mute = DispatchPipeline {
        topology: PipelineTopology::new(vec![PipelineStep {
            log: None,
            ..scheduled("refresh-queue")
        }]),
        ..healthy
    };
    let report = mute.report(Some(&queue(Some(NOW - 5), &[9])), NOW);
    assert!(!report.healthy());
    assert!(report
        .failures()
        .iter()
        .any(|failure| failure.code == FailureCode::StepHasNoLog));
}

#[test]
fn report_lines_lead_with_defects_and_end_with_the_verdict() {
    let broken = DispatchPipeline::new(
        PipelineTopology::new(vec![step(
            "topup",
            HostKind::SharedCluster,
            CredentialRequirement::GitHubToken,
            StepSchedule::Scheduled {
                interval_secs: DEFAULT_INTERVAL_SECS,
            },
            None,
            &[QUEUE_ARTIFACT],
        )]),
        LivenessLedger::new(),
        FreshnessPolicy::default(),
    );

    let lines = broken.report(None, NOW).lines();
    assert!(lines[0].starts_with("TOPOLOGY DEFECT"));
    assert!(lines.last().expect("line").contains("LIVENESS FAILURE"));
}

// ── Artifact format ────────────────────────────────────────────────────────

#[test]
fn queue_roundtrips_entries_and_freshness_headers() {
    let mut file = QueueFile::parse("44\n45\n");
    assert_eq!(file.entries, vec![44, 45]);
    assert_eq!(file.refreshed_at, None);

    file.stamp(NOW, "refresh-queue");
    let rendered = file.render();
    let reparsed = QueueFile::parse(&rendered);

    assert_eq!(reparsed, file);
    assert_eq!(reparsed.refreshed_at, Some(NOW));
    assert_eq!(reparsed.refreshed_by.as_deref(), Some("refresh-queue"));
    assert_eq!(reparsed.entry_count(), 2);
}

#[test]
fn queue_parse_skips_comments_blanks_and_non_numeric_lines() {
    let file = QueueFile::parse(
        "# refreshed-at: 1700000000\n# refreshed-by: refresh-queue\n\n  12 \nnot-a-number\n13\n",
    );

    assert_eq!(file.entries, vec![12, 13]);
    assert_eq!(file.refreshed_at, Some(1_700_000_000));
    assert_eq!(file.refreshed_by.as_deref(), Some("refresh-queue"));
}

#[test]
fn freshness_policy_refuses_a_zero_interval_or_tolerance() {
    assert!(FreshnessPolicy::new(0, 3).is_none());
    assert!(FreshnessPolicy::new(600, 0).is_none());
    assert!(FreshnessPolicy::new(600, 3).is_some());
}

// ── Admission reconciliation (#3927) ───────────────────────────────────────

#[test]
fn reconciliation_is_clean_when_every_admitted_issue_is_in_the_queue() {
    // Every admitted issue is schedulable; the one queue entry that is no
    // longer admitted is reported as stale, not as a defect.
    let recon = SchedulingReconciliation::new([10, 20, 30], [10, 20, 30, 40]);

    assert_eq!(recon.admitted_not_schedulable_count(), 0);
    assert!(!recon.is_defect());
    assert_eq!(recon.schedulable, vec![10, 20, 30]);
    assert_eq!(recon.schedulable_not_admitted, vec![40]);
    assert!(recon.line().contains("clean"));
    assert!(recon.line().contains("40"));
}

#[test]
fn reconciliation_names_the_admitted_issues_not_in_the_queue_as_a_defect() {
    // Two admitted issues sit outside the queue: the count is 2, naming both in
    // ascending order, and the reverse-direction stale entry is still reported.
    let recon = SchedulingReconciliation::new([7, 9, 21], [21, 30]);

    assert_eq!(recon.admitted_not_schedulable, vec![7, 9]);
    assert_eq!(recon.admitted_not_schedulable_count(), 2);
    assert!(recon.is_defect());
    assert_eq!(recon.schedulable, vec![21]);
    assert_eq!(recon.schedulable_not_admitted, vec![30]);
    let line = recon.line();
    assert!(line.contains("DEFECT"));
    assert!(line.contains("2 admitted issue"));
    assert!(line.contains("7, 9"));
}

#[test]
fn reconciliation_deduplicates_and_orders_both_sides() {
    let recon = SchedulingReconciliation::new([5, 5, 3], [3, 3, 5, 5]);
    assert_eq!(recon.admitted_not_schedulable, Vec::<u64>::new());
    assert!(!recon.is_defect());
    assert_eq!(recon.schedulable, vec![3, 5]);
}

#[test]
fn idle_dispatcher_with_no_admitted_work_stays_idle() {
    let pipeline = pipeline(PipelineTopology::reference());
    let outcome = pipeline.authorize_queue_with_admission(
        Some(&queue(Some(NOW - 30), &[])),
        Vec::<u64>::new(),
        NOW,
    );

    assert_eq!(outcome, DispatchOutcome::Idle { age_secs: 30 });
    assert!(!outcome.held());
    assert!(outcome.line().contains("no new issues were filed"));
}

#[test]
fn idle_dispatcher_with_admitted_work_is_a_fault_not_silence() {
    let pipeline = pipeline(PipelineTopology::reference());
    // Free capacity (a fresh, empty queue) while two admitted issues sit outside
    // it: the dispatcher must hold, naming the refresh step, not claim idle.
    let outcome =
        pipeline.authorize_queue_with_admission(Some(&queue(Some(NOW - 30), &[])), [41, 57], NOW);

    let DispatchOutcome::Hold { failure } = &outcome else {
        panic!("free capacity over filed work must hold, got {outcome:?}");
    };
    assert_eq!(failure.code, FailureCode::AdmittedNotSchedulable);
    assert_eq!(failure.step, "refresh-queue");
    assert_eq!(failure.artifact.as_deref(), Some(QUEUE_ARTIFACT));
    assert!(failure.message.contains("41, 57"));
    assert!(!failure.message.contains("no new issues were filed"));
}

#[test]
fn a_proceeding_queue_is_untouched_by_admission_reconciliation() {
    let pipeline = pipeline(PipelineTopology::reference());
    // The queue already has work, so the dispatcher proceeds regardless; the
    // admitted-but-unschedulable issue remains the reconciliation's business.
    let outcome =
        pipeline.authorize_queue_with_admission(Some(&queue(Some(NOW - 30), &[8])), [9], NOW);

    assert_eq!(
        outcome,
        DispatchOutcome::Proceed {
            entries: 1,
            age_secs: 30
        }
    );
}

#[test]
fn a_stale_queue_stays_stale_regardless_of_admission() {
    let pipeline = pipeline(PipelineTopology::reference());
    // A 4-interval-old empty queue is a staleness fault before admission is
    // even consulted.
    let outcome =
        pipeline.authorize_queue_with_admission(Some(&queue(Some(NOW - 2_400), &[])), [41], NOW);

    let DispatchOutcome::Hold { failure } = &outcome else {
        panic!("a stale queue must hold, got {outcome:?}");
    };
    assert_eq!(failure.code, FailureCode::StampNotRefreshed);
}

// ── The populated case (#3927) ──────────────────────────────────────────────

#[test]
fn populated_case_names_the_admitted_issues_missing_from_a_populated_queue() {
    // The populated case from #3927: a queue with 242 entries and six admitted
    // issues the refresher never staged. The reconciliation flags exactly six,
    // and the dispatcher — with work to do — still proceeds.
    let queue_numbers: Vec<u64> = (1..=242).collect();
    let admitted: Vec<u64> = (243..=248).collect();
    let recon =
        SchedulingReconciliation::new(admitted.iter().copied(), queue_numbers.iter().copied());

    assert_eq!(recon.admitted_not_schedulable_count(), 6);
    assert_eq!(recon.admitted_not_schedulable, admitted);
    assert!(recon.is_defect());
    assert!(recon.line().contains("6 admitted issue"));

    let pipeline = pipeline(PipelineTopology::reference());
    let populated = queue(Some(NOW - 30), &queue_numbers);
    let outcome =
        pipeline.authorize_queue_with_admission(Some(&populated), admitted.iter().copied(), NOW);
    assert_eq!(
        outcome,
        DispatchOutcome::Proceed {
            entries: 242,
            age_secs: 30
        }
    );
}

#[test]
fn populated_case_reports_a_fault_when_the_queue_is_empty() {
    // The same six admitted issues, but the queue is empty: now the dispatcher
    // itself must report the fault instead of idling silently.
    let pipeline = pipeline(PipelineTopology::reference());
    let outcome = pipeline.authorize_queue_with_admission(
        Some(&queue(Some(NOW - 30), &[])),
        (243..=248).collect::<Vec<u64>>(),
        NOW,
    );

    let DispatchOutcome::Hold { failure } = &outcome else {
        panic!("an empty queue over admitted work must hold, got {outcome:?}");
    };
    assert_eq!(failure.code, FailureCode::AdmittedNotSchedulable);
    assert_eq!(failure.step, "refresh-queue");
    assert!(failure.message.contains("243, 244, 245, 246, 247, 248"));
}

// ── Entry lifecycle (#3911) ────────────────────────────────────────────────

#[test]
fn populated_lifecycle_case_moves_produced_entries_through_conversion() {
    // A 20-entry queue moving through three lifecycle phases. The claim
    // under test: produced is a state mid-lifecycle, not a terminal one —
    // the tick converts it, reports every skip with a reason, and a cleared
    // hold re-enters as conversion rather than fresh dispatch.
    let entries: Vec<u64> = (3911..=3930).collect();
    let populated = queue(Some(NOW), &entries);

    // Phase 1 — nothing stamped: every entry is queued by default and the
    // tick dispatches the whole queue fresh.
    let ledger = LifecycleLedger::new();
    let tick = DispatchTick::run(&populated, &ledger);
    assert!(tick.dispatched_anything());
    assert_eq!(tick.fresh_count(), 20);
    assert_eq!(tick.convert_count(), 0);
    assert!(tick.skipped().is_empty());

    // Phase 2 — the lifecycle has moved: the first eight entries produced a
    // patch, two of them converted, one of the produced held. Every one of
    // the 20 entries is accounted for, and the skips are named.
    let mut ledger = LifecycleLedger::new();
    for issue in entries.iter().take(8) {
        assert!(ledger.record(*issue, EntryState::Produced, NOW));
    }
    assert!(ledger.record(3911, EntryState::Converted, NOW));
    assert!(ledger.record(3912, EntryState::Converted, NOW));
    assert!(ledger.hold(3913, "conversion blocked: branch dirty", NOW));

    // produced-but-unconverted is directly queryable: the six produced
    // entries that have not converted — the held one still counts.
    assert_eq!(
        ledger.produced_but_unconverted(),
        vec![3913, 3914, 3915, 3916, 3917, 3918]
    );
    assert_eq!(ledger.unconverted_count(), 6);

    let tick = DispatchTick::run(&populated, &ledger);
    assert_eq!(tick.fresh_count(), 12);
    assert_eq!(tick.convert_count(), 5);
    assert_eq!(tick.held_count(), 1);
    assert_eq!(tick.converted_count(), 2);

    let lines = tick.lines();
    assert_eq!(
        lines[0],
        "dispatch tick: 17 dispatched (12 fresh, 5 convert), 3 skipped"
    );
    assert!(lines
        .iter()
        .any(|line| line == "#3911 converted [terminal]: should leave the queue"));
    assert!(lines
        .iter()
        .any(|line| line == "#3912 converted [terminal]: should leave the queue"));
    assert!(lines
        .iter()
        .any(|line| line == "#3913 held [produced]: conversion blocked: branch dirty"));
    // a produced entry is converted, never re-dispatched fresh
    assert!(lines.iter().any(|line| line == "#3914 convert [produced]"));
    assert!(lines.iter().any(|line| line == "#3930 dispatch [queued]"));

    // the hold carries its reason as structured data, not just text
    let held = tick
        .skipped()
        .iter()
        .find(|entry| entry.issue == 3913)
        .expect("3913 is skipped");
    assert_eq!(
        held.reason,
        SkipReason::Held {
            reason: "conversion blocked: branch dirty".to_string(),
            state: EntryState::Produced,
        }
    );

    // Phase 3 — the hold clears: the entry resumes as conversion, not fresh
    // dispatch, and the tick is back to acting on 18 of the 20 entries.
    assert!(ledger.release(3913, NOW));
    let tick = DispatchTick::run(&populated, &ledger);
    assert_eq!(tick.convert_count(), 6);
    assert_eq!(tick.held_count(), 0);
    assert_eq!(tick.converted_count(), 2);
    let lines = tick.lines();
    assert!(lines.iter().any(|line| line == "#3913 convert [produced]"));
}
