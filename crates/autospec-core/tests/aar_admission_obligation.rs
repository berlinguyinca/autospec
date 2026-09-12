//! A check must run on the path the condition it detects actually takes
//! (issue #4411).
//!
//! The regression this suite pins: a throughput floor was added to
//! admission so a CPU-bound worker could not join the pool, evaluated after
//! the admission completion returned 200. A CPU-bound worker's admission
//! probe times out — the "the probe timed out but the endpoint reports this
//! model; admitting (busy is not dead)" branch — which bypasses the floor
//! entirely. The floor was live, correct, tested, and it caught nothing.
//!
//! Each test below maps to one of the issue's invariants; the final test
//! reconstructs the incident end to end.

use autospec_core::aar::admission_obligation::{
    check_covers, provisional_status, CheckPlacement, ConditionBehavior, MisplacedCheck,
    ProvisionalStatus, DEFAULT_MEASUREMENT_WINDOW_SECS,
};
use autospec_core::aar::inferweave::{
    admit, AdmissionVerdict, LivenessProbe, PoolAction, ProbeCheck, ProbeSignal, RegistrationProbe,
};

/// The admission probe for `identity`: a one-token completion liveness
/// step, whose cost scales with load, and the expected identity of the
/// worker being (re-)registered.
fn probe(identity: &str) -> LivenessProbe {
    LivenessProbe {
        interval_secs: 300,
        deadline_secs: 5,
        liveness: ProbeCheck::Completion {
            starvation_argument: "on a busy worker the probe queues behind \
                production traffic, so its deadline miss measures the queue, not \
                the worker"
                .to_string(),
        },
        verification: None,
        expected_identity: Some(identity.to_string()),
    }
}

// ── The invariant: a check must run on the path the condition takes ───────

/// The incident, as a predicate: the throughput floor sat on the success
/// branch; the worker it existed to catch times out on the liveness probe.
/// The check never ran against the condition, so it caught nothing.
#[test]
fn a_floor_on_the_success_branch_never_sees_a_worker_that_times_out() {
    assert!(!check_covers(
        CheckPlacement::SuccessBranch,
        ConditionBehavior::TimesOut
    ));
    assert_eq!(
        MisplacedCheck::find(CheckPlacement::SuccessBranch, ConditionBehavior::TimesOut),
        Some(MisplacedCheck {
            placement: CheckPlacement::SuccessBranch,
            condition: ConditionBehavior::TimesOut,
        })
    );
    let line = MisplacedCheck {
        placement: CheckPlacement::SuccessBranch,
        condition: ConditionBehavior::TimesOut,
    }
    .line();
    assert!(line.contains("success"));
    assert!(line.contains("times out"));
}

/// The three answers the issue names: "it times out", "it errors", "it
/// returns nothing" — a success-branch check is in the wrong place for all
/// three.
#[test]
fn a_success_branch_check_is_wrong_for_every_failing_behavior() {
    for condition in [
        ConditionBehavior::TimesOut,
        ConditionBehavior::Errors,
        ConditionBehavior::ReturnsNothing,
    ] {
        assert!(
            !check_covers(CheckPlacement::SuccessBranch, condition),
            "a success-branch check must not claim to cover a condition that {condition:?}"
        );
        assert!(MisplacedCheck::find(CheckPlacement::SuccessBranch, condition).is_some());
    }
    // And a success-branch check does cover the healthy case.
    assert!(check_covers(
        CheckPlacement::SuccessBranch,
        ConditionBehavior::Succeeds
    ));
}

/// A check on each branch covers exactly the behaviors that branch takes,
/// and a check on every branch covers all of them.
#[test]
fn a_check_on_the_right_branch_covers_its_condition() {
    assert!(check_covers(
        CheckPlacement::TimeoutBranch,
        ConditionBehavior::TimesOut
    ));
    // "Returns nothing" is observed as an absence: it lands on the timeout
    // branch, not the success branch.
    assert!(check_covers(
        CheckPlacement::TimeoutBranch,
        ConditionBehavior::ReturnsNothing
    ));
    assert!(check_covers(
        CheckPlacement::ErrorBranch,
        ConditionBehavior::Errors
    ));
    for condition in [
        ConditionBehavior::Succeeds,
        ConditionBehavior::TimesOut,
        ConditionBehavior::Errors,
        ConditionBehavior::ReturnsNothing,
    ] {
        assert!(check_covers(CheckPlacement::EveryBranch, condition));
        assert!(!check_covers(CheckPlacement::Nowhere, condition));
    }
    // The other branches are still misses.
    assert!(!check_covers(
        CheckPlacement::ErrorBranch,
        ConditionBehavior::TimesOut
    ));
    assert!(!check_covers(
        CheckPlacement::TimeoutBranch,
        ConditionBehavior::Errors
    ));
    assert!(!check_covers(
        CheckPlacement::SuccessBranch,
        ConditionBehavior::Errors
    ));
}

// ── The fix shape: the bypass carries an obligation, not an exit ──────────

/// The incident's branch, as admission: the liveness step timed out but the
/// endpoint reports the requested model. The admission is provisional, not
/// a plain exit — the worker is marked never measured.
#[test]
fn the_timeout_branch_admits_provisionally_with_an_obligation() {
    let cpu_bound = RegistrationProbe {
        identity: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
        observed_models: vec!["qwen3.8-27b".to_string()],
        liveness: ProbeSignal::DeadlineExceeded,
    };

    let verdict = admit(&probe("worker-7"), &cpu_bound, "qwen3.8-27b");

    assert_eq!(verdict, AdmissionVerdict::AdmittedProvisionally);
    assert!(verdict.is_admitted());
    assert!(verdict.requires_measurement());
}

/// The window: a never-measured worker is owed its measurement until the
/// window closes, and is not evicted for being busy inside it.
#[test]
fn a_never_measured_worker_is_owed_its_measurement_inside_the_window() {
    let status = provisional_status(1_000, None, 1_500, DEFAULT_MEASUREMENT_WINDOW_SECS);
    assert_eq!(status, ProvisionalStatus::Owed);
    assert_eq!(status.pool_action(), PoolAction::Keep);
    assert!(!status.counts_as_healthy());
}

/// The window closes: a worker that can never complete a measurement is not
/// the same as one that was merely busy when asked. It stops counting as
/// healthy, and the pool may evict it.
#[test]
fn a_never_measured_worker_past_the_window_is_unmeasurable() {
    let at_deadline = provisional_status(
        1_000,
        None,
        1_000 + DEFAULT_MEASUREMENT_WINDOW_SECS,
        DEFAULT_MEASUREMENT_WINDOW_SECS,
    );
    assert_eq!(at_deadline, ProvisionalStatus::Unmeasurable);
    assert_eq!(at_deadline.pool_action(), PoolAction::Evict);
    assert!(!at_deadline.counts_as_healthy());

    let past_deadline = provisional_status(
        1_000,
        None,
        1_000 + DEFAULT_MEASUREMENT_WINDOW_SECS + 60,
        600,
    );
    assert_eq!(past_deadline, ProvisionalStatus::Unmeasurable);
}

/// A completed measurement makes the worker healthy at any age — including
/// after the window would have closed.
#[test]
fn a_completed_measurement_makes_the_worker_healthy() {
    let late = provisional_status(
        1_000,
        Some(1_000 + DEFAULT_MEASUREMENT_WINDOW_SECS + 120),
        1_000 + DEFAULT_MEASUREMENT_WINDOW_SECS + 600,
        DEFAULT_MEASUREMENT_WINDOW_SECS,
    );
    assert_eq!(late, ProvisionalStatus::Healthy);
    assert_eq!(late.pool_action(), PoolAction::Keep);
    assert!(late.counts_as_healthy());
}

/// A clock that rewinds is zero age, never an underflow: the worker stays
/// owed, not unmeasurable.
#[test]
fn a_rewound_clock_is_zero_age() {
    let status = provisional_status(2_000, None, 1_000, DEFAULT_MEASUREMENT_WINDOW_SECS);
    assert_eq!(status, ProvisionalStatus::Owed);
}

/// The window is bounded, not a perpetual keep: a never-measured worker
/// past the default window is decided against, so "never measured" cannot
/// persist forever.
#[test]
fn the_measurement_window_is_bounded() {
    let admitted_at = 100;
    assert_eq!(
        provisional_status(
            admitted_at,
            None,
            admitted_at + DEFAULT_MEASUREMENT_WINDOW_SECS,
            DEFAULT_MEASUREMENT_WINDOW_SECS
        ),
        ProvisionalStatus::Unmeasurable
    );
    // And a worker that completes the measurement inside the window is
    // healthy.
    assert_eq!(
        provisional_status(
            admitted_at,
            Some(admitted_at + 1),
            admitted_at + DEFAULT_MEASUREMENT_WINDOW_SECS,
            DEFAULT_MEASUREMENT_WINDOW_SECS
        ),
        ProvisionalStatus::Healthy
    );
}

// ── The incident, end to end ──────────────────────────────────────────────

/// Two workers register for the same model. The GPU worker's admission
/// completion returns 200: measured, admitted, healthy. The CPU-bound
/// worker's completion probe times out: admitted through the
/// "busy is not dead" branch.
///
/// The throughput floor sat on the 200 branch. It ran against the GPU
/// worker and never against the CPU-bound one — `check_covers` says the
/// floor was misplaced — and both workers registered anyway.
///
/// The fix: the CPU-bound worker's admission is provisional and carries the
/// obligation. It never completes a measurement, so when the window closes
/// it stops counting as healthy and is evicted — while the merely busy
/// worker that completes a measurement on its second attempt stays.
#[test]
fn the_floor_on_the_200_branch_caught_nothing_the_obligation_catches_the_cpu_worker() {
    // The floor: a check on the success branch, guarding against a
    // condition — a CPU-bound worker — that times out.
    assert_eq!(
        MisplacedCheck::find(CheckPlacement::SuccessBranch, ConditionBehavior::TimesOut)
            .map(|finding| finding.line()),
        Some(
            "a check on the success branch never runs against a condition that times out: \
             a check must run on the path the condition it detects actually takes"
                .to_string()
        )
    );

    // The GPU worker: the completion returns 200, the floor runs, it is
    // measured and healthy.
    let gpu = RegistrationProbe {
        identity: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
        observed_models: vec!["qwen3.8-27b".to_string()],
        liveness: ProbeSignal::Live {
            identity: Some("worker-7".to_string()),
        },
    };
    let gpu_admission = admit(&probe("worker-7"), &gpu, "qwen3.8-27b");
    assert_eq!(gpu_admission, AdmissionVerdict::Admitted);
    assert!(!gpu_admission.requires_measurement());

    // The CPU-bound worker: a different worker, probed with its own
    // expected identity, but so slow that its admission probe times out.
    // The old code admitted it outright; the floor on the 200 branch never
    // ran against it.
    let cpu = RegistrationProbe {
        identity: ProbeSignal::Live {
            identity: Some("worker-9".to_string()),
        },
        observed_models: vec!["qwen3.8-27b".to_string()],
        liveness: ProbeSignal::DeadlineExceeded,
    };
    let cpu_admission = admit(&probe("worker-9"), &cpu, "qwen3.8-27b");
    assert_eq!(cpu_admission, AdmissionVerdict::AdmittedProvisionally);
    assert!(cpu_admission.requires_measurement());

    // Halfway through the window, never measured: still owed, still in the
    // pool.
    let admitted_at = 10_000;
    assert_eq!(
        provisional_status(
            admitted_at,
            None,
            admitted_at + DEFAULT_MEASUREMENT_WINDOW_SECS / 2,
            DEFAULT_MEASUREMENT_WINDOW_SECS
        ),
        ProvisionalStatus::Owed
    );

    // The window closes with no measurement: not the same as a worker that
    // was merely busy when asked.
    let past = provisional_status(
        admitted_at,
        None,
        admitted_at + DEFAULT_MEASUREMENT_WINDOW_SECS,
        DEFAULT_MEASUREMENT_WINDOW_SECS,
    );
    assert_eq!(past, ProvisionalStatus::Unmeasurable);
    assert_eq!(past.pool_action(), PoolAction::Evict);
    assert!(!past.counts_as_healthy());

    // The control case: a merely busy worker whose first attempt queued
    // behind traffic completes the measurement on its second attempt and
    // counts as healthy.
    let busy = provisional_status(
        admitted_at,
        Some(admitted_at + 300),
        admitted_at + DEFAULT_MEASUREMENT_WINDOW_SECS,
        DEFAULT_MEASUREMENT_WINDOW_SECS,
    );
    assert_eq!(busy, ProvisionalStatus::Healthy);
    assert!(busy.counts_as_healthy());
}
