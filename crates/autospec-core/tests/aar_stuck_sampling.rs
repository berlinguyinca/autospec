//! Unconditional progress sampling for stuck-worker detection (issue #4401).
//!
//! The regression this suite pins: the gateway's stuck-worker detector
//! (metabolomics-us/inferweave-gateway #133/#136) sampled token progress
//! only from inside the probe-timeout branch, so detection was conditional
//! on a second, unrelated event that rarely holds. Worker `23015220` was
//! genuinely stuck — counters frozen across a 35s window while it failed a
//! real generation request — while the gateway log for the same period
//! counted 3 probe timeouts (spread across different workers), 0 stalled
//! samples, and 0 stuck verdicts. Each test below maps to one of the
//! issue's invariants; the final test reconstructs the incident end to end.

use autospec_core::aar::stuck_sampling::{
    ProgressSample, SamplingDesign, SamplingFinding, SignalCost, StuckTracker, SweepOutcome,
    DEFAULT_STUCK_THRESHOLD,
};

fn sample(tokens: u64, failing_to_serve: bool) -> ProgressSample {
    ProgressSample {
        tokens,
        failing_to_serve,
    }
}

// ── Invariant 1: do not gate a cheap, reliable signal ─────────────────────

/// The incident's detector design: the token counter (a small GET off the
/// work queue, and the truth about frozen counters) sampled only from
/// inside the probe-timeout branch. The trigger became the detector's true
/// sensitivity.
#[test]
fn the_incidents_design_gates_a_cheap_reliable_signal() {
    let design = SamplingDesign::gated_by_probe_failure();
    assert_eq!(
        design.findings(),
        vec![SamplingFinding::GatedSignal],
        "the #133/#136 design must be flagged"
    );
    assert!(SamplingFinding::GatedSignal
        .line()
        .contains("true sensitivity"));
}

/// The fix: the same signal sampled for every live worker on every sweep.
#[test]
fn unconditional_sampling_of_a_cheap_reliable_signal_is_sound() {
    let design = SamplingDesign::unconditional_sampling();
    assert!(design.findings().is_empty());
}

/// An expensive signal may be gated: that is how a completion probe — which
/// runs the model and queues behind production traffic — is supposed to be
/// sampled.
#[test]
fn an_expensive_signal_may_be_gated() {
    let probe = SamplingDesign {
        cost: SignalCost::Expensive,
        reliable: false,
        unconditional: false,
    };
    assert!(probe.findings().is_empty());
}

/// A cheap but unreliable signal is not worth sampling unconditionally; the
/// invariant only protects signals that are cheap AND reliable.
#[test]
fn a_cheap_unreliable_signal_may_be_gated() {
    let design = SamplingDesign {
        cost: SignalCost::Cheap,
        reliable: false,
        unconditional: false,
    };
    assert!(design.findings().is_empty());
}

// ── Invariant 2: progress is a difference ─────────────────────────────────

/// A first sample can never conclude anything, even for a worker that is
/// failing to serve: there is no previous counter to take a difference
/// against.
#[test]
fn a_first_sample_is_a_baseline_and_concludes_nothing() {
    let mut t = StuckTracker::with_default_threshold();

    let obs = t.observe("w-1", sample(100, true));
    assert_eq!(obs.outcome, SweepOutcome::Baseline);
    assert_eq!(obs.stalled_streak, 0);
    assert!(!obs.evict);
}

/// A counter that moved backwards (the worker's process restarted)
/// re-baselines and does not inherit the old streak.
#[test]
fn a_backwards_counter_rebaselines_and_does_not_inherit_the_streak() {
    let mut t = StuckTracker::new(2);
    t.observe("w-1", sample(100, false));
    let stalled = t.observe("w-1", sample(100, true));
    assert_eq!(stalled.stalled_streak, 1);

    let obs = t.observe("w-1", sample(5, false));
    assert_eq!(obs.outcome, SweepOutcome::Baseline);
    assert_eq!(obs.stalled_streak, 0);

    // One more stalled sample is below the threshold of 2.
    let obs = t.observe("w-1", sample(5, true));
    assert_eq!(obs.stalled_streak, 1);
    assert!(!obs.evict);
}

// ── Invariant 3: the decision ─────────────────────────────────────────────

/// Counters advancing means healthy, whatever the probe said: a worker
/// whose probe times out but whose counter advances is not stalled and the
/// streak resets.
#[test]
fn advancing_counters_are_healthy_whatever_the_probe_said() {
    let mut t = StuckTracker::new(2);
    t.observe("w-1", sample(100, false));
    let stalled = t.observe("w-1", sample(100, true));
    assert_eq!(stalled.outcome, SweepOutcome::Stalled);
    assert_eq!(stalled.stalled_streak, 1);

    let obs = t.observe("w-1", sample(140, true));
    assert_eq!(obs.outcome, SweepOutcome::Advancing);
    assert_eq!(obs.stalled_streak, 0);
    assert!(!obs.evict);
}

/// A frozen counter while the worker is serving is not a stalled sample:
/// a stalled sample requires the frozen counter AND the failure to serve.
#[test]
fn a_frozen_counter_while_serving_is_not_stalled() {
    let mut t = StuckTracker::with_default_threshold();
    t.observe("w-1", sample(100, false));

    let obs = t.observe("w-1", sample(100, false));
    assert_eq!(obs.outcome, SweepOutcome::FrozenButServing);
    assert_eq!(obs.stalled_streak, 0);
    assert!(!obs.evict);
}

/// `stuckThreshold` consecutive stalled samples evict; the streak must be
/// consecutive — an advancing sample in between restarts the count.
#[test]
fn consecutive_stalled_samples_up_to_the_threshold_evict() {
    let mut t = StuckTracker::new(3);
    t.observe("w-1", sample(100, false)); // baseline

    let obs = t.observe("w-1", sample(100, true));
    assert_eq!(obs.stalled_streak, 1);
    assert!(!obs.evict);

    let obs = t.observe("w-1", sample(100, true));
    assert_eq!(obs.stalled_streak, 2);
    assert!(!obs.evict);

    // An advancing sample breaks the streak.
    let obs = t.observe("w-1", sample(150, true));
    assert_eq!(obs.outcome, SweepOutcome::Advancing);
    assert_eq!(obs.stalled_streak, 0);

    // Two more stalled samples: still below the threshold.
    t.observe("w-1", sample(150, true));
    let obs = t.observe("w-1", sample(150, true));
    assert_eq!(obs.stalled_streak, 2);
    assert!(!obs.evict);

    // The third consecutive stalled sample: evict.
    let obs = t.observe("w-1", sample(150, true));
    assert_eq!(obs.outcome, SweepOutcome::Stalled);
    assert_eq!(obs.stalled_streak, 3);
    assert!(obs.evict);
}

/// A threshold of 0 is treated as 1: evict on the first stalled sample,
/// never on a baseline.
#[test]
fn a_zero_threshold_evicts_on_the_first_stalled_sample() {
    let mut t = StuckTracker::new(0);
    assert_eq!(t.threshold(), 1);

    assert!(!t.observe("w-1", sample(100, true)).evict);
    let obs = t.observe("w-1", sample(100, true));
    assert!(obs.evict);
}

// ── Invariant 4: telemetry is per worker per cycle ────────────────────────

/// `worker_health` rows exist for every worker every cycle — including the
/// healthy ones — with a progress rate per worker, rather than only rows
/// for workers that happened to trip a probe timeout.
#[test]
fn worker_health_rows_exist_for_every_worker_every_cycle() {
    let mut t = StuckTracker::with_default_threshold();

    // First sweep: baselines only.
    t.observe("busy", sample(100, false));
    t.observe("stuck", sample(200, true));

    let rows = t.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].worker, "busy");
    assert_eq!(rows[0].progress, None); // baseline: no difference yet
    assert_eq!(rows[1].worker, "stuck");
    assert!(rows[1].failing_to_serve);

    // Second sweep: the busy worker advances 40 tokens, the stuck worker
    // moves none. Both get a row.
    let obs = t.observe("busy", sample(140, false));
    assert_eq!(obs.outcome, SweepOutcome::Advancing);
    t.observe("stuck", sample(200, true));

    let rows = t.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].worker, "busy");
    assert_eq!(rows[0].progress, Some(40));
    assert_eq!(rows[0].stalled_streak, 0);
    assert_eq!(rows[1].worker, "stuck");
    assert_eq!(rows[1].tokens, 200);
    assert_eq!(rows[1].progress, Some(0));
    assert_eq!(rows[1].stalled_streak, 1);

    assert_eq!(t.row("busy").unwrap().progress, Some(40));
    assert!(t.row("ghost").is_none());
}

// ── The incident, end to end ──────────────────────────────────────────────

/// Worker `23015220`: genuinely stuck (counters frozen across a 35s window
/// while it failed a real generation request) while three probe timeouts
/// landed on *other* workers. Under the #133/#136 design the detector
/// sampled only inside the probe-timeout branch, so the stuck worker
/// produced zero stalled samples and zero conclusions.
///
/// Under unconditional sampling the stuck worker is sampled every sweep
/// regardless of the probe: after the baseline it accumulates one stalled
/// sample per sweep and evicts at the threshold.
#[test]
fn the_incident_worker_evicts_under_unconditional_sampling() {
    assert_eq!(DEFAULT_STUCK_THRESHOLD, 3);

    let mut t = StuckTracker::with_default_threshold();
    let stuck = "23015220";

    // Sweep 1: the stuck worker fails a real request (its counter is
    // frozen at 999,999); the probe timeouts land elsewhere and are
    // irrelevant — every live worker is sampled.
    let obs = t.observe(stuck, sample(999_999, true));
    assert_eq!(obs.outcome, SweepOutcome::Baseline);
    assert!(!obs.evict);

    // Sweeps 2–3: still frozen, still failing to serve. Two stalled
    // samples: below the threshold.
    for expected in 1..=2 {
        let obs = t.observe(stuck, sample(999_999, true));
        assert_eq!(obs.outcome, SweepOutcome::Stalled);
        assert_eq!(obs.stalled_streak, expected);
        assert!(!obs.evict);
    }

    // Sweep 4: the third consecutive stalled sample. The detector no
    // longer needs the probe to time out for this worker to conclude
    // anything.
    let obs = t.observe(stuck, sample(999_999, true));
    assert_eq!(obs.outcome, SweepOutcome::Stalled);
    assert_eq!(obs.stalled_streak, 3);
    assert!(obs.evict, "worker 23015220 is STUCK");
}

/// The incident's blind spot, closed: the probe is a one-token completion
/// and can succeed on a worker that cannot serve real traffic. The
/// `failing_to_serve` condition is the serve-failure signal (the worker
/// claims a request in flight but moved no tokens), not the probe's
/// verdict — so a stuck worker that answers its probe still evicts.
#[test]
fn a_stuck_worker_that_answers_the_probe_still_evicts() {
    let mut t = StuckTracker::new(2);
    let stuck = "23015220";

    t.observe(stuck, sample(50_000, false)); // baseline
    t.observe(stuck, sample(50_000, true)); // stalled
    let obs = t.observe(stuck, sample(50_000, true)); // stalled
    assert_eq!(obs.stalled_streak, 2);
    assert!(obs.evict);
}
