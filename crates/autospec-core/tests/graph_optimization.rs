use autospec_core::graph::optimization::{
    compare, should_retry, GraphMetrics, OptimizationSummary, RetryPolicy,
    DEFAULT_MAX_OPTIMIZATION_PASSES, DEFAULT_RETRY_THRESHOLD,
};

fn metrics(
    issue_count: usize,
    hard_edge_count: usize,
    critical_path_length: usize,
) -> GraphMetrics {
    GraphMetrics {
        issue_count,
        hard_edge_count,
        critical_path_length,
    }
}

#[test]
fn default_policy_threshold_is_65() {
    assert_eq!(DEFAULT_RETRY_THRESHOLD, 65);
    assert_eq!(RetryPolicy::default().threshold, 65);
}

#[test]
fn default_policy_max_passes_is_1() {
    assert_eq!(DEFAULT_MAX_OPTIMIZATION_PASSES, 1);
    assert_eq!(RetryPolicy::default().max_passes, 1);
}

#[test]
fn should_retry_returns_true_at_score_64() {
    assert!(should_retry(64, &RetryPolicy::default(), 0));
}

#[test]
fn should_retry_returns_false_at_score_65() {
    assert!(!should_retry(65, &RetryPolicy::default(), 0));
}

#[test]
fn should_retry_returns_false_at_score_66() {
    assert!(!should_retry(66, &RetryPolicy::default(), 0));
}

#[test]
fn should_retry_refuses_a_second_pass() {
    assert!(!should_retry(10, &RetryPolicy::default(), 1));
}

#[test]
fn max_passes_is_configurable_upward() {
    let policy = RetryPolicy {
        threshold: 65,
        max_passes: 2,
    };
    assert!(should_retry(10, &policy, 1));
    assert!(!should_retry(10, &policy, 2));
}

#[test]
fn compare_reports_edge_count_delta() {
    let before = metrics(57, 13, 9);
    let after = metrics(58, 5, 5);
    let summary = compare(&before, &after);
    assert_eq!(summary.edge_delta, -8);
    assert_eq!(summary.removed, 8);
    assert_eq!(summary.added, 0);
    assert_eq!(summary.critical_path_before, 9);
    assert_eq!(summary.critical_path_after, 5);
}

#[test]
fn compare_counts_added_edges_when_graph_gains_edges() {
    let before = metrics(10, 4, 3);
    let after = metrics(10, 7, 3);
    let summary = compare(&before, &after);
    assert_eq!(summary.edge_delta, 3);
    assert_eq!(summary.added, 3);
    assert_eq!(summary.removed, 0);
}

#[test]
fn compare_counts_split_and_merged_issues() {
    let split = compare(&metrics(10, 8, 4), &metrics(12, 8, 4));
    assert_eq!(split.split, 2);
    assert_eq!(split.merged, 0);

    let merged = compare(&metrics(12, 8, 4), &metrics(10, 8, 4));
    assert_eq!(merged.merged, 2);
    assert_eq!(merged.split, 0);
}

#[test]
fn compare_rejects_worsened_critical_path() {
    let before = metrics(20, 15, 5);
    let after = metrics(21, 9, 8);
    let summary = compare(&before, &after);
    assert!(summary.rejected);
}

#[test]
fn compare_accepts_equal_critical_path() {
    let summary = compare(&metrics(20, 15, 5), &metrics(20, 10, 5));
    assert!(!summary.rejected);
}

#[test]
fn compare_accepts_shortened_critical_path() {
    let summary = compare(&metrics(20, 15, 9), &metrics(20, 13, 5));
    assert!(!summary.rejected);
}

#[test]
fn summary_renders_section_25_change_list() {
    let summary = OptimizationSummary {
        removed: 8,
        added: 0,
        split: 2,
        merged: 0,
        edge_delta: -8,
        critical_path_before: 9,
        critical_path_after: 5,
        rejected: false,
    };
    let rendered = summary.render();
    assert!(rendered.contains("Concurrency review:"));
    assert!(rendered.contains("removed 8 unnecessary dependency edges"));
    assert!(rendered.contains("split 2 broad ownership issues"));
    assert!(rendered.contains("reduced critical path from 9 to 5"));
}

#[test]
fn rejected_summary_renders_refusal_line() {
    let summary = compare(&metrics(20, 15, 5), &metrics(20, 9, 8));
    let rendered = summary.render();
    assert!(rendered.contains("REJECTED"));
    assert!(rendered.contains("critical path grew from 5 to 8"));
}

#[test]
fn graph_metrics_round_trip_through_json() {
    let before = metrics(57, 13, 9);
    let encoded = serde_json::to_string(&before).expect("encode metrics");
    let decoded: GraphMetrics = serde_json::from_str(&encoded).expect("decode metrics");
    assert_eq!(decoded, before);
}
