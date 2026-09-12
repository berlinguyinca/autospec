//! Automatic concurrency retry policy and optimizer change summary.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! — §23 (automatic concurrency retry), §25 (planner output summary).
//!
//! The retry policy gates how many dedicated optimization passes the
//! planner may run after the initial decomposition: it fires only when the
//! advisory [`GraphMetrics::parallelization_score`] falls below the
//! threshold, and it is capped at a hard number of passes so a low score
//! can never burn the fleet budget by regenerating indefinitely. The
//! change summary diffs a before/after pair of [`GraphMetrics`] and rejects
//! an "optimized" graph whose critical path grew.

use crate::graph::metrics::GraphMetrics;
use serde::{Deserialize, Serialize};

/// §23 default: `parallelization_score < 65` triggers the retry.
pub const DEFAULT_THRESHOLD: u8 = 65;
/// §23: "Maximum automatic optimization passes: 1".
pub const DEFAULT_MAX_PASSES: u8 = 1;

/// §23 automatic concurrency retry policy.
///
/// A retry is offered only while the score is below [`Self::threshold`] and
/// fewer than [`Self::max_passes`] passes have already run. [`max_passes`]
/// defaults to 1; a second pass MAY be allowed via configuration (a larger
/// value), never an unbounded loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Retry while `score < threshold`.
    pub threshold: u8,
    /// Hard cap on automatic optimization passes (≥1; a second pass is
    /// allowed by configuring this upward).
    pub max_passes: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_THRESHOLD,
            max_passes: DEFAULT_MAX_PASSES,
        }
    }
}

impl RetryPolicy {
    /// Policy with an explicit pass cap over the default threshold.
    pub fn with_max_passes(max_passes: u8) -> Self {
        Self {
            threshold: DEFAULT_THRESHOLD,
            max_passes,
        }
    }
}

/// §23: should the planner run another optimization pass?
///
/// `true` only when `score < policy.threshold` **and** `passes` (passes
/// already run) is below the cap. A score at or above the threshold never
/// retries; a second pass is refused once one has already run.
pub fn should_retry(score: u8, policy: &RetryPolicy, passes: u8) -> bool {
    score < policy.threshold && passes < policy.max_passes
}

/// §25 optimizer change summary for a before/after graph pair.
///
/// `removed`/`added` count hard-dependency edges dropped or introduced by
/// the pass; `split`/`merged` count issues that were split into more issues
/// or collapsed into fewer. `rejected` is set when the after-graph's
/// critical path grew, which is a correctness regression the pass must not
/// accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizationSummary {
    /// Hard edges removed: `before - after` (saturating, 0 if none).
    pub removed: usize,
    /// Hard edges added: `after - before` (saturating, 0 if none).
    pub added: usize,
    /// Issues split into more issues: `after - before` issue count.
    pub split: usize,
    /// Issues merged into fewer issues: `before - after` issue count.
    pub merged: usize,
    /// Signed edge-count delta: `after - before` (negative when removed).
    pub edge_count_delta: i64,
    /// Set when the after-graph's critical path grew — the retry is
    /// rejected (correctness validation failed).
    pub rejected: bool,
}

/// §25: diff two [`GraphMetrics`] snapshots into an [`OptimizationSummary`].
///
/// The after-graph is rejected when its critical path length exceeds the
/// before-graph's — an "improved" graph whose longest hard-dependency chain
/// got longer is a regression, not an improvement.
pub fn compare(before: &GraphMetrics, after: &GraphMetrics) -> OptimizationSummary {
    let edge_count_delta = after.hard_edge_count as i64 - before.hard_edge_count as i64;
    OptimizationSummary {
        removed: before.hard_edge_count.saturating_sub(after.hard_edge_count),
        added: after.hard_edge_count.saturating_sub(before.hard_edge_count),
        split: after.issue_count.saturating_sub(before.issue_count),
        merged: before.issue_count.saturating_sub(after.issue_count),
        edge_count_delta,
        rejected: after.critical_path_length > before.critical_path_length,
    }
}
