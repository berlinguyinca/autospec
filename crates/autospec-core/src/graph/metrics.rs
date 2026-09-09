//! Aggregate DAG metrics and the 0–100 parallelization score.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! — §20 (graph metrics), §21 (parallelization score), §28 (proposed Rust
//! data structures).
//!
//! [`metrics`] is the single entry point: it derives every field from an
//! [`IssueGraph`] and a fleet `capacity`, then stamps the advisory
//! [`GraphMetrics::parallelization_score`]. The score is advisory by
//! construction — correctness always wins (§21).

use crate::graph::issue_dag::IssueGraph;
use crate::graph::metadata::normalize_path;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Deterministic aggregate metrics for one issue DAG (§20, §28) plus the
/// fleet saturation values and the advisory score computed by
/// [`metrics`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphMetrics {
    pub issue_count: usize,
    pub hard_edge_count: usize,
    pub root_count: usize,
    pub leaf_count: usize,
    pub initial_width: usize,
    pub maximum_width: usize,
    pub critical_path_length: usize,
    pub average_wave_width: f64,
    /// §20.4: `hard_edge_count / max(1, issue_count * (issue_count - 1) / 2)`.
    pub serialization_ratio: f64,
    /// Shared-write surfaces contested by two or more issues (§29.5).
    pub shared_write_hotspots: usize,
    /// §20.5: `min(initial_width, capacity) / capacity`, 0 when capacity is 0.
    pub initial_saturation: f64,
    /// §20.5: `min(maximum_width, capacity) / capacity`, 0 when capacity is 0.
    pub peak_saturation: f64,
    /// Advisory 0–100 score; see [`GraphMetrics::parallelization_score`].
    pub parallelization_score: u8,

    /// Hard edges with an artifact-backed justification. Derived data, not
    /// serialized; recomputed by [`metrics`].
    #[serde(skip)]
    hard_edges_with_artifact: usize,
}

impl GraphMetrics {
    /// 0–100 advisory score with the §21 weights: 30% initial saturation,
    /// 20% peak saturation, 20% inverse critical-path pressure, 15% low
    /// shared-write overlap, 15% dependency justification quality.
    ///
    /// Inverse critical-path pressure is `1 - critical_path_length /
    /// issue_count` (clamped; zero for an empty graph). Overlap credit is
    /// `1 - shared_write_hotspots / issue_count` (clamped). Justification
    /// quality is the fraction of hard edges carrying an artifact; with no
    /// hard edges there is nothing to justify and the component is full.
    pub fn parallelization_score(&self) -> u8 {
        if self.issue_count == 0 {
            return 0;
        }
        let pressure = (self.critical_path_length as f64 / self.issue_count as f64).clamp(0.0, 1.0);
        let overlap_credit =
            (1.0 - self.shared_write_hotspots as f64 / self.issue_count as f64).clamp(0.0, 1.0);
        let justification = if self.hard_edge_count == 0 {
            1.0
        } else {
            self.hard_edges_with_artifact.min(self.hard_edge_count) as f64
                / self.hard_edge_count as f64
        };
        let weighted = 0.30 * self.initial_saturation
            + 0.20 * self.peak_saturation
            + 0.20 * (1.0 - pressure)
            + 0.15 * overlap_credit
            + 0.15 * justification;
        (weighted * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

/// Compute all DAG metrics for `graph` at fleet `capacity` (§20, §28).
///
/// Cyclic graphs degrade to the acyclic prefix: waves stop where the cycle
/// begins, so widths and the critical path reflect only the resolvable
/// prefix. Cycle detection is the caller's job via
/// [`IssueGraph::detect_cycle`]; this function never fails.
pub fn metrics(graph: &IssueGraph, capacity: usize) -> GraphMetrics {
    let waves = graph.waves().unwrap_or_default();
    let issue_count = graph.issues.len();
    let hard_edge_count = graph.hard_edges.len();
    let root_count = graph.root_count();
    let leaf_count = graph.leaf_count();
    let initial_width = waves.first().map(Vec::len).unwrap_or(0);
    let maximum_width = waves.iter().map(Vec::len).max().unwrap_or(0);
    let critical_path_length = graph.critical_path_length();
    let average_wave_width = if waves.is_empty() {
        0.0
    } else {
        waves.iter().map(Vec::len).sum::<usize>() as f64 / waves.len() as f64
    };
    let possible_pairs = issue_count.saturating_mul(issue_count.saturating_sub(1)) / 2;
    let serialization_ratio = hard_edge_count as f64 / possible_pairs.max(1) as f64;
    let shared_write_hotspots = count_shared_write_hotspots(graph);
    let initial_saturation = saturation(initial_width, capacity);
    let peak_saturation = saturation(maximum_width, capacity);
    let hard_edges_with_artifact = graph
        .hard_edges
        .iter()
        .filter(|edge| edge.artifact.is_some())
        .count();

    let mut result = GraphMetrics {
        issue_count,
        hard_edge_count,
        root_count,
        leaf_count,
        initial_width,
        maximum_width,
        critical_path_length,
        average_wave_width,
        serialization_ratio,
        shared_write_hotspots,
        initial_saturation,
        peak_saturation,
        parallelization_score: 0,
        hard_edges_with_artifact,
    };
    result.parallelization_score = result.parallelization_score();
    result
}

/// §20.5: `min(width, capacity) / capacity`, 0 for a zero-capacity fleet.
fn saturation(width: usize, capacity: usize) -> f64 {
    if capacity == 0 {
        0.0
    } else {
        width.min(capacity) as f64 / capacity as f64
    }
}

/// Count shared-write surfaces (§29.5): normalized paths declared as
/// `shared_write` by two or more issues.
///
/// Each issue counts at most once per normalized path, so one issue listing
/// the same path twice does not self-contest.
fn count_shared_write_hotspots(graph: &IssueGraph) -> usize {
    let mut declared_by: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for issue in &graph.issues {
        for surface in &issue.ownership.shared_write {
            let path = normalize_path(&surface.path);
            declared_by.entry(path).or_default().insert(&issue.id);
        }
    }
    declared_by
        .values()
        .filter(|issues| issues.len() >= 2)
        .count()
}
