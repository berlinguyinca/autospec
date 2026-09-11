//! Bounded liveness for a pool of generation workers (issue #4372).
//!
//! The regression this suite pins: the fleet gated pool membership on `GET
//! /health`, which answered 200 while generation was wedged, and its one
//! detector that *could* see the wedge (the completion probe) fired 237 times
//! and was ignored as "busy is not dead". Each test below maps to one of the
//! issue's invariants; the final test reconstructs the incident end to end.

use autospec_core::aar::inferweave::{
    classify_probe, DeadReason, LivenessProbe, PoolAction, ProbeCheck, ProbeSignal,
};
use autospec_core::aar::liveness_bound::{
    bound_decision, exercises_client_path, liveness_blindspot, Blindspot, BoundedVerdict,
    DetectorFinding, DetectorLedger, IndependentSignal, LivenessPolicy, LivenessPolicyFinding,
    MitigationClaim, MitigationFinding, RotationReport, DEFAULT_INCONCLUSIVE_BOUND_SECS,
};

/// The incident's probe design: a constant-cost `GET /health` liveness check
/// plus a completion in the verification slot. It passes `validate` (the
/// pre-incident rules) and is exactly the design that was blind.
fn probe() -> LivenessProbe {
    LivenessProbe {
        interval_secs: 300,
        deadline_secs: 5, // the incident's probeTimeout = 5 * time.Second
        liveness: ProbeCheck::Health,
        verification: Some(ProbeCheck::Completion {
            starvation_argument: "on a busy worker the probe queues behind \
                production traffic, so its deadline miss measures the queue, not \
                the worker"
                .to_string(),
        }),
        expected_identity: Some("worker-7".to_string()),
    }
}

fn miss() -> ProbeSignal {
    ProbeSignal::DeadlineExceeded
}

// ── Invariant 1: a liveness check must exercise the path the client uses ───

#[test]
fn only_a_completion_exercises_the_client_path() {
    assert!(exercises_client_path(&ProbeCheck::Completion {
        starvation_argument: "see above".to_string()
    }));
    assert!(!exercises_client_path(&ProbeCheck::Health));
    assert!(!exercises_client_path(&ProbeCheck::ModelList));
}

#[test]
fn the_health_and_model_list_checks_are_blind() {
    assert_eq!(
        liveness_blindspot(&ProbeCheck::Health),
        Some(Blindspot::ControlPlaneOnly)
    );
    assert_eq!(
        liveness_blindspot(&ProbeCheck::ModelList),
        Some(Blindspot::ControlPlaneOnly)
    );
    assert!(Blindspot::ControlPlaneOnly.line().contains("client's path"));
}

#[test]
fn a_completion_check_is_not_blind() {
    assert_eq!(
        liveness_blindspot(&ProbeCheck::Completion {
            starvation_argument: "see above".to_string()
        }),
        None
    );
}

// ── Invariant 3: "busy is not dead" is a bound, not a verdict ──────────────

/// The old behavior, pinned as the regression: a deadline miss is
/// inconclusive and kept, no matter how long the streak. This is what hid the
/// wedge behind 237 warnings.
#[test]
fn classify_probe_keeps_a_deadline_miss_forever() {
    let p = probe();
    let verdict = classify_probe(&p, &miss());
    assert!(matches!(
        verdict,
        autospec_core::aar::inferweave::ProbeVerdict::Inconclusive
    ));
    assert_eq!(verdict.pool_action(), PoolAction::Keep);
}

/// The incident: a completion that timed out for 12 hours with the GPU at 0%.
/// The idle GPU contradicts "busy" and is decisive on its own.
#[test]
fn an_idle_gpu_contradicts_busy_and_evicts() {
    let p = probe();
    let now = 12 * 3600;

    let verdict = bound_decision(
        &p,
        &miss(),
        Some(0),
        now,
        Some(IndependentSignal::Idle),
        600,
    );

    assert_eq!(verdict, BoundedVerdict::WedgedIdleGpu);
    assert_eq!(verdict.pool_action(), PoolAction::Evict);
    assert!(verdict.decisive());
}

/// An active GPU proves the worker is doing real work: the probe is queued
/// behind production traffic (the starvation argument). Keep it, no matter how
/// long the streak.
#[test]
fn an_active_gpu_overrides_the_bound_and_keeps() {
    let p = probe();
    let now = 12 * 3600;

    let verdict = bound_decision(
        &p,
        &miss(),
        Some(0),
        now,
        Some(IndependentSignal::Active),
        600,
    );

    assert_eq!(verdict, BoundedVerdict::Busy);
    assert_eq!(verdict.pool_action(), PoolAction::Keep);
    assert!(!verdict.decisive());
}

/// With no independent signal, the bound is the decision. A miss within the
/// bound is still "busy is not dead" and is kept.
#[test]
fn a_fresh_miss_with_no_signal_is_still_busy() {
    let p = probe();
    let now = 1000;

    let verdict = bound_decision(&p, &miss(), Some(now - 5), now, None, 600);

    assert_eq!(verdict, BoundedVerdict::Busy);
    assert_eq!(verdict.pool_action(), PoolAction::Keep);
}

/// With no independent signal, a miss that has persisted beyond the bound is
/// decided a wedge rather than kept forever: "not by refusing to decide".
#[test]
fn a_miss_beyond_the_bound_with_no_signal_is_wedged() {
    let p = probe();
    let now = 1000;

    let verdict = bound_decision(&p, &miss(), Some(now - 600), now, None, 600);

    assert_eq!(verdict, BoundedVerdict::WedgedBeyondBound);
    assert_eq!(verdict.pool_action(), PoolAction::Evict);
    assert!(verdict.decisive());
}

/// The bound is inclusive: a miss exactly at the bound is already a wedge, one
/// second short of it is not.
#[test]
fn the_bound_is_inclusive_at_the_edge() {
    let p = probe();
    let now = 1000;

    assert_eq!(
        bound_decision(&p, &miss(), Some(now - 599), now, None, 600),
        BoundedVerdict::Busy
    );
    assert_eq!(
        bound_decision(&p, &miss(), Some(now - 600), now, None, 600),
        BoundedVerdict::WedgedBeyondBound
    );
}

/// The streak is consecutive: an answer resets it, so a worker that answered
/// and then missed once is not held to the bound.
#[test]
fn a_live_answer_is_alive_and_resets_the_streak() {
    let p = probe();

    let verdict = bound_decision(
        &p,
        &ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
        None,
        1000,
        None,
        600,
    );

    assert_eq!(verdict, BoundedVerdict::Alive);
    assert_eq!(verdict.pool_action(), PoolAction::Keep);
}

/// A definitive failure classifies as dead, exactly as `classify_probe` does.
#[test]
fn a_definitive_failure_still_evicts() {
    let p = probe();
    for signal in [
        ProbeSignal::Refused,
        ProbeSignal::Reset,
        ProbeSignal::ErrorStatus { status: 503 },
    ] {
        let verdict = bound_decision(&p, &signal, None, 1000, None, 600);
        assert_eq!(verdict.pool_action(), PoolAction::Evict);
        assert!(verdict.decisive());
    }

    // An identity mismatch is definitive too, even though it is a Live signal.
    let verdict = bound_decision(
        &p,
        &ProbeSignal::Live {
            identity: Some("worker-9".to_string()),
        },
        None,
        1000,
        None,
        600,
    );
    assert_eq!(verdict, BoundedVerdict::Dead(DeadReason::IdentityMismatch));
}

// ── Invariants 1 + 3 combined: a sound policy gates on a bounded completion ─

#[test]
fn the_incident_policy_is_blind() {
    // The fleet gated on GET /health, with or without a bound on a check that
    // does not exercise the client's path is blind.
    for bounded in [true, false] {
        let policy = LivenessPolicy {
            gating: ProbeCheck::Health,
            bounded,
        };
        assert_eq!(
            policy.findings(),
            vec![LivenessPolicyFinding::BlindGatingCheck],
            "bounded={bounded}"
        );
    }
}

#[test]
fn an_unbounded_completion_keeps_a_wedged_worker_forever() {
    let policy = LivenessPolicy {
        gating: ProbeCheck::Completion {
            starvation_argument: "see above".to_string(),
        },
        bounded: false,
    };
    assert_eq!(
        policy.findings(),
        vec![LivenessPolicyFinding::UnboundedCompletion]
    );
}

#[test]
fn a_bounded_completion_is_the_sound_policy() {
    let policy = LivenessPolicy {
        gating: ProbeCheck::Completion {
            starvation_argument: "see above".to_string(),
        },
        bounded: true,
    };
    assert!(policy.findings().is_empty(), "got {:?}", policy.findings());
}

// ── Invariant 2: a detector that fires must act ────────────────────────────

#[test]
fn a_detector_that_fired_without_acting_is_ignored() {
    let detector = DetectorLedger {
        name: "probe-timed-out".to_string(),
        firings: 237,
        actions: 0,
    };

    assert!(detector.ignored());
    assert_eq!(
        detector.finding(),
        Some(DetectorFinding::Ignored { firings: 237 })
    );
    let line = detector.line();
    assert!(line.contains("237 firing(s)"), "got: {line}");
    assert!(line.contains("0 action(s)"), "got: {line}");
    assert!(line.contains("worse than no detector"), "got: {line}");
}

#[test]
fn a_detector_that_acted_on_its_firings_is_not_ignored() {
    let detector = DetectorLedger {
        name: "probe-timed-out".to_string(),
        firings: 237,
        actions: 1,
    };
    assert!(!detector.ignored());
    assert_eq!(detector.finding(), None);
    assert!(detector.line().contains("1 action(s)"));
}

#[test]
fn a_detector_that_never_fired_is_not_ignored() {
    let detector = DetectorLedger {
        name: "never-fired".to_string(),
        firings: 0,
        actions: 0,
    };
    assert!(!detector.ignored());
    assert_eq!(detector.finding(), None);
}

// ── Invariant 4: rotation is mitigation, and its recurrence is tracked ─────

#[test]
fn a_rotation_reported_as_mitigation_with_recurrence_is_sound() {
    let report = RotationReport {
        rotations: 5,
        recurrence: 2,
        claim: MitigationClaim::Mitigation,
    };
    assert!(report.findings().is_empty());
    let line = report.line();
    assert!(line.contains("mitigation"), "got: {line}");
    assert!(line.contains("recurrence 2"), "got: {line}");
    assert!(line.contains("not a cure"), "got: {line}");
}

#[test]
fn a_rotation_claimed_as_a_fix_is_a_finding() {
    let report = RotationReport {
        rotations: 5,
        recurrence: 2,
        claim: MitigationClaim::Fix,
    };
    assert_eq!(report.findings(), vec![MitigationFinding::ReportedAsFix]);
    assert!(report.findings()[0].line().contains("not a fix"));
}

#[test]
fn a_repeated_rotation_with_no_recurrence_is_untracked() {
    let report = RotationReport {
        rotations: 5,
        recurrence: 0,
        claim: MitigationClaim::Mitigation,
    };
    assert_eq!(
        report.findings(),
        vec![MitigationFinding::RecurrenceUntracked { rotations: 5 }]
    );
}

/// A single rotation has no recurrence yet, so a recurrence of 0 is correct,
/// not untracked.
#[test]
fn a_single_rotation_with_no_recurrence_is_not_untracked() {
    let report = RotationReport {
        rotations: 1,
        recurrence: 0,
        claim: MitigationClaim::Mitigation,
    };
    assert!(report.findings().is_empty());
}

// ── The incident, reconstructed end to end ─────────────────────────────────

#[test]
fn the_incident_the_wedged_worker_is_now_evicted_not_kept() {
    let p = probe();

    // Invariant 1: the fleet's liveness check (GET /health) answered 200
    // while generation returned 000. It is blind to the wedge.
    assert_eq!(
        liveness_blindspot(&p.liveness),
        Some(Blindspot::ControlPlaneOnly)
    );

    // Invariant 3: the completion probe timed out for 12 hours (a streak of
    // 12h) with the GPU at 0%. classify_probe keeps it forever; the bound
    // decides it a wedge.
    let now = 12 * 3600;
    assert_eq!(classify_probe(&p, &miss()).pool_action(), PoolAction::Keep);

    let verdict = bound_decision(
        &p,
        &miss(),
        Some(0),
        now,
        Some(IndependentSignal::Idle),
        DEFAULT_INCONCLUSIVE_BOUND_SECS,
    );
    assert_eq!(verdict, BoundedVerdict::WedgedIdleGpu);
    assert_eq!(verdict.pool_action(), PoolAction::Evict);
    assert!(verdict.decisive());

    // Invariant 2: the detector fired 237 times and took 0 actions. That is
    // now a finding, not a quiet keep.
    let detector = DetectorLedger {
        name: "probe-timed-out".to_string(),
        firings: 237,
        actions: 0,
    };
    assert!(detector.ignored());
    assert_eq!(
        detector.finding(),
        Some(DetectorFinding::Ignored { firings: 237 })
    );

    // Invariant 4: five replacements were rotated; two wedged again within
    // minutes. Reported as mitigation, with the recurrence tracked.
    let rotation = RotationReport {
        rotations: 5,
        recurrence: 2,
        claim: MitigationClaim::Mitigation,
    };
    assert!(rotation.findings().is_empty());
    assert!(rotation.line().contains("mitigation"));
}
