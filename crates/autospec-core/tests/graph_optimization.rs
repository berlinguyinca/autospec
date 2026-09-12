//! §23 automatic concurrency retry policy and §25 optimizer change summary.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! — §23 (retry), §25 (summary), §35.1 (unit test plan). Real in-memory
//! graphs via [`metrics`], no mocks.

use autospec_core::graph::{
    compare, metrics, should_retry, DependencyEdge, DependencyReason, GraphMetrics, IssueGraph,
    OptimizationSummary, PlannedIssue, RetryPolicy, DEFAULT_MAX_PASSES, DEFAULT_THRESHOLD,
};
use autospec_core::graph::{ConcurrencyMetadata, Ownership};

const CAPACITY: usize = 32;

fn issue(id: &str) -> PlannedIssue {
    PlannedIssue {
        id: id.to_string(),
        title: id.to_string(),
        ownership: Ownership {
            exclusive: Vec::new(),
            shared_read: Vec::new(),
            shared_write: Vec::new(),
        },
        concurrency: ConcurrencyMetadata {
            parallel_safe: true,
            conflict_domains: Vec::new(),
        },
    }
}

fn edge(predecessor: &str, successor: &str) -> DependencyEdge {
    DependencyEdge {
        predecessor: predecessor.to_string(),
        successor: successor.to_string(),
        reason: DependencyReason::ConsumesNewInterface,
        artifact: None,
    }
}

fn graph(ids: &[&str], edges: &[(&str, &str)]) -> IssueGraph {
    IssueGraph {
        issues: ids.iter().map(|id| issue(id)).collect(),
        hard_edges: edges.iter().map(|(p, s)| edge(p, s)).collect(),
    }
}

fn metrics_of(graph: &IssueGraph) -> GraphMetrics {
    metrics(graph, CAPACITY)
}

// --- should_retry ---------------------------------------------------------

#[test]
fn default_policy_matches_spec_constants() {
    let policy = RetryPolicy::default();
    assert_eq!(policy.threshold, 65);
    assert_eq!(policy.max_passes, 1);
    assert_eq!(DEFAULT_THRESHOLD, 65);
    assert_eq!(DEFAULT_MAX_PASSES, 1);
}

#[test]
fn should_retry_fires_below_threshold() {
    let policy = RetryPolicy::default();
    assert!(should_retry(64, &policy, 0));
}

#[test]
fn should_retry_does_not_fire_at_threshold() {
    let policy = RetryPolicy::default();
    assert!(!should_retry(65, &policy, 0));
}

#[test]
fn should_retry_does_not_fire_above_threshold() {
    let policy = RetryPolicy::default();
    assert!(!should_retry(66, &policy, 0));
}

#[test]
fn should_retry_refuses_second_pass_by_default() {
    let policy = RetryPolicy::default();
    // Below threshold, but one pass already ran — the hard cap is 1.
    assert!(!should_retry(64, &policy, 1));
}

#[test]
fn should_retry_allows_second_pass_when_configured_upward() {
    let policy = RetryPolicy::with_max_passes(2);
    assert!(should_retry(64, &policy, 0));
    assert!(should_retry(64, &policy, 1));
    // The second pass ran; now even a configured-2 policy refuses a third.
    assert!(!should_retry(64, &policy, 2));
}

#[test]
fn should_retry_drives_off_real_graph_score() {
    // A deep chain is a low parallelization score (long critical path);
    // the decision must track that real score against the threshold.
    let chain = graph(
        &["a", "b", "c", "d", "e", "f", "g"],
        &[
            ("a", "b"),
            ("b", "c"),
            ("c", "d"),
            ("d", "e"),
            ("e", "f"),
            ("f", "g"),
        ],
    );
    let m_chain = metrics_of(&chain);
    // The decision is a pure function of the (real) score and the cap.
    assert_eq!(
        should_retry(m_chain.parallelization_score, &RetryPolicy::default(), 0),
        m_chain.parallelization_score < 65
    );
    // ...and never a second pass regardless of score.
    assert!(!should_retry(
        m_chain.parallelization_score,
        &RetryPolicy::default(),
        1
    ));
}

// --- compare --------------------------------------------------------------

#[test]
fn compare_reports_removed_edges() {
    let before = graph(
        &["a", "b", "c", "d", "e"],
        &[("a", "c"), ("b", "d"), ("c", "e"), ("d", "e")],
    );
    let after = graph(&["a", "b", "c", "d", "e"], &[("a", "c"), ("c", "e")]);
    let b = metrics_of(&before);
    let a = metrics_of(&after);

    let summary = compare(&b, &a);
    assert_eq!(summary.removed, 2);
    assert_eq!(summary.added, 0);
    assert_eq!(summary.edge_count_delta, -2);
}

#[test]
fn compare_reports_added_edges() {
    let before = graph(&["a", "b", "c"], &[("a", "c")]);
    let after = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
    let b = metrics_of(&before);
    let a = metrics_of(&after);

    let summary = compare(&b, &a);
    assert_eq!(summary.added, 1);
    assert_eq!(summary.removed, 0);
    assert_eq!(summary.edge_count_delta, 1);
}

#[test]
fn compare_reports_split_and_merge() {
    // Split: the pass broke a broad issue into more.
    let split_before = graph(&["a", "b", "c"], &[]);
    let split_after = graph(&["a", "a2", "a3", "b", "c"], &[]);
    let s = compare(&metrics_of(&split_before), &metrics_of(&split_after));
    assert_eq!(s.split, 2);
    assert_eq!(s.merged, 0);

    // Merge: the pass collapsed issues into fewer.
    let merge_before = graph(&["a", "a2", "a3", "b", "c"], &[]);
    let merge_after = graph(&["a", "b", "c"], &[]);
    let m = compare(&metrics_of(&merge_before), &metrics_of(&merge_after));
    assert_eq!(m.merged, 2);
    assert_eq!(m.split, 0);
}

#[test]
fn compare_rejects_grown_critical_path() {
    let before = graph(&["a", "b"], &[("a", "b")]); // critical path 2
    let after = graph(
        &["a", "b", "c"],
        &[("a", "b"), ("b", "c")], // critical path 3
    );
    let b = metrics_of(&before);
    let a = metrics_of(&after);
    assert_eq!(b.critical_path_length, 2);
    assert_eq!(a.critical_path_length, 3);

    let summary = compare(&b, &a);
    assert!(
        summary.rejected,
        "a grown critical path must reject the pass"
    );
}

#[test]
fn compare_accepts_shrunk_critical_path() {
    let before = graph(
        &["a", "b", "c"],
        &[("a", "b"), ("b", "c")], // critical path 3
    );
    let after = graph(&["a", "b"], &[("a", "b")]); // critical path 2
    let summary = compare(&metrics_of(&before), &metrics_of(&after));
    assert!(!summary.rejected);
}

#[test]
fn compare_equal_graphs_is_a_noop() {
    let g1 = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
    let g2 = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
    let summary = compare(&metrics_of(&g1), &metrics_of(&g2));
    assert_eq!(
        summary,
        OptimizationSummary {
            removed: 0,
            added: 0,
            split: 0,
            merged: 0,
            edge_count_delta: 0,
            rejected: false,
        }
    );
}
