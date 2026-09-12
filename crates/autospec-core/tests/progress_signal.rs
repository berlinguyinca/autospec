//! Progress signals and update granularity (issue #4407).
//!
//! The incident: a stuck-worker detector sampled
//! `llamacpp:prompt_tokens_total` and concluded "no progress" when it did not
//! move. That counter is incremented when a request *completes*, not as
//! tokens are processed. A worker mid-prefill on a 40-minute request shows a
//! frozen counter while doing exactly the work it is supposed to do, and the
//! detector classified two healthy workers as STUCK — they were logging
//! progress lines the whole time
//! (`prompt processing, n_tokens = 4096, progress = 0.10`).
//!
//! The regression tests run in the configuration the bug required: a
//! per-completion total that is frozen while the worker is doing a long
//! in-flight operation. The controls: the same frozen counter read from a
//! per-unit signal, and a spec that does (and does not) state the metric's
//! update granularity.

use autospec_core::progress_signal::{
    classify_stuck, liveness_capable, InFlight, LivenessSpec, Metric, MidOperation, StuckVerdict,
    UpdateGranularity,
};

/// `llamacpp:prompt_tokens_total` — incremented when a request completes.
/// Frozen while a long prefill is in flight.
fn prompt_tokens_total() -> Metric {
    Metric {
        name: "llamacpp:prompt_tokens_total".into(),
        granularity: UpdateGranularity::PerCompletion,
    }
}

/// `llamacpp:tokens_predicted_total` — the other total the detector sampled.
/// Also incremented on completion, not during the operation.
fn tokens_predicted_total() -> Metric {
    Metric {
        name: "llamacpp:tokens_predicted_total".into(),
        granularity: UpdateGranularity::PerCompletion,
    }
}

/// `llamacpp:n_decode_total` — incremented as the work proceeds. Advances
/// while the operation is in flight.
fn n_decode_total() -> Metric {
    Metric {
        name: "llamacpp:n_decode_total".into(),
        granularity: UpdateGranularity::PerUnit,
    }
}

/// The worker is logging `prompt processing, n_tokens = 4096, progress =
/// 0.10` the whole time: it is reporting in-flight progress.
fn reporting() -> InFlight {
    InFlight { reporting: true }
}

/// The worker is not reporting any in-flight progress.
fn silent() -> InFlight {
    InFlight { reporting: false }
}

#[test]
fn the_incident_a_frozen_total_while_reporting_is_mid_operation_not_stuck() {
    // The two healthy workers: prompt_tokens_total did not move, and the
    // detector called them STUCK. With the granularity established, the same
    // reading is InFlight — the counter is frozen by definition while a
    // completion-keyed operation is in flight, and the worker is reporting
    // progress.
    let verdict = classify_stuck(&prompt_tokens_total(), false, &reporting());

    assert!(matches!(verdict, StuckVerdict::InFlight));
    assert!(
        !verdict.is_stuck(),
        "a reporting worker with a frozen total is not stuck"
    );

    let line = verdict.line(&prompt_tokens_total());
    assert!(line.starts_with("OK:"), "{line}");
    assert!(line.contains("mid-operation, not stuck"), "{line}");
}

#[test]
fn both_healthy_workers_are_clear_and_the_second_total_agrees() {
    // The incident named two workers. Both read the completion-keyed totals
    // as frozen while reporting progress; neither may be read as stuck, and
    // the two totals agree.
    let a = classify_stuck(&prompt_tokens_total(), false, &reporting());
    let b = classify_stuck(&tokens_predicted_total(), false, &reporting());
    assert!(!a.is_stuck());
    assert!(!b.is_stuck());
    assert!(matches!(a, StuckVerdict::InFlight));
    assert!(matches!(b, StuckVerdict::InFlight));
}

#[test]
fn a_frozen_total_with_no_in_flight_report_is_inconclusive_not_stuck() {
    // The corollary's other branch: the total is frozen and the worker is not
    // reporting in-flight progress. A total still cannot call this — frozen
    // is its normal in-flight state. The detector must not conclude STUCK;
    // it needs a per-unit signal or the process's own report.
    let verdict = classify_stuck(&prompt_tokens_total(), false, &silent());

    assert!(matches!(verdict, StuckVerdict::Inconclusive));
    assert!(
        !verdict.is_stuck(),
        "a total alone can never be read as stuck"
    );

    let line = verdict.line(&prompt_tokens_total());
    assert!(line.starts_with("INCONCLUSIVE:"), "{line}");
    assert!(line.contains("a total cannot call this"), "{line}");
}

#[test]
fn a_per_unit_signal_read_directly_distinguishes_slow_from_stopped() {
    // n_decode_total advances as the work proceeds: a moving reading is slow
    // (the worker is doing the work), a frozen reading is the only state that
    // may be read as stopped.
    let slow = classify_stuck(&n_decode_total(), true, &silent());
    assert!(matches!(slow, StuckVerdict::Slow));
    assert!(!slow.is_stuck());

    let stopped = classify_stuck(&n_decode_total(), false, &reporting());
    assert!(matches!(stopped, StuckVerdict::Stopped));
    assert!(
        stopped.is_stuck(),
        "a frozen per-unit signal is the only STUCK"
    );

    assert!(slow.line(&n_decode_total()).starts_with("OK:"));
    assert!(stopped.line(&n_decode_total()).starts_with("STUCK:"));
}

#[test]
fn only_a_per_unit_metric_is_a_liveness_signal() {
    // Only the per-unit signal can distinguish slow from stopped. The
    // completion-keyed totals cannot, because frozen is their normal
    // in-flight state.
    assert!(liveness_capable(&n_decode_total()));
    assert!(!liveness_capable(&prompt_tokens_total()));
    assert!(!liveness_capable(&tokens_predicted_total()));
}

#[test]
fn granularity_fixes_the_mid_operation_behavior() {
    // A per-unit signal advances while its operation is in flight; a total is
    // frozen. This is the fact a spec must state before watching the metric.
    assert_eq!(
        UpdateGranularity::PerUnit.mid_operation(),
        MidOperation::Advances
    );
    assert_eq!(
        UpdateGranularity::PerCompletion.mid_operation(),
        MidOperation::Frozen
    );
    assert_eq!(UpdateGranularity::PerUnit.label(), "per-unit");
    assert_eq!(UpdateGranularity::PerCompletion.label(), "per-completion");
}

#[test]
fn is_stuck_is_exclusive_to_stopped() {
    // The guard in one call: only a frozen per-unit reading may be read as
    // stuck. Every other verdict — including both total-derived ones — is
    // not.
    assert!(StuckVerdict::Stopped.is_stuck());
    assert!(!StuckVerdict::Slow.is_stuck());
    assert!(!StuckVerdict::InFlight.is_stuck());
    assert!(!StuckVerdict::Inconclusive.is_stuck());
}

#[test]
fn a_spec_that_states_the_granularity_is_complete() {
    // The "for specs" invariant: a spec that says "detect a stalled worker by
    // watching n_decode_total" states it advances while the operation is
    // in flight — correct for a per-unit metric, and no finding.
    let spec = LivenessSpec {
        metric: n_decode_total(),
        stated_mid_operation: Some(MidOperation::Advances),
    };
    assert!(spec.states_granularity());
    assert!(spec.finding().is_none());
}

#[test]
fn a_spec_that_watches_a_total_without_stating_granularity_is_a_finding() {
    // The defect the incident came from: a spec that watches
    // prompt_tokens_total but never establishes its update granularity.
    // It is specified against a metric whose semantics nobody checked.
    let spec = LivenessSpec {
        metric: prompt_tokens_total(),
        stated_mid_operation: None,
    };
    assert!(!spec.states_granularity());
    let finding = spec
        .finding()
        .expect("a spec with no stated granularity is a finding");
    assert!(finding.starts_with("SPEC:"), "{finding}");
    assert!(finding.contains("prompt_tokens_total"), "{finding}");
    assert!(finding.contains("per-completion"), "{finding}");
    assert!(finding.contains("frozen"), "{finding}");
}

#[test]
fn a_spec_that_misstates_a_totals_granularity_is_also_a_finding() {
    // One step later than "nobody checked": the spec checked, and got it
    // wrong — it states the total "advances" while it is actually frozen.
    // That is a detector specified against a different metric than the one
    // it is watching.
    let spec = LivenessSpec {
        metric: prompt_tokens_total(),
        stated_mid_operation: Some(MidOperation::Advances),
    };
    assert!(!spec.states_granularity());
    let finding = spec
        .finding()
        .expect("a misstated granularity is a finding");
    assert!(finding.contains("per-completion"), "{finding}");
    assert!(finding.contains("advancing"), "{finding}");
    assert!(finding.contains("frozen"), "{finding}");
}

#[test]
fn a_per_unit_spec_must_state_advancing_not_frozen() {
    // The mirror: a spec watching a per-unit signal that states it "frozen"
    // has the semantics of the wrong metric.
    let spec = LivenessSpec {
        metric: n_decode_total(),
        stated_mid_operation: Some(MidOperation::Frozen),
    };
    assert!(!spec.states_granularity());
    assert!(spec.finding().is_some());
}

#[test]
fn the_incident_end_to_end_from_frozen_total_to_mid_operation() {
    // Reconstructed end to end: the detector sampled the completion-keyed
    // total, it did not move, and the worker was logging progress. Before the
    // granularity was established the reading looked like "no progress";
    // with it established, the same reading is a healthy worker mid-prefill.
    let total = prompt_tokens_total();

    // The reading the incident's detector made: total frozen.
    let frozen = classify_stuck(&total, false, &reporting());
    assert!(!frozen.is_stuck());
    assert!(matches!(frozen, StuckVerdict::InFlight));

    // The signal that actually distinguishes slow from stopped is per-unit.
    // While the same worker is mid-prefill, n_decode_total is moving.
    let per_unit = classify_stuck(&n_decode_total(), true, &reporting());
    assert!(matches!(per_unit, StuckVerdict::Slow));
    assert!(!per_unit.is_stuck());

    // And it is the one that can call the stop, when the work actually stops.
    let stopped = classify_stuck(&n_decode_total(), false, &reporting());
    assert!(stopped.is_stuck());
}
