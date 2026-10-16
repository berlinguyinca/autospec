//! Delivery benchmark: the 10-task set, the A1-A9 scenario catalog, and
//! baseline-relative aggregation of recorded results.
//!
//! Spec: `docs/specs/2026-10-14-pi-autospec-autonomous-delivery-design.md`
//! section 7. This module is a pure library over recorded result rows: it
//! starts no process, injects no fault and writes no file. Thresholds are
//! never invented; a candidate is only ever compared with a baseline that was
//! measured.
//!
//! Percentiles are nearest-rank over successful runs. A failed run lowers the
//! success rate and is excluded from every percentile, so a failure cannot hide
//! inside a good-looking p95. A task whose baseline arm has no successful run
//! reports [`Comparison::BaselineMissing`] instead of a number.

use serde::{Deserialize, Serialize};

/// The ten task kinds every arm runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BenchTask {
    TrivialFix,
    MediumFeature,
    ComplexRustChange,
    MultiRepoContract,
    FailingTest,
    GatewayBusy,
    StaleCi,
    PatchCollision,
    Crash,
    LongTest,
}

impl BenchTask {
    /// Every task kind, in report order.
    pub const ALL: [BenchTask; 10] = [
        BenchTask::TrivialFix,
        BenchTask::MediumFeature,
        BenchTask::ComplexRustChange,
        BenchTask::MultiRepoContract,
        BenchTask::FailingTest,
        BenchTask::GatewayBusy,
        BenchTask::StaleCi,
        BenchTask::PatchCollision,
        BenchTask::Crash,
        BenchTask::LongTest,
    ];

    /// The stable wire name, as it appears in result rows.
    pub fn as_str(&self) -> &'static str {
        match self {
            BenchTask::TrivialFix => "trivial-fix",
            BenchTask::MediumFeature => "medium-feature",
            BenchTask::ComplexRustChange => "complex-rust-change",
            BenchTask::MultiRepoContract => "multi-repo-contract",
            BenchTask::FailingTest => "failing-test",
            BenchTask::GatewayBusy => "gateway-busy",
            BenchTask::StaleCi => "stale-ci",
            BenchTask::PatchCollision => "patch-collision",
            BenchTask::Crash => "crash",
            BenchTask::LongTest => "long-test",
        }
    }
}

/// Which system ran a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Arm {
    /// Standalone Pi, the measured baseline.
    Baseline,
    /// Pi with AutoSpec.
    PiAutospec,
}

/// One zip acceptance scenario and the section 7 items it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scenario {
    pub id: &'static str,
    /// Numbers from the section 7 acceptance list; empty when the spec maps none.
    pub acceptance_items: &'static [u8],
    pub summary: &'static str,
}

/// A1-A9, mandatory.
pub const SCENARIOS: [Scenario; 9] = [
    Scenario {
        id: "A1",
        acceptance_items: &[10],
        summary: "hanging 8 h Rust test detected in minutes with last progress and process tree",
    },
    Scenario {
        id: "A2",
        acceptance_items: &[11],
        summary: "overnight 20-worker build storm stays inside budgets, cache reuse, no Cargo lock convoy",
    },
    Scenario {
        id: "A3",
        acceptance_items: &[12],
        summary: "merge triggers idempotent, restart-safe cleanup that loses no dirty work",
    },
    Scenario {
        id: "A4",
        acceptance_items: &[9],
        summary: "capacity failure switches to a qualified model without thrash or duplicate tool effects, else bounded BLOCKED",
    },
    Scenario {
        id: "A5",
        acceptance_items: &[7, 8],
        summary: "concurrent PRs revalidate on stale SHA, protections honored, optional CI does not hold progress",
    },
    Scenario {
        id: "A6",
        acceptance_items: &[4, 5],
        summary: "supervisor killed mid review, merge or cleanup resumes without duplicate merge or unsafe deletion",
    },
    Scenario {
        id: "A7",
        acceptance_items: &[13],
        summary: "HerdR shows live state and replay after restart; missing telemetry labelled unknown or stale",
    },
    Scenario {
        id: "A8",
        acceptance_items: &[2, 7],
        summary: "mixed-repo mission correlated end to end; dependencies unblock only after verified gates",
    },
    Scenario {
        id: "A9",
        acceptance_items: &[],
        summary: "faults in disk, permissions, test runner, GitHub, inference and network give bounded retries, diagnostics and safe pause",
    },
];

/// One recorded run of one task by one arm. Tokens, retries and disk are
/// aggregated over successful runs only, like the time percentiles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub task: BenchTask,
    pub arm: Arm,
    pub time_to_verified_secs: f64,
    pub tokens: u64,
    pub retries: u32,
    pub disk_bytes: u64,
    pub success: bool,
}

/// Aggregate of one arm on one task.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArmSummary {
    pub runs: usize,
    pub successes: usize,
    /// `successes / runs`; `None` when there were no runs.
    pub success_rate: Option<f64>,
    /// Over successful runs only; `None` when there were none.
    pub p50_secs: Option<f64>,
    pub p95_secs: Option<f64>,
    /// Nearest-rank median over successful runs.
    pub median_tokens: Option<u64>,
    pub median_retries: Option<u32>,
    pub median_disk_bytes: Option<u64>,
}

/// How the candidate arm compares with the measured baseline.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "comparison", rename_all = "kebab-case")]
pub enum Comparison {
    /// Both arms have a successful run. Deltas are candidate minus baseline,
    /// so negative is faster.
    Measured {
        p50_delta_secs: f64,
        p95_delta_secs: f64,
    },
    /// The baseline arm has no successful run: there is nothing to compare to.
    BaselineMissing,
    /// The baseline is measured but the candidate arm has no successful run.
    CandidateMissing,
}

/// One task's two arms and their comparison.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskSummary {
    pub task: BenchTask,
    pub baseline: ArmSummary,
    pub pi_autospec: ArmSummary,
    pub comparison: Comparison,
}

/// Nearest-rank percentile of an ascending, non-empty slice: the value at
/// rank `ceil(p/100 * n)`, one-based.
fn nearest_rank<T: Copy>(sorted: &[T], percent: u32) -> Option<T> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    let rank = (n * percent as usize).div_ceil(100).clamp(1, n);
    Some(sorted[rank - 1])
}

/// A run counts as a success only if it says so and its time is a real,
/// non-negative number. A row claiming success with a NaN, infinite or
/// negative time is bad data: it counts as a failure rather than becoming a
/// percentile.
fn counts_as_success(row: &RunResult) -> bool {
    row.success && row.time_to_verified_secs.is_finite() && row.time_to_verified_secs >= 0.0
}

fn summarize_arm(rows: &[&RunResult]) -> ArmSummary {
    let ok: Vec<&&RunResult> = rows.iter().filter(|row| counts_as_success(row)).collect();
    let mut times: Vec<f64> = ok.iter().map(|row| row.time_to_verified_secs).collect();
    times.sort_by(f64::total_cmp);
    let mut tokens: Vec<u64> = ok.iter().map(|row| row.tokens).collect();
    tokens.sort_unstable();
    let mut retries: Vec<u32> = ok.iter().map(|row| row.retries).collect();
    retries.sort_unstable();
    let mut disk: Vec<u64> = ok.iter().map(|row| row.disk_bytes).collect();
    disk.sort_unstable();
    ArmSummary {
        runs: rows.len(),
        successes: ok.len(),
        success_rate: (!rows.is_empty()).then(|| ok.len() as f64 / rows.len() as f64),
        p50_secs: nearest_rank(&times, 50),
        p95_secs: nearest_rank(&times, 95),
        median_tokens: nearest_rank(&tokens, 50),
        median_retries: nearest_rank(&retries, 50),
        median_disk_bytes: nearest_rank(&disk, 50),
    }
}

/// Summarize every task kind, in [`BenchTask::ALL`] order, including kinds
/// with no recorded runs.
pub fn summarize(results: &[RunResult]) -> Vec<TaskSummary> {
    BenchTask::ALL
        .iter()
        .map(|task| {
            let arm = |which: Arm| -> ArmSummary {
                let rows: Vec<&RunResult> = results
                    .iter()
                    .filter(|row| row.task == *task && row.arm == which)
                    .collect();
                summarize_arm(&rows)
            };
            let baseline = arm(Arm::Baseline);
            let pi_autospec = arm(Arm::PiAutospec);
            let comparison = match (baseline.p50_secs, baseline.p95_secs) {
                (Some(base_p50), Some(base_p95)) => {
                    match (pi_autospec.p50_secs, pi_autospec.p95_secs) {
                        (Some(p50), Some(p95)) => Comparison::Measured {
                            p50_delta_secs: p50 - base_p50,
                            p95_delta_secs: p95 - base_p95,
                        },
                        _ => Comparison::CandidateMissing,
                    }
                }
                _ => Comparison::BaselineMissing,
            };
            TaskSummary {
                task: *task,
                baseline,
                pi_autospec,
                comparison,
            }
        })
        .collect()
}
