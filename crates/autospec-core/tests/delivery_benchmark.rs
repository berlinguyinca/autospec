//! Delivery benchmark contract (spec section 7), over literal result rows.

use autospec_core::delivery_benchmark::{
    summarize, Arm, BenchTask, Comparison, RunResult, SCENARIOS,
};

fn run(task: BenchTask, arm: Arm, secs: f64, success: bool) -> RunResult {
    RunResult {
        task,
        arm,
        time_to_verified_secs: secs,
        tokens: 100,
        retries: 1,
        disk_bytes: 4096,
        success,
    }
}

fn summary_of(
    results: &[RunResult],
    task: BenchTask,
) -> autospec_core::delivery_benchmark::TaskSummary {
    summarize(results)
        .into_iter()
        .find(|summary| summary.task == task)
        .expect("every task is summarized")
}

#[test]
fn scenarios_are_a1_through_a9_in_order() {
    let ids: Vec<&str> = SCENARIOS.iter().map(|scenario| scenario.id).collect();
    assert_eq!(ids, ["A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8", "A9"]);
    assert!(SCENARIOS
        .iter()
        .all(|scenario| !scenario.summary.is_empty()));
    assert_eq!(SCENARIOS[4].acceptance_items, &[7, 8]);
}

#[test]
fn the_benchmark_set_has_ten_distinct_tasks() {
    assert_eq!(BenchTask::ALL.len(), 10);
    let mut names: Vec<&str> = BenchTask::ALL.iter().map(|task| task.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 10);
}

#[test]
fn wire_names_match_as_str() {
    for task in BenchTask::ALL {
        let json = serde_json::to_string(&task).unwrap();
        assert_eq!(json, format!("\"{}\"", task.as_str()));
    }
}

#[test]
fn nearest_rank_percentiles_of_four_times() {
    let rows: Vec<RunResult> = [30.0, 10.0, 40.0, 20.0]
        .iter()
        .map(|secs| run(BenchTask::TrivialFix, Arm::Baseline, *secs, true))
        .collect();
    let summary = summary_of(&rows, BenchTask::TrivialFix);
    assert_eq!(summary.baseline.p50_secs, Some(20.0));
    assert_eq!(summary.baseline.p95_secs, Some(40.0));
}

#[test]
fn a_task_with_only_pi_autospec_rows_has_no_baseline() {
    let rows = [run(BenchTask::Crash, Arm::PiAutospec, 5.0, true)];
    let summary = summary_of(&rows, BenchTask::Crash);
    assert_eq!(summary.comparison, Comparison::BaselineMissing);
    assert_eq!(summary.pi_autospec.p50_secs, Some(5.0));
}

#[test]
fn a_baseline_whose_every_run_failed_is_still_missing() {
    let rows = [
        run(BenchTask::Crash, Arm::Baseline, 5.0, false),
        run(BenchTask::Crash, Arm::PiAutospec, 4.0, true),
    ];
    assert_eq!(
        summary_of(&rows, BenchTask::Crash).comparison,
        Comparison::BaselineMissing
    );
}

#[test]
fn a_failed_run_lowers_the_success_rate_and_is_excluded_from_p50() {
    let rows = [
        run(BenchTask::LongTest, Arm::Baseline, 10.0, true),
        run(BenchTask::LongTest, Arm::Baseline, 1000.0, false),
        run(BenchTask::LongTest, Arm::Baseline, 30.0, true),
    ];
    let baseline = summary_of(&rows, BenchTask::LongTest).baseline;
    assert_eq!(baseline.runs, 3);
    assert_eq!(baseline.successes, 2);
    assert!((baseline.success_rate.unwrap() - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(baseline.p50_secs, Some(10.0));
    assert_eq!(baseline.p95_secs, Some(30.0));
}

#[test]
fn a_measured_comparison_is_candidate_minus_baseline() {
    let rows = [
        run(BenchTask::StaleCi, Arm::Baseline, 100.0, true),
        run(BenchTask::StaleCi, Arm::PiAutospec, 70.0, true),
    ];
    assert_eq!(
        summary_of(&rows, BenchTask::StaleCi).comparison,
        Comparison::Measured {
            p50_delta_secs: -30.0,
            p95_delta_secs: -30.0
        }
    );
}

#[test]
fn a_candidate_with_no_success_is_reported_not_measured() {
    let rows = [
        run(BenchTask::GatewayBusy, Arm::Baseline, 10.0, true),
        run(BenchTask::GatewayBusy, Arm::PiAutospec, 9.0, false),
    ];
    assert_eq!(
        summary_of(&rows, BenchTask::GatewayBusy).comparison,
        Comparison::CandidateMissing
    );
}

#[test]
fn every_task_is_summarized_even_with_no_rows() {
    let summaries = summarize(&[]);
    assert_eq!(summaries.len(), 10);
    assert!(summaries
        .iter()
        .all(|summary| summary.comparison == Comparison::BaselineMissing
            && summary.baseline.success_rate.is_none()));
}

#[test]
fn a_result_row_round_trips_through_json() {
    let row = run(BenchTask::MultiRepoContract, Arm::PiAutospec, 12.5, true);
    let json = serde_json::to_string(&row).unwrap();
    assert!(json.contains("\"pi-autospec\""), "{json}");
    assert_eq!(serde_json::from_str::<RunResult>(&json).unwrap(), row);
}

#[test]
fn a_success_with_a_non_finite_or_negative_time_counts_as_a_failure() {
    let rows = [
        run(BenchTask::Crash, Arm::Baseline, 10.0, true),
        run(BenchTask::Crash, Arm::Baseline, f64::NAN, true),
        run(BenchTask::Crash, Arm::Baseline, f64::INFINITY, true),
        run(BenchTask::Crash, Arm::Baseline, -5.0, true),
    ];
    let baseline = summary_of(&rows, BenchTask::Crash).baseline;
    assert_eq!(baseline.runs, 4);
    assert_eq!(baseline.successes, 1);
    assert_eq!(baseline.p95_secs, Some(10.0));
}
