//! The tracker's program result target (#3315): at least a 2x median
//! wall-clock reduction on successful local coding issues, without crossing
//! the configured quality or node-stability gates.

use autospec_core::aar::result_target::{
    evaluate_result_target, median_wall_ms, GateStatus, ResultTarget,
};

fn five(value: u64) -> Vec<u64> {
    vec![value; 5]
}

#[test]
fn median_of_odd_count_is_the_middle() {
    assert_eq!(median_wall_ms(&[5, 1, 3]), 3.0);
}

#[test]
fn median_of_even_count_averages_the_two_middle() {
    assert_eq!(median_wall_ms(&[4, 1, 3, 2]), 2.5);
}

#[test]
fn median_of_empty_is_zero() {
    assert_eq!(median_wall_ms(&[]), 0.0);
}

#[test]
fn a_two_x_median_reduction_is_proven() {
    let verdict = evaluate_result_target(&ResultTarget::default(), &five(100), &five(50), 0.9, 1.0);
    assert_eq!(verdict.status, GateStatus::Proven);
    assert!(verdict.is_met());
    assert_eq!(verdict.achieved_speedup, Some(2.0));
}

#[test]
fn exactly_at_the_factor_is_proven() {
    // "at least 2x" — the boundary itself passes.
    let verdict =
        evaluate_result_target(&ResultTarget::default(), &five(200), &five(100), 1.0, 1.0);
    assert_eq!(verdict.status, GateStatus::Proven);
    assert_eq!(verdict.achieved_speedup, Some(2.0));
}

#[test]
fn below_the_factor_is_not_proven() {
    let verdict = evaluate_result_target(&ResultTarget::default(), &five(100), &five(60), 1.0, 1.0);
    assert_eq!(verdict.status, GateStatus::InsufficientSpeedup);
    assert!(!verdict.is_met());
}

#[test]
fn a_fast_candidate_below_the_quality_floor_is_a_gate_crossing_not_a_win() {
    let target = ResultTarget {
        quality_floor: 0.5,
        ..ResultTarget::default()
    };
    let verdict = evaluate_result_target(&target, &five(100), &five(10), 0.2, 1.0);
    assert_eq!(verdict.status, GateStatus::QualityGateCrossed);
    assert!(!verdict.is_met());
    // The speedup is still reported for the audit trail.
    assert_eq!(verdict.achieved_speedup, Some(10.0));
}

#[test]
fn an_unstable_node_crosses_the_stability_gate() {
    let verdict = evaluate_result_target(&ResultTarget::default(), &five(100), &five(10), 0.9, 0.8);
    assert_eq!(verdict.status, GateStatus::StabilityGateCrossed);
    assert!(!verdict.is_met());
}

#[test]
fn too_few_successful_samples_are_not_trusted() {
    let verdict = evaluate_result_target(
        &ResultTarget::default(),
        &[100, 100, 100],
        &[10, 10, 10],
        1.0,
        1.0,
    );
    assert_eq!(verdict.status, GateStatus::InsufficientSamples);
    assert!(verdict.achieved_speedup.is_none());
}

#[test]
fn a_zero_candidate_median_is_the_best_possible_speedup() {
    let verdict = evaluate_result_target(&ResultTarget::default(), &five(100), &five(0), 1.0, 1.0);
    assert_eq!(verdict.status, GateStatus::Proven);
    assert_eq!(verdict.achieved_speedup, Some(f64::INFINITY));
}

#[test]
fn gates_are_checked_before_speedup_so_quality_is_reported_first() {
    let target = ResultTarget {
        quality_floor: 0.5,
        stability_floor: 0.9,
        ..ResultTarget::default()
    };
    // 10x faster but both gates are crossed: the quality gate, checked first,
    // is the one reported.
    let verdict = evaluate_result_target(&target, &five(100), &five(10), 0.2, 0.5);
    assert_eq!(verdict.status, GateStatus::QualityGateCrossed);
}

#[test]
fn status_names_are_stable() {
    assert_eq!(GateStatus::Proven.as_str(), "proven");
    assert_eq!(
        GateStatus::InsufficientSamples.as_str(),
        "insufficient_samples"
    );
    assert_eq!(
        GateStatus::InsufficientSpeedup.as_str(),
        "insufficient_speedup"
    );
    assert_eq!(
        GateStatus::QualityGateCrossed.as_str(),
        "quality_gate_crossed"
    );
    assert_eq!(
        GateStatus::StabilityGateCrossed.as_str(),
        "stability_gate_crossed"
    );
}
