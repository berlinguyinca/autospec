//! An average over non-substitutable units is a different quantity, not an
//! approximation (issue #4461).
//!
//! The regression shape: one saturated small-slot model plus a large idle
//! pool must produce a scale-up for the saturated model, and the decision
//! must not be satisfiable by lowering a global threshold.

use autospec_core::aggregate_granularity::{
    aggregate_threshold, decide_scale_up, max_average_shift, FleetSample, Fungibility, ScaleUp,
    UnitObservation,
};

fn deepseek(deferred: u32) -> UnitObservation {
    UnitObservation {
        unit: "deepseek-v4-flash".to_string(),
        slots: 1,
        busy: 1,
        requests_deferred: deferred,
    }
}

fn qwen(busy: u32) -> UnitObservation {
    UnitObservation {
        unit: "qwen3.8-27b".to_string(),
        slots: 86,
        busy,
        requests_deferred: 0,
    }
}

/// The incident window: `deepseek-v4-flash` at 1/1 with a deferred queue of
/// 2, fleet at 27/87 busy, three consecutive samples, same shape each time.
fn incident_samples() -> Vec<FleetSample> {
    (0..3)
        .map(|_| FleetSample {
            units: vec![deepseek(2), qwen(26)],
        })
        .collect()
}

#[test]
fn saturated_small_model_plus_idle_pool_triggers_scale_up_for_that_model() {
    let samples = incident_samples();
    let decision = decide_scale_up(&samples, 3, 75.0);

    // The fleet average says "31% busy, target 75%": no scale-up. The
    // per-unit decision says the saturated model is turning requests away.
    assert_eq!(
        decision.scale_ups,
        vec![ScaleUp {
            unit: "deepseek-v4-flash".to_string(),
            requests_deferred: 2,
            sustained_samples: 3,
        }]
    );
    // The aggregate is still reported, for context: 27 of 87 slots busy.
    // The population the average covers is exactly the incident's: 27/87,
    // about 31%.
    assert_eq!(decision.context.busy, 27);
    assert_eq!(decision.context.slots, 87);
    let fleet_percent = samples.last().unwrap().busy_percent();
    assert!((fleet_percent - 100.0 * 27.0 / 87.0).abs() < 1e-9);
    assert_eq!(decision.context.target_busy_percent, 75.0);
}

#[test]
fn decision_is_independent_of_the_global_threshold() {
    let samples = incident_samples();
    let reference = decide_scale_up(&samples, 3, 75.0).scale_ups;

    // No fleet-wide occupancy value, and no value of the target, changes
    // the per-unit decision — including targets below the fleet average
    // (lowering the threshold) and targets at the ceiling.
    for target in [0.0, 31.0, 50.0, 75.0, 100.0] {
        assert_eq!(
            decide_scale_up(&samples, 3, target).scale_ups,
            reference,
            "target {target} must not change the per-unit decision"
        );
    }
}

#[test]
fn no_global_threshold_can_reproduce_the_decision() {
    // The structural form of "not satisfiable by lowering a global
    // threshold": over non-substitutable units the aggregate check is
    // refused for every value of the threshold — it can never be the
    // decision, at any target, not just 75%.
    for target in (0..=100).map(|t| t as f64) {
        match aggregate_threshold(27, 87, target, Fungibility::NotSubstitutable) {
            autospec_core::aggregate_granularity::AggregateCheck::RefusedNonFungible { .. } => {}
            other => panic!(
                "target {target}: aggregate check over non-substitutable units must be refused, got {other:?}"
            ),
        }
    }
}

#[test]
fn aggregate_threshold_is_valid_on_fungible_units() {
    match aggregate_threshold(60, 80, 75.0, Fungibility::Substitutable) {
        autospec_core::aggregate_granularity::AggregateCheck::Triggered { percent, .. } => {
            assert!((percent - 75.0).abs() < 1e-9);
        }
        other => panic!("60/80 at target 75% must trigger, got {other:?}"),
    }
    match aggregate_threshold(20, 80, 75.0, Fungibility::Substitutable) {
        autospec_core::aggregate_granularity::AggregateCheck::BelowTarget { percent, .. } => {
            assert!((percent - 25.0).abs() < 1e-9);
        }
        other => panic!("20/80 at target 75% must be below target, got {other:?}"),
    }
}

#[test]
fn the_mask_is_bounded_by_the_units_share_of_the_fleet() {
    // A 1-slot model in an 87-slot fleet moves the fleet average by at
    // most ~1.1 percentage points: the whole band a global threshold
    // could use to see its saturation.
    let shift = max_average_shift(1, 87);
    assert!((shift - 100.0 / 87.0).abs() < 1e-9);
    assert!(
        shift < 1.2,
        "a 1-of-87 unit must not move the average more than ~1.1 points"
    );

    // Degenerate cases: no unit, an empty fleet, and a unit larger than
    // the fleet (clamped to the whole average).
    assert_eq!(max_average_shift(0, 87), 0.0);
    assert_eq!(max_average_shift(1, 0), 0.0);
    assert_eq!(max_average_shift(200, 87), 100.0);
}

#[test]
fn deferral_must_be_sustained_across_the_window() {
    // Deferred in one of the three samples: a blip, not sustained.
    let blip = vec![
        FleetSample {
            units: vec![deepseek(2), qwen(26)],
        },
        FleetSample {
            units: vec![deepseek(0), qwen(26)],
        },
        FleetSample {
            units: vec![deepseek(0), qwen(26)],
        },
    ];
    assert!(decide_scale_up(&blip, 3, 75.0).scale_ups.is_empty());

    // Deferred in the last two of three samples: still not sustained.
    let late = vec![
        FleetSample {
            units: vec![deepseek(0), qwen(26)],
        },
        FleetSample {
            units: vec![deepseek(2), qwen(26)],
        },
        FleetSample {
            units: vec![deepseek(2), qwen(26)],
        },
    ];
    assert!(decide_scale_up(&late, 3, 75.0).scale_ups.is_empty());

    // A shorter window than the sustained count: not yet sustained.
    assert!(decide_scale_up(&incident_samples()[..2], 3, 75.0)
        .scale_ups
        .is_empty());

    // Present with deferrals in only two of the three samples (absent
    // from one): not sustained.
    let absent = vec![
        FleetSample {
            units: vec![qwen(26)],
        },
        FleetSample {
            units: vec![deepseek(2), qwen(26)],
        },
        FleetSample {
            units: vec![deepseek(2), qwen(26)],
        },
    ];
    assert!(decide_scale_up(&absent, 3, 75.0).scale_ups.is_empty());

    // Sustained: deferred in all three samples.
    assert_eq!(
        decide_scale_up(&incident_samples(), 3, 75.0)
            .scale_ups
            .len(),
        1
    );
}

#[test]
fn a_fully_busy_fleet_still_names_the_scarce_unit() {
    // Every slot in the fleet is busy; only one model is deferring. The
    // decision is per-unit, so it names that model — the fleet being at
    // 100% changes nothing about which unit triggers.
    let saturated = (0..3)
        .map(|_| FleetSample {
            units: vec![deepseek(5), qwen(86)],
        })
        .collect::<Vec<_>>();
    let decision = decide_scale_up(&saturated, 3, 75.0);
    assert_eq!(decision.context.busy, 87);
    assert_eq!(
        decision.scale_ups,
        vec![ScaleUp {
            unit: "deepseek-v4-flash".to_string(),
            requests_deferred: 5,
            sustained_samples: 3,
        }]
    );
}

#[test]
fn the_decision_line_names_the_model_and_its_deferred_count() {
    let decision = decide_scale_up(&incident_samples(), 3, 75.0);
    let line = decision.line();
    assert_eq!(
        line,
        "scale-up model=deepseek-v4-flash requests_deferred=2 (sustained over 3 samples); \
         fleet 31% busy (27/87 over 3 samples), target 75%"
    );

    // A window with no sustained deferral says so, with the fleet
    // aggregate still alongside for context.
    let idle = (0..3)
        .map(|_| FleetSample {
            units: vec![
                UnitObservation {
                    unit: "deepseek-v4-flash".to_string(),
                    slots: 1,
                    busy: 0,
                    requests_deferred: 0,
                },
                qwen(26),
            ],
        })
        .collect::<Vec<_>>();
    assert_eq!(
        decide_scale_up(&idle, 3, 75.0).line(),
        "no scale-up; fleet 30% busy (26/87 over 3 samples), target 75%"
    );
}

#[test]
fn an_empty_window_is_a_no_op_with_zero_context() {
    let decision = decide_scale_up(&[], 3, 75.0);
    assert!(decision.scale_ups.is_empty());
    assert_eq!(decision.context.busy, 0);
    assert_eq!(decision.context.slots, 0);
    assert_eq!(
        decision.line(),
        "no scale-up; fleet 0% busy (0/0 over 0 samples), target 75%"
    );
}
