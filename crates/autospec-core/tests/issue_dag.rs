//! Issue-DAG model, waves, critical path, and parallelization score.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! — §20 (graph metrics), §21 (parallelization score), §24 (execution waves),
//! §28 (proposed Rust data structures), §29 (algorithm requirements),
//! §35.1 (unit test plan). Real in-memory graphs, no mocks.

use autospec_core::graph::{
    metrics, DependencyEdge, DependencyReason, GraphErrorKind, GraphMetrics, IssueGraph,
    PlannedIssue,
};
use autospec_core::graph::{ConcurrencyMetadata, Ownership};

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

/// 5-node diamond holding 2 roots: A, B ready at once; C waits on A; D on B;
/// E on both.
fn diamond() -> IssueGraph {
    graph(
        &["a", "b", "c", "d", "e"],
        &[("a", "c"), ("b", "d"), ("c", "e"), ("d", "e")],
    )
}

fn linear_chain() -> IssueGraph {
    graph(
        &["a", "b", "c", "d", "e"],
        &[("a", "b"), ("b", "c"), ("c", "d"), ("d", "e")],
    )
}

fn wide_roots() -> IssueGraph {
    graph(&["a", "b", "c", "d", "e"], &[])
}

fn score_in_range(metrics: &GraphMetrics) {
    assert!(
        (0..=100).contains(&metrics.parallelization_score),
        "score {} out of 0..=100",
        metrics.parallelization_score
    );
}

#[test]
fn empty_graph_reports_zero_metrics_and_no_cycle() {
    let g = graph(&[], &[]);

    assert!(g.detect_cycle().is_none());
    assert_eq!(
        g.waves().expect("empty graph is acyclic"),
        Vec::<Vec<String>>::new()
    );
    assert_eq!(g.critical_path_length(), 0);
    assert_eq!(g.root_count(), 0);
    assert_eq!(g.leaf_count(), 0);

    let m = metrics(&g, 32);
    assert_eq!(m.issue_count, 0);
    assert_eq!(m.hard_edge_count, 0);
    assert_eq!(m.initial_width, 0);
    assert_eq!(m.maximum_width, 0);
    assert_eq!(m.average_wave_width, 0.0);
    assert_eq!(m.serialization_ratio, 0.0);
    score_in_range(&m);
}

#[test]
fn single_node_graph_metrics() {
    let g = graph(&["a"], &[]);

    let waves = g.waves().expect("single node is acyclic");
    assert_eq!(waves, vec![vec!["a".to_string()]]);
    assert_eq!(g.critical_path_length(), 1);
    assert_eq!(g.root_count(), 1);
    assert_eq!(g.leaf_count(), 1);

    let m = metrics(&g, 1);
    assert_eq!(m.initial_width, 1);
    assert_eq!(m.maximum_width, 1);
    assert_eq!(m.critical_path_length, 1);
    assert!((m.initial_saturation - 1.0).abs() < 1e-9);
    assert!((m.peak_saturation - 1.0).abs() < 1e-9);
    assert_eq!(m.serialization_ratio, 0.0);
    score_in_range(&m);
}

#[test]
fn wide_roots_collapse_into_one_wave() {
    let g = wide_roots();

    let waves = g.waves().expect("independent nodes are acyclic");
    assert_eq!(waves.len(), 1);
    assert_eq!(waves[0], vec!["a", "b", "c", "d", "e"]);
    assert_eq!(g.critical_path_length(), 1);

    let m = metrics(&g, 100);
    assert_eq!(m.initial_width, 5);
    assert_eq!(m.maximum_width, 5);
    assert_eq!(m.root_count, 5);
    assert_eq!(m.leaf_count, 5);
    assert!((m.serialization_ratio - 0.0).abs() < 1e-9);
    score_in_range(&m);
}

#[test]
fn linear_chain_maximizes_critical_path() {
    let g = linear_chain();

    let waves = g.waves().expect("chain is acyclic");
    assert_eq!(
        waves,
        vec![
            vec!["a".to_string()],
            vec!["b".to_string()],
            vec!["c".to_string()],
            vec!["d".to_string()],
            vec!["e".to_string()]
        ]
    );
    assert_eq!(g.critical_path_length(), 5);

    let m = metrics(&g, 4);
    assert_eq!(m.critical_path_length, 5);
    assert_eq!(m.initial_width, 1);
    assert_eq!(m.maximum_width, 1);
    // 4 hard edges over max(1, 5*4/2) = 10 possible pairs.
    assert!((m.serialization_ratio - 0.4).abs() < 1e-9);
    score_in_range(&m);
}

#[test]
fn diamond_returns_three_waves_holding_two_roots() {
    let g = diamond();

    let waves = g.waves().expect("diamond is acyclic");
    assert_eq!(
        waves,
        vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["c".to_string(), "d".to_string()],
            vec!["e".to_string()]
        ]
    );
    assert_eq!(g.root_count(), 2);
    assert_eq!(g.leaf_count(), 1);
    assert_eq!(g.critical_path_length(), 3);

    let m = metrics(&g, 2);
    assert_eq!(m.initial_width, 2);
    assert_eq!(m.maximum_width, 2);
    assert_eq!(m.critical_path_length, 3);
    // average of 2, 2, 1.
    assert!((m.average_wave_width - 5.0 / 3.0).abs() < 1e-9);
    score_in_range(&m);
}

#[test]
fn fan_in_converges_many_roots_into_one_leaf() {
    let g = graph(&["a", "b", "c", "z"], &[("a", "z"), ("b", "z"), ("c", "z")]);

    let waves = g.waves().expect("fan-in is acyclic");
    assert_eq!(waves.len(), 2);
    assert_eq!(waves[0], vec!["a", "b", "c"]);
    assert_eq!(waves[1], vec!["z"]);
    assert_eq!(g.critical_path_length(), 2);
    assert_eq!(g.root_count(), 3);
    assert_eq!(g.leaf_count(), 1);
    score_in_range(&metrics(&g, 3));
}

#[test]
fn fan_out_spreads_one_root_into_many_leaves() {
    let g = graph(&["z", "a", "b", "c"], &[("z", "a"), ("z", "b"), ("z", "c")]);

    let waves = g.waves().expect("fan-out is acyclic");
    assert_eq!(waves[0], vec!["z"]);
    assert_eq!(waves[1], vec!["a", "b", "c"]);
    assert_eq!(g.critical_path_length(), 2);
    assert_eq!(g.root_count(), 1);
    assert_eq!(g.leaf_count(), 3);
    score_in_range(&metrics(&g, 10));
}

#[test]
fn cycle_a_to_b_to_a_returns_both_nodes() {
    let g = graph(&["a", "b"], &[("a", "b"), ("b", "a")]);

    let cycle = g
        .detect_cycle()
        .expect("A->B->A must be detected as a cycle");
    assert!(
        cycle.iter().any(|node| node == "a"),
        "cycle {cycle:?} missing a"
    );
    assert!(
        cycle.iter().any(|node| node == "b"),
        "cycle {cycle:?} missing b"
    );
    assert_eq!(cycle.len(), 2);
}

#[test]
fn cycle_makes_waves_fail_with_cycle_error() {
    let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("c", "a")]);

    let error = g.waves().expect_err("cyclic graph must fail waves");
    assert_eq!(error.kind, GraphErrorKind::Cycle);
    assert!(error.cycle.contains(&"a".to_string()));
    assert!(error.cycle.contains(&"b".to_string()));
    assert!(error.cycle.contains(&"c".to_string()));
}

#[test]
fn wave_ordering_is_deterministic_across_input_order() {
    let mut shuffled = diamond();
    shuffled.issues = vec![issue("e"), issue("a"), issue("d"), issue("b"), issue("c")];

    assert_eq!(
        shuffled
            .waves()
            .expect("diamond is acyclic regardless of input order"),
        diamond().waves().expect("diamond is acyclic")
    );
}

#[test]
fn serialization_ratio_distinguishes_chain_from_independent() {
    let chain = metrics(&linear_chain(), 4);
    let wide = metrics(&wide_roots(), 4);

    assert!((chain.serialization_ratio - 0.4).abs() < 1e-9);
    assert!((wide.serialization_ratio - 0.0).abs() < 1e-9);
}

#[test]
fn parallelization_score_in_range_for_every_fixture() {
    let fixtures: Vec<IssueGraph> = vec![
        graph(&[], &[]),
        graph(&["a"], &[]),
        wide_roots(),
        linear_chain(),
        diamond(),
        graph(&["a", "b", "c", "z"], &[("a", "z"), ("b", "z"), ("c", "z")]),
        graph(&["z", "a", "b", "c"], &[("z", "a"), ("z", "b"), ("z", "c")]),
    ];

    for g in &fixtures {
        for capacity in [1usize, 2, 10, 100] {
            score_in_range(&metrics(g, capacity));
        }
    }
}

#[test]
fn zero_capacity_saturation_is_zero_not_a_divide_by_zero() {
    let m = metrics(&diamond(), 0);

    assert!((m.initial_saturation - 0.0).abs() < 1e-9);
    assert!((m.peak_saturation - 0.0).abs() < 1e-9);
    score_in_range(&m);
}

#[test]
fn saturation_is_capped_by_width() {
    // Wide graph, tiny fleet: saturation caps at 1.0 (width >= capacity).
    let wide = metrics(&wide_roots(), 2);
    assert!((wide.initial_saturation - 1.0).abs() < 1e-9);

    // Small graph, big fleet: saturation reflects the small width.
    let small = metrics(&graph(&["a", "b"], &[]), 10);
    assert!((small.initial_saturation - 0.2).abs() < 1e-9);
    assert!((small.peak_saturation - 0.2).abs() < 1e-9);
}

#[test]
fn shared_write_hotspots_count_contested_surfaces() {
    let mut g = graph(&["a", "b", "c"], &[]);
    g.issues[0].ownership.shared_write = vec![autospec_core::graph::OwnedSurface {
        path: "src/main.rs".to_string(),
        symbols: Vec::new(),
    }];
    g.issues[1].ownership.shared_write = vec![
        autospec_core::graph::OwnedSurface {
            path: "src/main.rs".to_string(),
            symbols: Vec::new(),
        },
        autospec_core::graph::OwnedSurface {
            path: "src/util.rs".to_string(),
            symbols: vec!["helper".to_string()],
        },
    ];
    g.issues[2].ownership.shared_write = vec![autospec_core::graph::OwnedSurface {
        path: "src/util.rs".to_string(),
        symbols: vec!["helper".to_string()],
    }];

    let m = metrics(&g, 4);
    // Two contested surfaces: src/main.rs (2 issues) and src/util.rs (2
    // issues; the shared symbol does not add a third hotspot).
    assert_eq!(m.shared_write_hotspots, 2);
    score_in_range(&m);
}

#[test]
fn justified_edges_score_higher_than_artifactless_ones() {
    let mut justified = graph(&["a", "b"], &[("a", "b")]);
    justified.hard_edges[0].artifact = Some("docs/api.md".to_string());
    let unjustified = graph(&["a", "b"], &[("a", "b")]);

    let score_justified = metrics(&justified, 2).parallelization_score;
    let score_unjustified = metrics(&unjustified, 2).parallelization_score;

    assert!(
        score_justified > score_unjustified,
        "justified score {score_justified} should beat unjustified {score_unjustified}"
    );
}
