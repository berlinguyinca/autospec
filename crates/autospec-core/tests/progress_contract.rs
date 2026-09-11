//! Progress contracts and liveness from positive artifacts (issue #4259).
//!
//! The incident: 19 of 21 running agents stuck past their 45-minute limit —
//! one by 6 h 50 m — each holding a GPU worker slot while producing zero
//! bytes. Job state said `RUNNING`, agent count said 22/22 fully utilised, the
//! wrapper `.out` file existed at 264 bytes and `agent.out` at 0 bytes, and
//! every one of those readings was true for a healthy agent too.
//!
//! The invariants under test, in the order the incident produced them:
//!
//! * buffered-until-exit output is not instrumentation, and an observation
//!   made under it yields no verdict rather than a false one (invariant 1);
//! * liveness comes from a positive artifact — absence of the known-next-step
//!   marker past `limit + grace` beats a live process (invariant 2);
//! * a check that could not run is never a green check: a missing or
//!   unreadable directory, or a relative marker path, blocks the verdict
//!   instead of producing "healthy" or a false `stuck` (invariant 3);
//! * a detector must not be confoundable by buffering: "the log is empty" is
//!   rejected outright, "still inside the call past the limit" is not
//!   (invariant 4);
//! * utilisation is occupancy, so a saturation reading needs a throughput
//!   companion on the same line, and 22/22 with 2 patches/h reads as
//!   `OccupiedNotWorking` (invariant 5);
//! * a spec commissioning a long-running worker must commission a progress
//!   contract, not only a completion contract.

use std::time::Duration;

use autospec_core::progress_contract::{
    commission_verdict, fleet_verdict, instrumented, observe_run, read_marker, reject_confoundable,
    step_liveness, step_liveness_at, usable_for_liveness, validate_marker_path, CommissionVerdict,
    ConfoundedReason, DetectionVerdict, Detector, Emitter, FleetSignal, FleetVerdict, Instrumented,
    MarkerEvidence, MarkerLocation, MarkerReadDefect, OutputSink, ProgressContract,
    ProgressEmission, RunObservation, RunVerdict, StepLiveness, StepMarker, StepObservation,
    WorkerCommission,
};

const MIN: Duration = Duration::from_secs(60);

/// The 45-minute agent limit from the incident.
const LIMIT: Duration = Duration::from_secs(45 * 60);

/// A contract that actually works: a per-step flush well inside the limit.
fn working_contract() -> ProgressContract {
    ProgressContract {
        max_silence: 5 * MIN,
        sink: OutputSink::FlushPerStep,
        run_limit: LIMIT,
    }
}

/// The contract the fleet actually had: stdout to a file, nothing else.
fn buffered_contract() -> ProgressContract {
    ProgressContract {
        max_silence: 5 * MIN,
        sink: OutputSink::RedirectedStdout,
        run_limit: LIMIT,
    }
}

/// The marker that finally discriminated: written immediately after the model
/// call returns, by the wrapper.
fn model_call_marker() -> StepMarker {
    StepMarker {
        name: "model-call".to_string(),
        artifact: "build.log".to_string(),
    }
}

fn step_observation(elapsed: Duration, marker_present: bool) -> StepObservation {
    StepObservation {
        elapsed,
        limit: LIMIT,
        grace: 10 * MIN,
        marker_present,
        // The scheduler reported RUNNING for all 19 wedged agents.
        process_running: true,
    }
}

// --- Invariant 1: buffered-until-exit output is not instrumentation -------

#[test]
fn redirected_stdout_is_never_instrumentation_however_short_the_interval() {
    // The 264-byte wrapper log and the 0-byte agent.out: both exist, neither
    // discriminates, and no promised interval fixes a buffer flushed at exit.
    assert_eq!(
        instrumented(&buffered_contract()),
        Instrumented::BufferedUntilExit
    );
    assert!(!Instrumented::BufferedUntilExit.is_observable());
    assert!(Instrumented::BufferedUntilExit
        .line()
        .contains("not instrumentation"));
}

#[test]
fn a_flushed_sink_with_a_firing_interval_is_observable() {
    assert_eq!(instrumented(&working_contract()), Instrumented::Observable);
    assert!(Instrumented::Observable.is_observable());
}

#[test]
fn an_interval_longer_than_the_run_limit_never_fires() {
    // A "heartbeat every 2h" on a 45-minute run: a healthy task is silent for
    // its whole life, so silence proves nothing.
    let contract = ProgressContract {
        max_silence: 2 * Duration::from_secs(3600),
        sink: OutputSink::WrapperArtifact,
        run_limit: LIMIT,
    };
    assert_eq!(
        instrumented(&contract),
        Instrumented::IntervalNeverFires {
            max_silence: 2 * Duration::from_secs(3600),
            run_limit: LIMIT,
        }
    );
}

#[test]
fn every_output_sink_that_flushes_is_visible_while_running() {
    assert!(OutputSink::FlushPerStep.visible_while_running());
    assert!(OutputSink::WrapperArtifact.visible_while_running());
    assert!(!OutputSink::RedirectedStdout.visible_while_running());
}

#[test]
fn a_silent_run_under_a_good_contract_is_flagged() {
    let obs = RunObservation {
        age: 20 * MIN,
        last_progress: Some(12 * MIN),
        output_bytes: 4096,
    };
    assert_eq!(
        observe_run(&working_contract(), &obs),
        RunVerdict::SilentBeyondContract {
            silent_for: 12 * MIN
        }
    );
}

#[test]
fn a_recent_emission_reads_as_progressing() {
    let obs = RunObservation {
        age: 20 * MIN,
        last_progress: Some(MIN),
        output_bytes: 0,
    };
    // Zero bytes with a fresh emission is a working run: bytes are recorded,
    // never decisive.
    assert_eq!(
        observe_run(&working_contract(), &obs),
        RunVerdict::Progressing
    );
}

#[test]
fn a_run_with_no_emission_yet_is_only_overdue_past_the_bound() {
    let fresh = RunObservation {
        age: MIN,
        last_progress: None,
        output_bytes: 0,
    };
    assert_eq!(
        observe_run(&working_contract(), &fresh),
        RunVerdict::NotYetDue
    );
    let overdue = RunObservation {
        age: 30 * MIN,
        last_progress: None,
        output_bytes: 0,
    };
    assert_eq!(
        observe_run(&working_contract(), &overdue),
        RunVerdict::SilentBeyondContract {
            silent_for: 30 * MIN
        }
    );
}

#[test]
fn a_buffered_run_is_unobservable_whether_healthy_or_wedged() {
    // The two agents that were byte-identical on disk: a healthy one three
    // minutes into its run and a hung one seven hours in. Both read
    // `Unobservable`, which is the only honest verdict available, and neither
    // is reported as stuck.
    // The healthy agent: three minutes in, on a 45-minute limit.
    let healthy = RunObservation {
        age: 3 * MIN,
        last_progress: None,
        output_bytes: 0,
    };
    let hung = RunObservation {
        age: 7 * Duration::from_secs(3600),
        last_progress: None,
        output_bytes: 0,
    };
    let a = observe_run(&buffered_contract(), &healthy);
    let b = observe_run(&buffered_contract(), &hung);
    assert_eq!(a, RunVerdict::Unobservable);
    assert_eq!(b, RunVerdict::Unobservable);
    assert!(!a.discriminating() && !b.discriminating());
    // Nine minutes and seven hours collapse to one reading: the contract
    // cannot tell them apart, which is the defect being fixed.
    assert_eq!(
        a, b,
        "a healthy and a wedged run read identically under buffering"
    );
    assert!(b.line().contains("healthy run and a wedged one"));

    // The same two observations under a contract that flushes do separate
    // them — one is simply not due yet — so the fault is the sink, not the
    // observation.
    assert_eq!(
        observe_run(&working_contract(), &healthy),
        RunVerdict::NotYetDue
    );
    assert_eq!(
        observe_run(&working_contract(), &hung),
        RunVerdict::SilentBeyondContract {
            silent_for: 7 * Duration::from_secs(3600)
        }
    );
}

#[test]
fn an_unobservable_contract_never_claims_silence_beyond_contract() {
    // Firing on "the log is empty" fires on everybody, including the 22
    // working agents; the module refuses to derive that verdict at all.
    let obs = RunObservation {
        age: 6 * Duration::from_secs(3600),
        last_progress: None,
        output_bytes: 0,
    };
    assert!(!matches!(
        observe_run(&buffered_contract(), &obs),
        RunVerdict::SilentBeyondContract { .. }
    ));
}

// --- Invariant 2: liveness from a positive artifact -----------------------

#[test]
fn the_completion_marker_settles_liveness_positively() {
    let obs = step_observation(3 * Duration::from_secs(3600), true);
    assert_eq!(step_liveness(&obs), StepLiveness::Completed);
    assert!(step_liveness(&obs).discriminating());
    let line = StepLiveness::Completed.line(&model_call_marker());
    assert!(line.contains("build.log"), "{line}");
}

#[test]
fn zero_output_inside_the_limit_means_nothing() {
    let obs = step_observation(9 * MIN, false);
    assert_eq!(step_liveness(&obs), StepLiveness::WithinLimit);
    assert!(!step_liveness(&obs).discriminating());
    assert!(!step_liveness(&obs).wedged());
}

#[test]
fn past_the_limit_but_inside_grace_is_late_not_evidence() {
    let obs = step_observation(LIMIT + 4 * MIN, false);
    assert_eq!(
        step_liveness(&obs),
        StepLiveness::AwaitingGrace {
            beyond_limit: 4 * MIN
        }
    );
    assert!(!step_liveness(&obs).wedged());
}

#[test]
fn absence_of_the_marker_past_limit_plus_grace_proves_the_call_never_returned() {
    // The one signal that discriminated in the incident: build.log written
    // immediately after the model call returns, absent 6h50m past the limit.
    let obs = step_observation(LIMIT + 50 * MIN, false);
    let verdict = step_liveness(&obs);
    assert!(verdict.wedged());
    assert!(verdict.discriminating());
    assert_eq!(
        verdict,
        StepLiveness::WedgedInStep {
            beyond_limit: 50 * MIN
        }
    );
    let line = verdict.line(&model_call_marker());
    assert!(line.contains("never returned"), "{line}");
    assert!(line.contains("build.log"), "{line}");
}

#[test]
fn a_live_process_and_a_running_job_never_outrule_the_missing_marker() {
    // Every wedged agent had process_running == true and Slurm state RUNNING.
    let wedged = step_observation(6 * Duration::from_secs(3600) + LIMIT, false);
    assert!(wedged.process_running);
    assert!(step_liveness(&wedged).wedged());
}

#[test]
fn a_short_run_never_reads_as_wedged_on_absence_alone() {
    // The marker has no reason to exist yet, so absence must not fire.
    let obs = StepObservation {
        elapsed: 30 * MIN,
        limit: LIMIT,
        grace: 10 * MIN,
        marker_present: false,
        process_running: true,
    };
    assert!(!step_liveness(&obs).wedged());
}

// --- Invariant 3: a check that could not run is never a green check -------

#[test]
fn a_scanned_absence_is_evidence_and_a_missing_directory_is_not() {
    let path = "/work/out/issue-4259/build.log";
    // The directory is there and the marker is not: that is the wedge signal.
    assert_eq!(
        read_marker(path, MarkerLocation::Scanned { present: false }),
        MarkerEvidence::Absent
    );
    // The directory is not there: the check never ran, so absence is not a
    // finding about the run.
    let blocked = read_marker(path, MarkerLocation::DirectoryMissing);
    assert_eq!(
        blocked,
        MarkerEvidence::CouldNotRun {
            path: path.to_string(),
            reason: MarkerReadDefect::DirectoryMissing,
        }
    );
    assert!(!blocked.ran());
}

#[test]
fn a_present_marker_is_reported_whatever_the_directory_looks_like_afterwards() {
    assert_eq!(
        read_marker(
            "/work/out/build.log",
            MarkerLocation::Scanned { present: true }
        ),
        MarkerEvidence::Present
    );
}

#[test]
fn an_unreadable_directory_blocks_the_check_rather_than_reporting_absence() {
    let blocked = read_marker("/work/out/build.log", MarkerLocation::Unreadable);
    assert!(!blocked.ran());
    match blocked {
        MarkerEvidence::CouldNotRun { reason, .. } => {
            assert_eq!(reason, MarkerReadDefect::Unreadable);
            assert!(reason.as_str().contains("unreadable"), "{reason:?}");
        }
        other => panic!("expected a blocked read, got {other:?}"),
    }
}

#[test]
fn a_relative_or_empty_marker_path_is_rejected_before_any_lookup() {
    for path in ["", "build.log", "./build.log", "/", "out/build.log/"] {
        assert_eq!(
            validate_marker_path(path),
            Err(MarkerReadDefect::MalformedPath),
            "{path:?} is not a usable marker path"
        );
        // Even a scanned "present" cannot rescue a malformed path: the file
        // that was found is not the file the contract named.
        assert!(!read_marker(path, MarkerLocation::Scanned { present: true }).ran());
    }
    assert_eq!(
        validate_marker_path("/work/out/build.log"),
        Ok(()),
        "absolute paths with a parent are usable"
    );
}

#[test]
fn liveness_from_a_bad_path_is_blocked_not_within_limit() {
    // The fold this prevents: a check pointed at a directory that is empty by
    // definition would read every job as healthy (or, past the limit, as
    // wedged) while consulting nothing.
    let obs = step_observation(LIMIT + 50 * MIN, false);
    let outcome = step_liveness_at("", MarkerLocation::DirectoryMissing, &obs);
    assert!(!outcome.ran());
    assert_eq!(outcome.verdict(), None);
    let line = outcome.line(&model_call_marker());
    assert!(line.contains("could not run"), "{line}");
    assert!(!line.contains("never returned"), "{line}");
}

#[test]
fn liveness_from_a_readable_path_still_reaches_the_wedge_verdict() {
    let obs = step_observation(LIMIT + 50 * MIN, false);
    let outcome = step_liveness_at(
        "/work/out/issue-4259/build.log",
        MarkerLocation::Scanned { present: false },
        &obs,
    );
    assert!(outcome.ran());
    assert!(outcome.verdict().unwrap().wedged());
    assert!(outcome
        .line(&model_call_marker())
        .contains("never returned"));
}

#[test]
fn a_short_run_with_a_read_absence_is_still_not_wedged() {
    let obs = step_observation(4 * MIN, false);
    let outcome = step_liveness_at(
        "/work/out/build.log",
        MarkerLocation::Scanned { present: false },
        &obs,
    );
    assert_eq!(outcome.verdict(), Some(StepLiveness::WithinLimit));
    assert!(!outcome.verdict().unwrap().wedged());
}

// --- Invariant 4: a detector must not be confoundable by buffering --------

#[test]
fn an_output_size_snapshot_never_discriminates() {
    // 0 bytes is what 22 working agents looked like; 264 bytes is what the
    // wrapper log looked like for everybody.
    for bytes in [0u64, 264, 1_000_000] {
        let d = Detector::OutputSnapshot { bytes };
        assert!(!d.discriminates(), "{bytes} bytes is not liveness");
        assert_eq!(d.defect(), Some(ConfoundedReason::Buffering));
        let err = usable_for_liveness(&d).unwrap_err();
        assert!(err.as_str().contains("flushes at exit"), "{err:?}");
    }
}

#[test]
fn a_wrapper_log_mtime_never_discriminates() {
    // Nearly cancelled 19 healthy-reading jobs on exactly this signal.
    let d = Detector::OutputMtime {
        mtime_age: 4 * Duration::from_secs(3600),
    };
    assert!(!d.discriminates());
    assert_eq!(d.defect(), Some(ConfoundedReason::FrozenMtime));
    assert!(usable_for_liveness(&d)
        .unwrap_err()
        .as_str()
        .contains("never grows"));
}

#[test]
fn a_step_overrun_discriminates_and_reports_stuck() {
    let d = Detector::StepOverrun(step_observation(LIMIT + 50 * MIN, false));
    assert!(d.discriminates());
    assert_eq!(d.defect(), None);
    assert_eq!(usable_for_liveness(&d), Ok(DetectionVerdict::Stuck));
}

#[test]
fn a_step_overrun_reports_finished_and_no_evidence() {
    let done = Detector::StepOverrun(step_observation(3 * Duration::from_secs(3600), true));
    assert_eq!(usable_for_liveness(&done), Ok(DetectionVerdict::Finished));
    let early = Detector::StepOverrun(step_observation(2 * MIN, false));
    assert_eq!(
        usable_for_liveness(&early),
        Ok(DetectionVerdict::NoEvidence)
    );
}

#[test]
fn a_dashboard_screen_rejects_the_three_useless_panels_and_keeps_one() {
    // The four signals from the incident table, in the order they were read.
    let panels = vec![
        Detector::OutputMtime {
            mtime_age: 4 * Duration::from_secs(3600),
        },
        Detector::OutputSnapshot { bytes: 264 },
        Detector::OutputSnapshot { bytes: 0 },
        Detector::StepOverrun(step_observation(LIMIT + 50 * MIN, false)),
    ];
    let rejected = reject_confoundable(&panels);
    assert_eq!(rejected.len(), 3, "three of four cannot support liveness");
    assert!(rejected.iter().all(|l| l.contains("not a liveness signal")));
}

#[test]
fn an_all_confoundable_board_rejects_every_panel() {
    let panels = vec![
        Detector::OutputSnapshot { bytes: 0 },
        Detector::OutputMtime {
            mtime_age: Duration::from_secs(1),
        },
    ];
    assert_eq!(reject_confoundable(&panels).len(), 2);
    assert!(reject_confoundable(&[]).is_empty());
}

// --- Invariant 5: occupancy is not work -----------------------------------

fn incident_fleet() -> FleetSignal {
    FleetSignal {
        running: 22,
        capacity: 22,
        // Patches per hour had collapsed to 2 while occupancy read 100%.
        throughput_per_hour: 2,
        expected_per_hour: 22,
    }
}

#[test]
fn a_full_fleet_at_two_percent_output_reads_as_occupied_not_working() {
    let signal = incident_fleet();
    assert_eq!(signal.occupancy_pct(), Some(100));
    assert!(signal.saturated());
    assert!(signal.output_collapsed());
    let verdict = fleet_verdict(&signal);
    assert_eq!(
        verdict,
        FleetVerdict::OccupiedNotWorking { occupancy_pct: 100 }
    );
    assert!(!verdict.working());
    let line = verdict.line(&signal);
    // Occupancy and throughput on one line, so neither is read alone.
    assert!(line.contains("100% occupied"), "{line}");
    assert!(line.contains("2/h"), "{line}");
    assert!(line.contains("occupancy is not work"), "{line}");
}

#[test]
fn a_full_fleet_at_expected_output_reads_as_working() {
    let signal = FleetSignal {
        throughput_per_hour: 20,
        ..incident_fleet()
    };
    assert!(!signal.output_collapsed());
    assert!(fleet_verdict(&signal).working());
}

#[test]
fn a_saturation_reading_without_a_throughput_companion_is_not_health() {
    let signal = FleetSignal {
        expected_per_hour: 0,
        ..incident_fleet()
    };
    let verdict = fleet_verdict(&signal);
    assert_eq!(verdict, FleetVerdict::NoThroughputCompanion);
    assert!(!verdict.working());
    assert!(verdict
        .line(&signal)
        .contains("occupancy reading, not a health reading"));
}

#[test]
fn unknown_capacity_is_never_reported_as_zero_percent_occupied() {
    let signal = FleetSignal {
        running: 0,
        capacity: 0,
        throughput_per_hour: 0,
        expected_per_hour: 22,
    };
    assert_eq!(signal.occupancy_pct(), None);
    assert!(!signal.saturated());
    assert_eq!(fleet_verdict(&signal), FleetVerdict::Unknown);
    assert!(fleet_verdict(&signal)
        .line(&signal)
        .contains("no fleet verdict"));
}

#[test]
fn a_half_occupied_fleet_is_not_saturated_however_low_the_output() {
    let signal = FleetSignal {
        running: 10,
        capacity: 22,
        throughput_per_hour: 0,
        expected_per_hour: 22,
    };
    assert!(!signal.saturated());
    assert!(signal.output_collapsed());
    // Occupancy is not the alarm at low occupancy; that is a different
    // question, and this verdict does not pretend otherwise.
    assert!(fleet_verdict(&signal).working());
}

#[test]
fn collapse_threshold_is_half_the_baseline_and_the_boundary_is_inclusive() {
    let signal = FleetSignal {
        throughput_per_hour: 11,
        ..incident_fleet()
    };
    // 11 of 22 is exactly half: at the baseline ratio, not below it.
    assert!(!signal.output_collapsed());
    let below = FleetSignal {
        throughput_per_hour: 10,
        ..incident_fleet()
    };
    assert!(below.output_collapsed());
}

// --- The spec-facing half: commission a progress contract too -------------

fn long_commission() -> WorkerCommission {
    WorkerCommission {
        run_limit: LIMIT,
        completion_artifact: "changes.patch".to_string(),
        emits_while_running: None,
    }
}

#[test]
fn a_completion_only_contract_for_an_hour_long_worker_is_rejected() {
    // "Writes changes.patch on success" says nothing while the run is live.
    let verdict = commission_verdict(&long_commission(), MIN);
    assert_eq!(
        verdict,
        CommissionVerdict::CompletionOnly { run_limit: LIMIT }
    );
    assert!(!verdict.complete());
    let line = verdict.line();
    assert!(line.contains("emits while running"), "{line}");
    assert!(line.contains("completion contract only"), "{line}");
}

#[test]
fn a_worker_with_a_flushing_progress_contract_passes() {
    let commission = WorkerCommission {
        emits_while_running: Some(ProgressEmission {
            artifact: "progress.jsonl".to_string(),
            every: 5 * MIN,
            emitter: Emitter::Task,
            sink: OutputSink::FlushPerStep,
        }),
        ..long_commission()
    };
    let verdict = commission_verdict(&commission, MIN);
    assert_eq!(verdict, CommissionVerdict::ContractComplete);
    assert!(verdict.complete());
}

#[test]
fn a_progress_contract_that_writes_into_a_buffer_is_inert() {
    // The emission exists, is promised every 5 minutes, and lands in a
    // stdio buffer flushed at exit: inert as instrumentation.
    let commission = WorkerCommission {
        emits_while_running: Some(ProgressEmission {
            artifact: "agent.out".to_string(),
            every: 5 * MIN,
            emitter: Emitter::Task,
            sink: OutputSink::RedirectedStdout,
        }),
        ..long_commission()
    };
    let verdict = commission_verdict(&commission, MIN);
    assert_eq!(
        verdict,
        CommissionVerdict::ContractInert(Instrumented::BufferedUntilExit)
    );
    assert!(!verdict.complete());
    assert!(verdict.line().contains("inert"), "{}", verdict.line());
}

#[test]
fn a_progress_interval_longer_than_the_run_is_inert() {
    let commission = WorkerCommission {
        emits_while_running: Some(ProgressEmission {
            artifact: "heartbeat".to_string(),
            every: 2 * Duration::from_secs(3600),
            emitter: Emitter::Wrapper,
            sink: OutputSink::WrapperArtifact,
        }),
        ..long_commission()
    };
    assert!(!commission_verdict(&commission, MIN).complete());
}

#[test]
fn a_run_inside_the_watch_horizon_may_ship_with_completion_only() {
    let verdict = commission_verdict(&long_commission(), LIMIT);
    assert_eq!(verdict, CommissionVerdict::ShortRun);
    assert!(verdict.complete());
}

#[test]
fn an_emission_reuses_the_run_time_check() {
    // The spec-time and run-time halves agree on one implementation, so a
    // contract cannot pass the spec review and then read as uninstrumented.
    let emission = ProgressEmission {
        artifact: "progress.jsonl".to_string(),
        every: 5 * MIN,
        emitter: Emitter::Wrapper,
        sink: OutputSink::WrapperArtifact,
    };
    assert_eq!(
        instrumented(&emission.as_contract(LIMIT)),
        Instrumented::Observable
    );
    let buffered = ProgressEmission {
        sink: OutputSink::RedirectedStdout,
        ..emission
    };
    assert_eq!(
        instrumented(&buffered.as_contract(LIMIT)),
        Instrumented::BufferedUntilExit
    );
}
