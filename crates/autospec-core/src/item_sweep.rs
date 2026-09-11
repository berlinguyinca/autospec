//! A retry job looping over independent items must collect every result and
//! report state, not attempt (issue #4242).
//!
//! The incident: a 60-second systemd unit registered the fleet's models
//! (deepseek-v4-flash, qwen3.8-27b, …) via `curl`. The loop ran under
//! `set -euo pipefail` over *independent* items, so one timed-out item — a
//! `curl` waiting on a stalled download — did not merely fail: it aborted
//! every remaining registration in the round, and the next six runs
//! re-discovered the same fact the hard way. Meanwhile a unit that failed
//! most of the time had taught its operator to treat red as the resting
//! state, so when something genuinely new broke, the red said nothing.
//!
//! This repo had already learned half the lesson once: the
//! reproducible-build job was fixed to "collect the results of all images
//! and fail at the very end with all of them". #4242 makes the rest of the
//! lesson checkable:
//!
//! 1. **One item's failure must not end the loop.** The job declares the
//!    items it will attempt, collects every item's result, and reports at
//!    the end with all failures named. An item with no result is a *failed*
//!    item, not a silent absence — the loop is what aborted.
//!    ([`fold_item_results`]).
//! 2. **A retry job's exit status describes state, not attempt.** 3/4
//!    healthy, 0/4 healthy, and "all were already registered and still
//!    alive" are three different states, and the job must render three
//!    different reports for them. Partial is not success, and no-op is not
//!    work-done. ([`SweepState::exit_code`], [`SweepState::summary_line`]).
//! 3. **The per-item timeout belongs to the item, not the run.** Every item's
//!    call carries its own `--max-time`, and the run's overall budget must
//!    *strictly exceed* the worst case (every item using its full per-item
//!    bound). An item with no per-item bound is exactly the defect: the call
//!    can hang, and only the run budget is left to catch it — or not.
//!    ([`SweepBudget`], [`validate_sweep_budget`]).
//! 4. **Alert on the transition, not the state.** A usually-red unit teaches
//!    its operator to ignore red. Alerts fire when the failure set changes
//!    (onset, new failures, recovery) or when the consecutive-failure streak
//!    crosses its threshold — and not on every red run.
//!    ([`FailureLedger::record_run`]).
//!
//! The module is pure: the caller runs the job (spawns the `curl` calls),
//! observes each item's outcome, and reports it here. The module never
//! spawns a process, never reads a clock, and never does I/O; it only
//! decides what the observed outcomes *mean* for the job's report, exit
//! status, and alert.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::Duration;

/// The reason recorded for an item the loop aborted before reaching.
///
/// Stable string so the report line and its consumers can match on it: a
/// failure with this reason is the loop's own abort, not the item's fault.
pub const NOT_ATTEMPTED: &str = "not attempted (loop aborted before this item)";

/// The outcome of one item's attempt in a single run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemOutcome {
    /// The item is registered and alive after this run — the run did the
    /// work.
    Registered,
    /// The item was already registered and still alive; the run did nothing
    /// for it. Distinct from [`Registered`]: "no work was needed" is not
    /// "the work succeeded", and invariant 2 keeps the two states separate.
    AlreadyAlive,
    /// The item failed this run. `reason` is the one-line transport/HTTP
    /// classification of the failure (the shape of
    /// `crate::service_timeout::CallFailure::log_line`), rendered into the
    /// job's report.
    Failed { reason: String },
}

/// One item's result from a run: which item, and what happened to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemResult {
    /// The item's declared name (a model name, an image name, …).
    pub item: String,
    /// What the run observed for this item.
    pub outcome: ItemOutcome,
}

/// An item that is not healthy after a run: its name and, if it was
/// attempted, why it failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedItem {
    pub item: String,
    /// The one-line failure classification, or [`NOT_ATTEMPTED`] for items
    /// the loop aborted before reaching.
    pub reason: String,
}

/// The fleet's state after a run — which items are registered and alive,
/// not what the run tried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SweepState {
    /// Every item is registered and alive, and the run registered at least
    /// one of them.
    AllHealthy {
        total: usize,
        /// Items the run itself registered (as opposed to finding alive).
        registered_now: usize,
    },
    /// Every item was already registered and still alive before the run; the
    /// run did no work. Exit 0 — but a *different* state from
    /// [`AllHealthy`]: a no-op success is not work done (invariant 2).
    AlreadyUpToDate { total: usize },
    /// Some items are healthy and some are not. Exit 1. Every failing item
    /// is named, in declared order.
    Partial {
        total: usize,
        failed: Vec<FailedItem>,
    },
    /// No item is healthy. Exit 1.
    AllFailed {
        total: usize,
        failed: Vec<FailedItem>,
    },
}

impl SweepState {
    /// The exit status the job must return for this state.
    ///
    /// Success states are `0`, failure states are `1` — and the failure
    /// states are distinct from each other and from the success states, so
    /// 3/4, 0/4, and "already registered and still alive" can never collapse
    /// onto one exit path (invariant 2).
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::AllHealthy { .. } | Self::AlreadyUpToDate { .. } => 0,
            Self::Partial { .. } | Self::AllFailed { .. } => 1,
        }
    }

    /// How many items the state covers.
    pub fn total(&self) -> usize {
        match self {
            Self::AllHealthy { total, .. }
            | Self::AlreadyUpToDate { total }
            | Self::Partial { total, .. }
            | Self::AllFailed { total, .. } => *total,
        }
    }

    /// How many items are registered and alive after the run.
    pub fn healthy(&self) -> usize {
        match self {
            Self::AllHealthy { total, .. } | Self::AlreadyUpToDate { total } => *total,
            Self::Partial { total, failed } | Self::AllFailed { total, failed } => {
                total - failed.len()
            }
        }
    }

    /// The failing items, in declared order; empty for the healthy states.
    pub fn failed_items(&self) -> &[FailedItem] {
        match self {
            Self::AllHealthy { .. } | Self::AlreadyUpToDate { .. } => &[],
            Self::Partial { failed, .. } | Self::AllFailed { failed, .. } => failed,
        }
    }

    /// The one-line report naming the state. 3/4, 0/4, and "already
    /// registered and still alive" render three different lines.
    pub fn summary_line(&self) -> String {
        match self {
            Self::AllHealthy {
                total,
                registered_now,
            } => format!(
                "sweep: {total}/{total} registered and alive ({registered_now} registered this run)"
            ),
            Self::AlreadyUpToDate { total } => {
                format!("sweep: {total}/{total} already registered and alive (no work done)")
            }
            Self::Partial { total, failed } => format!(
                "sweep: {}/{total} registered and alive; failed: {}",
                total - failed.len(),
                render_failures(failed)
            ),
            Self::AllFailed { total, failed } => {
                format!(
                    "sweep: 0/{total} registered and alive; failed: {}",
                    render_failures(failed)
                )
            }
        }
    }
}

/// Collect a run's per-item results into the fleet's [`SweepState`].
///
/// `declared` is the list of items the job said it would attempt (the
/// registry's models, the build's images); `results` is what the run
/// actually observed, in any order. The fold is total — the property that
/// makes invariant 1 checkable:
///
/// * every declared item appears in the state exactly once;
/// * a declared item with no result is a failed item with reason
///   [`NOT_ATTEMPTED`] — an absence is the loop aborting, and the report
///   says so instead of quietly shrinking the fleet;
/// * a result for an item that was never declared, or two results for one
///   item, are report defects and are rejected: the report covers exactly
///   the declared items.
///
/// The incident's shape, then, is a value: with eight declared items, item 4
/// timing out under `set -e`, and items 5–8 never attempted, the fold
/// returns a [`SweepState::Partial`] naming all four failures — not a
/// success over the three items that happened to run.
pub fn fold_item_results(
    declared: &[String],
    results: &[ItemResult],
) -> Result<SweepState, String> {
    if declared.is_empty() {
        return Err(
            "a sweep over zero declared items is a misconfiguration, not a no-op".to_string(),
        );
    }

    // Report defects: duplicates and undeclared items, both rejected before
    // any state is derived.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for result in results {
        if !seen.insert(result.item.as_str()) {
            return Err(format!(
                "duplicate results for item '{}' — one result per item per run",
                result.item
            ));
        }
    }
    for result in results {
        if !declared.iter().any(|item| item == &result.item) {
            return Err(format!(
                "result for undeclared item '{}': the report covers exactly the declared items",
                result.item
            ));
        }
    }

    let mut failed: Vec<FailedItem> = Vec::new();
    let mut registered_now = 0usize;
    for item in declared {
        match results.iter().find(|result| &result.item == item) {
            Some(ItemResult {
                outcome: ItemOutcome::Registered,
                ..
            }) => registered_now += 1,
            Some(ItemResult {
                outcome: ItemOutcome::AlreadyAlive,
                ..
            }) => {}
            Some(ItemResult {
                outcome: ItemOutcome::Failed { reason },
                ..
            }) => failed.push(FailedItem {
                item: item.clone(),
                reason: reason.clone(),
            }),
            None => failed.push(FailedItem {
                item: item.clone(),
                reason: NOT_ATTEMPTED.to_string(),
            }),
        }
    }

    let total = declared.len();
    if failed.is_empty() {
        if registered_now == 0 {
            Ok(SweepState::AlreadyUpToDate { total })
        } else {
            Ok(SweepState::AllHealthy {
                total,
                registered_now,
            })
        }
    } else if failed.len() == total {
        Ok(SweepState::AllFailed { total, failed })
    } else {
        Ok(SweepState::Partial { total, failed })
    }
}

fn render_failures(failed: &[FailedItem]) -> String {
    failed
        .iter()
        .map(|f| format!("{} ({})", f.item, f.reason))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The time bounds a sweep is allowed to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SweepBudget {
    /// The per-item bound — the `--max-time` of each item's call.
    pub per_item: Duration,
    /// The overall bound for the whole run — the unit's `TimeoutStartSec`
    /// or the job's own watchdog.
    pub overall: Duration,
}

/// A `SweepBudget` that cannot hold the run it is meant to bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BudgetViolation {
    pub per_item: Duration,
    pub overall: Duration,
    pub items: usize,
    /// Why the budget fails. A stable string so the job's log line — and a
    /// test — can match on the exact misconfiguration.
    pub reason: &'static str,
}

impl BudgetViolation {
    /// The one-line report of the misconfiguration, naming every number.
    pub fn line(&self) -> String {
        format!(
            "budget violation: per-item {:?} over {} items, overall {:?} — {}",
            self.per_item, self.items, self.overall, self.reason
        )
    }
}

/// Check that a sweep's time bounds can actually hold the run.
///
/// The invariants (invariant 3):
///
/// * every item needs a per-item bound — `per_item` must be non-zero. An
///   item whose call has no `--max-time` is exactly the incident: it can
///   hang, and only the run budget is left to catch it;
/// * the run needs an overall bound — `overall` must be non-zero;
/// * the overall bound must **strictly exceed** the worst case (every item
///   using its full per-item bound, sequentially). Equality is not enough:
///   a run that reaches its budget exactly is a run that hit its own
///   deadline, the same "client bound equal to the server bound" race that
///   #3637 forbids.
///
/// Zero declared items is rejected too: a sweep over nothing is a
/// misconfiguration, not a fast success (mirroring [`fold_item_results`]).
pub fn validate_sweep_budget(budget: &SweepBudget, items: usize) -> Result<(), BudgetViolation> {
    if items == 0 {
        return Err(BudgetViolation {
            per_item: budget.per_item,
            overall: budget.overall,
            items,
            reason: "a sweep over zero items is a misconfiguration, not a no-op",
        });
    }
    if budget.per_item.is_zero() {
        return Err(BudgetViolation {
            per_item: budget.per_item,
            overall: budget.overall,
            items,
            reason:
                "an item without a per-item bound can hang; only the run budget is left to catch it",
        });
    }
    if budget.overall.is_zero() {
        return Err(BudgetViolation {
            per_item: budget.per_item,
            overall: budget.overall,
            items,
            reason: "a run without an overall bound can run forever",
        });
    }
    // Worst case: every item uses its full per-item bound, sequentially.
    let worst_case = items
        .try_into()
        .ok()
        .and_then(|n: u32| budget.per_item.checked_mul(n))
        .ok_or(BudgetViolation {
            per_item: budget.per_item,
            overall: budget.overall,
            items,
            reason: "per-item bound times item count overflows; the budget is unbounded",
        })?;
    if budget.overall <= worst_case {
        return Err(BudgetViolation {
            per_item: budget.per_item,
            overall: budget.overall,
            items,
            reason: "the overall bound must strictly exceed the worst case (every item at its full per-item bound)",
        });
    }
    Ok(())
}

/// The change in the failing-item set between two consecutive runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transition", rename_all = "snake_case")]
pub enum FailureTransition {
    /// Both runs healthy.
    Steady,
    /// The previous run had failures and this one has none.
    Recovered { recovered: Vec<String> },
    /// The previous run was healthy and this one has failures: the moment an
    /// item first fails.
    Onset { failed: Vec<String> },
    /// The failure set changed but is neither onset nor recovery: some
    /// items newly failed, some recovered.
    Changed {
        new: Vec<String>,
        recovered: Vec<String>,
    },
    /// The same non-empty failure set again: no transition, only
    /// persistence — the usually-red case that must not re-alert.
    Persisted { failed: Vec<String> },
}

/// Diff two consecutive runs' failing-item sets into a transition.
///
/// Items are compared as a set (order and duplicates in the inputs do not
/// matter), and every output list is sorted and de-duplicated so the
/// rendered alert line is deterministic.
pub fn diff_failure_sets(prev: &[String], curr: &[String]) -> FailureTransition {
    let prev_set: BTreeSet<&str> = prev.iter().map(String::as_str).collect();
    let curr_set: BTreeSet<&str> = curr.iter().map(String::as_str).collect();
    match (prev_set.is_empty(), curr_set.is_empty()) {
        (true, true) => FailureTransition::Steady,
        (true, false) => FailureTransition::Onset {
            failed: sorted(&curr_set),
        },
        (false, true) => FailureTransition::Recovered {
            recovered: sorted(&prev_set),
        },
        (false, false) => {
            let new = sorted(
                &curr_set
                    .difference(&prev_set)
                    .copied()
                    .collect::<BTreeSet<&str>>(),
            );
            let recovered = sorted(
                &prev_set
                    .difference(&curr_set)
                    .copied()
                    .collect::<BTreeSet<&str>>(),
            );
            if new.is_empty() && recovered.is_empty() {
                FailureTransition::Persisted {
                    failed: sorted(&curr_set),
                }
            } else {
                FailureTransition::Changed { new, recovered }
            }
        }
    }
}

fn sorted(set: &BTreeSet<&str>) -> Vec<String> {
    set.iter().map(|s| s.to_string()).collect()
}

/// The job's memory across runs: the consecutive-failure streak and the
/// previous run's failing items.
///
/// This is what "N consecutive failures" counts from. The caller persists
/// it between runs (serde) — it is the unit's state file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureLedger {
    /// The consecutive-failure count at which a *persisted* failure set
    /// escalates to exactly one alert. Always >= 1 (see
    /// [`FailureLedger::with_threshold`]).
    threshold: u32,
    /// Consecutive failing runs; 0 while the last observed run was healthy.
    streak: u32,
    /// The items that were failing on the last observed run.
    last_failed: BTreeSet<String>,
    /// Whether any run has been observed: the first observed run is a
    /// transition from *unknown*, not from healthy.
    observed: bool,
}

impl Default for FailureLedger {
    fn default() -> Self {
        Self {
            threshold: 1,
            streak: 0,
            last_failed: BTreeSet::new(),
            observed: false,
        }
    }
}

impl FailureLedger {
    /// Create a ledger that escalates a persisted failure set after
    /// `threshold` consecutive failing runs.
    ///
    /// `threshold` must be >= 1: a threshold of 0 would alert on every red
    /// run, which is alerting on *state*, not transition — the defect this
    /// module exists to remove.
    pub fn with_threshold(threshold: u32) -> Result<Self, String> {
        if threshold == 0 {
            return Err(
                "alert threshold must be >= 1: a threshold of 0 alerts on state, not transition"
                    .to_string(),
            );
        }
        Ok(Self {
            threshold,
            ..Self::default()
        })
    }

    /// The configured escalation threshold.
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    /// The current consecutive-failure streak.
    pub fn streak(&self) -> u32 {
        self.streak
    }

    /// The items failing on the last observed run.
    pub fn last_failed(&self) -> Vec<String> {
        self.last_failed.iter().cloned().collect()
    }

    /// Observe a run's failing items and decide whether to alert.
    ///
    /// The alert policy (invariant 4) — alert on the transition, not the
    /// state:
    ///
    /// * the first observed run with failures fires (a transition from
    ///   unknown); the first observed healthy run does not;
    /// * an onset (healthy → failing) fires, naming every new failure;
    /// * a recovery (failing → healthy) fires, naming every recovered item;
    /// * a changed set (some new, some recovered) fires, naming both;
    /// * a *persisted* set fires exactly once, when the streak crosses the
    ///   threshold — "still failing after N consecutive runs" — and is quiet
    ///   on every other red run. This is what a usually-red unit was missing:
    ///   it stops re-alerting on the unchanged state, while a change is
    ///   always loud.
    pub fn record_run(&mut self, failed: &[String]) -> AlertDecision {
        let failed_sorted: Vec<String> = {
            let set: BTreeSet<&str> = failed.iter().map(String::as_str).collect();
            sorted(&set)
        };
        let failing = !failed_sorted.is_empty();
        let new_set: BTreeSet<String> = failed_sorted.iter().cloned().collect();

        if !self.observed {
            self.observed = true;
            self.streak = u32::from(failing);
            self.last_failed = new_set;
            return if failing {
                AlertDecision::Fire {
                    line: format!(
                        "alert: sweep failing on first observed run: {}",
                        names(&failed_sorted)
                    ),
                }
            } else {
                AlertDecision::Quiet {
                    reason: "steady on first observation",
                }
            };
        }

        let transition = diff_failure_sets(
            &self.last_failed.iter().cloned().collect::<Vec<_>>(),
            &failed_sorted,
        );
        self.streak = if failing {
            self.streak.saturating_add(1)
        } else {
            0
        };
        self.last_failed = new_set;

        match transition {
            FailureTransition::Steady => AlertDecision::Quiet { reason: "steady" },
            FailureTransition::Recovered { recovered } => AlertDecision::Fire {
                line: format!("alert: sweep recovered: {}", names(&recovered)),
            },
            FailureTransition::Onset { failed } => AlertDecision::Fire {
                line: format!("alert: sweep newly failing: {}", names(&failed)),
            },
            FailureTransition::Changed { new, recovered } => AlertDecision::Fire {
                line: format!(
                    "alert: sweep failure set changed: new {}; recovered {}",
                    names(&new),
                    names(&recovered)
                ),
            },
            FailureTransition::Persisted { failed } => {
                if self.streak == self.threshold {
                    AlertDecision::Fire {
                        line: format!(
                            "alert: sweep still failing after {} consecutive runs: {}",
                            self.threshold,
                            names(&failed)
                        ),
                    }
                } else {
                    AlertDecision::Quiet {
                        reason: "persisted failure, already reported",
                    }
                }
            }
        }
    }
}

/// Whether a run's transition warrants an alert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum AlertDecision {
    /// No alert. The state is steady, or a persistence that was already
    /// reported — the usually-red case that must not re-alert on every run.
    Quiet { reason: &'static str },
    /// Alert. `line` is the one-line message for the operator.
    Fire { line: String },
}

impl AlertDecision {
    /// Whether this decision sends an alert.
    pub fn fires(&self) -> bool {
        matches!(self, Self::Fire { .. })
    }
}

fn names(items: &[String]) -> String {
    items.join(", ")
}
