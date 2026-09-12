//! Admit workers on measured throughput, not liveness (issue #4409).
//!
//! The regression tests run in the configuration the incident required:
//! `qwen3.8-flash-next` on the same card, image and node class as
//! `qwen3.8-27b`, every liveness check green, and the model executing on
//! CPU at 0% GPU utilisation — the state every liveness check reads as
//! healthy.

use autospec_core::capability_admission::{
    admit, deviation, diverges, gpu_during_generation, Admission, BaselineTable, Deviation,
    GenerationGpu, LivenessProfile, Measurement, Stage, ThroughputFloor,
};

/// The incident's measured prefill rate: 6 tok/s.
const FLASH_PREFILL: u64 = 6;
/// The reference prefill rate on the same card: 918 tok/s for `qwen3.8-27b`.
const REFERENCE_PREFILL: u64 = 918;
/// The incident's decode rate was low too; use a pair of floors the
/// reference model clears comfortably.
const FLOOR_PREFILL: u64 = 500;
const FLOOR_DECODE: u64 = 20;

/// The incident's liveness profile: every check green, the capability
/// absent.
fn incident_liveness() -> LivenessProfile {
    LivenessProfile {
        process_up: true,
        health_ok: true,
        props_ok: true,
        probe_ok: true,
        registered: true,
        counters_advancing: true,
    }
}

// ── The incident ────────────────────────────────────────────────────────────

#[test]
fn the_incident_all_liveness_green_but_prefill_far_below_floor_fails_admission() {
    let liveness = incident_liveness();
    // Every signal the fleet had said healthy.
    assert!(liveness.all_green());
    // The measurement is one request, timed: 6 tok/s against a 918 tok/s
    // card.
    let measured = Measurement {
        prefill_toks_per_s: FLASH_PREFILL,
        decode_toks_per_s: 4,
    };
    let floor = ThroughputFloor::new(REFERENCE_PREFILL, 40).expect("valid floor");
    assert_eq!(
        admit(&liveness, Some(&measured), Some(&floor)),
        Admission::BelowFloor {
            stage: Stage::Prefill,
            measured: FLASH_PREFILL,
            floor: REFERENCE_PREFILL,
        }
    );
    // This is the general form: liveness and capability diverged, which is
    // exactly when something interesting is wrong.
    assert!(diverges(liveness.all_green(), false));
    // And the one-call-away signal: 0% GPU during an active prefill.
    assert_eq!(
        gpu_during_generation(0),
        GenerationGpu::IdleDuringGeneration { percent: 0 }
    );
}

// ── Invariant 1: admit on measured throughput ───────────────────────────────

#[test]
fn a_serving_worker_with_no_measurement_is_not_admitted_on_liveness_alone() {
    let liveness = incident_liveness();
    let floor = ThroughputFloor::new(FLOOR_PREFILL, FLOOR_DECODE).expect("valid floor");
    assert_eq!(admit(&liveness, None, Some(&floor)), Admission::Unmeasured);
}

#[test]
fn a_serving_worker_with_no_recorded_floor_fails_closed() {
    let liveness = incident_liveness();
    let measured = Measurement {
        prefill_toks_per_s: REFERENCE_PREFILL,
        decode_toks_per_s: 40,
    };
    assert_eq!(admit(&liveness, Some(&measured), None), Admission::NoFloor);
}

#[test]
fn a_non_serving_worker_is_reported_as_not_serving_before_any_rate_check() {
    let liveness = LivenessProfile {
        process_up: true,
        health_ok: true,
        props_ok: false, // one red liveness signal
        probe_ok: true,
        registered: true,
        counters_advancing: true,
    };
    assert!(!liveness.all_green());
    let measured = Measurement {
        prefill_toks_per_s: REFERENCE_PREFILL,
        decode_toks_per_s: 40,
    };
    let floor = ThroughputFloor::new(FLOOR_PREFILL, FLOOR_DECODE).expect("valid floor");
    // The rate is fine; the worker is simply not serving.
    assert_eq!(
        admit(&liveness, Some(&measured), Some(&floor)),
        Admission::NotServing
    );
}

#[test]
fn a_measured_worker_that_meets_the_floor_is_admitted() {
    let liveness = incident_liveness();
    let measured = Measurement {
        prefill_toks_per_s: REFERENCE_PREFILL,
        decode_toks_per_s: 40,
    };
    let floor = ThroughputFloor::new(FLOOR_PREFILL, FLOOR_DECODE).expect("valid floor");
    assert_eq!(
        admit(&liveness, Some(&measured), Some(&floor)),
        Admission::Admitted
    );
}

#[test]
fn a_decode_rate_below_the_floor_fails_admission_on_the_decode_stage() {
    let liveness = incident_liveness();
    let measured = Measurement {
        prefill_toks_per_s: REFERENCE_PREFILL, // prefill clears the floor
        decode_toks_per_s: 10,                 // decode does not
    };
    let floor = ThroughputFloor::new(FLOOR_PREFILL, FLOOR_DECODE).expect("valid floor");
    assert_eq!(
        admit(&liveness, Some(&measured), Some(&floor)),
        Admission::BelowFloor {
            stage: Stage::Decode,
            measured: 10,
            floor: FLOOR_DECODE,
        }
    );
}

#[test]
fn meeting_the_floor_exactly_admits_and_one_below_fails() {
    let liveness = incident_liveness();
    let floor = ThroughputFloor::new(FLOOR_PREFILL, FLOOR_DECODE).expect("valid floor");
    let at_floor = Measurement {
        prefill_toks_per_s: FLOOR_PREFILL,
        decode_toks_per_s: FLOOR_DECODE,
    };
    assert_eq!(
        admit(&liveness, Some(&at_floor), Some(&floor)),
        Admission::Admitted
    );
    let below = Measurement {
        prefill_toks_per_s: FLOOR_PREFILL - 1,
        decode_toks_per_s: FLOOR_DECODE,
    };
    assert_eq!(
        admit(&liveness, Some(&below), Some(&floor)),
        Admission::BelowFloor {
            stage: Stage::Prefill,
            measured: FLOOR_PREFILL - 1,
            floor: FLOOR_PREFILL,
        }
    );
}

#[test]
fn a_zero_in_either_stage_is_not_a_floor() {
    assert_eq!(ThroughputFloor::new(0, 40), None);
    assert_eq!(ThroughputFloor::new(500, 0), None);
}

// ── Invariant 2: GPU utilisation during generation ──────────────────────────

#[test]
fn near_zero_utilization_while_generating_is_unhealthy() {
    // The incident: both GPUs at 0% / 74 W during an active prefill, weights
    // resident in VRAM — executing on CPU.
    assert_eq!(
        gpu_during_generation(0),
        GenerationGpu::IdleDuringGeneration { percent: 0 }
    );
    assert!(!gpu_during_generation(0).healthy());
    assert!(!gpu_during_generation(4).healthy());
}

#[test]
fn a_busy_gpu_is_the_healthy_signal_during_generation() {
    assert_eq!(gpu_during_generation(5), GenerationGpu::Busy { percent: 5 });
    assert_eq!(
        gpu_during_generation(97),
        GenerationGpu::Busy { percent: 97 }
    );
    assert!(gpu_during_generation(97).healthy());
}

// ── Invariant 3: per-(model, card) baseline and deviation ───────────────────

#[test]
fn the_baseline_names_the_regression_an_image_or_upgrade_produces() {
    let mut baselines = BaselineTable::new();
    // The card's recorded baseline: what this (model, card) pair is known to
    // do.
    baselines.record("qwen3.8-27b", "rtx-4090", 918, 45);
    let baseline = baselines
        .baseline_for("qwen3.8-27b", "rtx-4090")
        .expect("recorded baseline");

    // The same pair after an image change: 6 tok/s. The deviation from the
    // baseline — not an absolute threshold — is what alerts.
    let measured = Measurement {
        prefill_toks_per_s: FLASH_PREFILL,
        decode_toks_per_s: 4,
    };
    assert_eq!(
        deviation(baseline, &measured, 5),
        Deviation::Regression {
            stage: Stage::Prefill,
            measured: FLASH_PREFILL,
            baseline: 918,
            times_slower: 153, // 918 / 6
        }
    );
}

#[test]
fn a_measurement_at_or_above_the_baseline_is_within_baseline() {
    let mut baselines = BaselineTable::new();
    baselines.record("qwen3.8-27b", "rtx-4090", 918, 45);
    let baseline = baselines
        .baseline_for("qwen3.8-27b", "rtx-4090")
        .expect("recorded baseline");

    let measured = Measurement {
        prefill_toks_per_s: 900, // below the baseline, but only ~1x
        decode_toks_per_s: 45,
    };
    assert_eq!(deviation(baseline, &measured, 5), Deviation::WithinBaseline);
}

#[test]
fn deviation_alerts_on_the_worse_stage_and_uses_the_smallest_meaningful_factor() {
    let mut baselines = BaselineTable::new();
    baselines.record("m", "c", 900, 45);
    let baseline = baselines.baseline_for("m", "c").expect("recorded baseline");

    // Decode regressed 9x, prefill regressed 3x: the worse stage is named.
    let measured = Measurement {
        prefill_toks_per_s: 300,
        decode_toks_per_s: 5,
    };
    assert_eq!(
        deviation(baseline, &measured, 2),
        Deviation::Regression {
            stage: Stage::Decode,
            measured: 5,
            baseline: 45,
            times_slower: 9,
        }
    );

    // A factor of 1 is no deviation: below the baseline by less than 2x
    // never alerts, no matter how low the requested minimum.
    let mild = Measurement {
        prefill_toks_per_s: 500, // 900 / 500 = 1
        decode_toks_per_s: 25,   // 45 / 25 = 1
    };
    assert_eq!(deviation(baseline, &mild, 1), Deviation::WithinBaseline);

    // A zero measured rate counts as 1: a dead stage reports the full
    // baseline factor.
    let dead = Measurement {
        prefill_toks_per_s: 900,
        decode_toks_per_s: 0,
    };
    assert_eq!(
        deviation(baseline, &dead, 2),
        Deviation::Regression {
            stage: Stage::Decode,
            measured: 0,
            baseline: 45,
            times_slower: 45,
        }
    );
}

#[test]
fn baselines_are_keyed_per_model_and_card_and_rerecorded_by_upsert() {
    let mut baselines = BaselineTable::new();
    baselines.record("qwen3.8-27b", "rtx-4090", 918, 45);
    baselines.record("qwen3.8-flash-next", "rtx-4090", 900, 44);
    // The same card, a different model: a different entry.
    assert_eq!(
        baselines
            .baseline_for("qwen3.8-27b", "rtx-4090")
            .unwrap()
            .prefill_toks_per_s,
        918
    );
    assert_eq!(
        baselines
            .baseline_for("qwen3.8-flash-next", "rtx-4090")
            .unwrap()
            .prefill_toks_per_s,
        900
    );
    // A different card, the same model: no entry yet.
    assert!(baselines.baseline_for("qwen3.8-27b", "h100").is_none());
    // Re-recording is an upsert: the new measurement replaces the entry.
    baselines.record("qwen3.8-27b", "rtx-4090", 940, 46);
    assert_eq!(
        baselines
            .baseline_for("qwen3.8-27b", "rtx-4090")
            .unwrap()
            .prefill_toks_per_s,
        940
    );
}

// ── The general form ────────────────────────────────────────────────────────

#[test]
fn liveness_and_capability_diverge_only_when_something_interesting_is_wrong() {
    // The incident: alive and responding, capability absent.
    assert!(diverges(true, false));
    // Healthy: alive and capable.
    assert!(!diverges(true, true));
    // Dead: not alive, and the capability is moot — this is what liveness
    // checks already catch.
    assert!(!diverges(false, false));
    assert!(!diverges(false, true));
}
