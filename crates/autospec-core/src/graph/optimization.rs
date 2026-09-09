//! Concurrency-optimization retry policy and before/after graph comparison.
//!
//! Implements spec sections 23 (automatic concurrency retry) and 25
//! (planner output summary) of
//! `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`.
//! The threshold and pass cap are single constants here; callers must not
//! scatter literals.

use serde::{Deserialize, Serialize};

/// Retry when the parallelization score falls strictly below this value.
pub const DEFAULT_RETRY_THRESHOLD: u8 = 65;

/// Maximum automatic optimization passes; a second pass MAY be allowed via
/// configuration (`RetryPolicy { max_passes, .. }`), never by default.
pub const DEFAULT_MAX_OPTIMIZATION_PASSES: u8 = 1;

/// Score-65 retry policy with a hard cap on automatic optimization passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Scores strictly below this value trigger a retry.
    pub threshold: u8,
    /// Maximum number of automatic optimization passes allowed.
    pub max_passes: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_RETRY_THRESHOLD,
            max_passes: DEFAULT_MAX_OPTIMIZATION_PASSES,
        }
    }
}

/// Decide whether a concurrency optimization retry runs.
///
/// Returns `true` only while the score is below the policy threshold AND the
/// automatic pass budget is not exhausted, so a retry can never loop
/// indefinitely regardless of how low the score stays.
pub fn should_retry(score: u8, policy: &RetryPolicy, passes: u8) -> bool {
    score < policy.threshold && passes < policy.max_passes
}

/// Measured shape of a decomposition graph, as produced by the DAG analyzer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphMetrics {
    /// Number of issue nodes in the graph.
    pub issue_count: usize,
    /// Number of hard dependency edges.
    pub hard_edge_count: usize,
    /// Length of the longest hard-dependency chain.
    pub critical_path_length: usize,
}

/// Before/after diff of one concurrency optimization pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptimizationSummary {
    /// Dependency edges removed by the pass.
    pub removed: usize,
    /// Dependency edges added by the pass.
    pub added: usize,
    /// Issues split into narrower ownership scopes.
    pub split: usize,
    /// Issues merged together by the pass.
    pub merged: usize,
    /// Signed edge-count delta (`after - before`).
    pub edge_delta: isize,
    /// Critical path length before the pass.
    pub critical_path_before: usize,
    /// Critical path length after the pass.
    pub critical_path_after: usize,
    /// `true` when the after-graph must be rejected: an optimization pass may
    /// not lengthen the critical path.
    pub rejected: bool,
}

/// Compare the graph metrics before and after one optimization pass.
///
/// The after-graph is rejected when its critical path grew: a retry re-runs
/// planner output only if the new graph is not worse on the serialization
/// dimension it was meant to improve.
pub fn compare(before: &GraphMetrics, after: &GraphMetrics) -> OptimizationSummary {
    let edge_delta = after.hard_edge_count as isize - before.hard_edge_count as isize;
    let issue_delta = after.issue_count as isize - before.issue_count as isize;
    OptimizationSummary {
        removed: edge_delta.min(0).unsigned_abs(),
        added: edge_delta.max(0) as usize,
        split: issue_delta.max(0) as usize,
        merged: issue_delta.min(0).unsigned_abs(),
        edge_delta,
        critical_path_before: before.critical_path_length,
        critical_path_after: after.critical_path_length,
        rejected: after.critical_path_length > before.critical_path_length,
    }
}

impl OptimizationSummary {
    /// Render the section 25 "Concurrency review" change summary block.
    pub fn render(&self) -> String {
        let mut lines = vec![
            "Concurrency review:".to_string(),
            format!("- removed {} unnecessary dependency edges", self.removed),
            format!("- added {} dependency edges", self.added),
            format!("- split {} broad ownership issues", self.split),
            format!("- merged {} issues", self.merged),
        ];
        if self.rejected {
            lines.push(format!(
                "- REJECTED: retry refused, critical path grew from {} to {}",
                self.critical_path_before, self.critical_path_after
            ));
        } else {
            lines.push(format!(
                "- reduced critical path from {} to {}",
                self.critical_path_before, self.critical_path_after
            ));
        }
        lines.join("\n")
    }
}
