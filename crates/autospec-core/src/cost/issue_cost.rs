//! Per-issue cost rollup with the retry signals beside the hours (issue #3983).
//!
//! Cumulative GPU-hours and total zero-output runs are reporting metrics: both
//! keep climbing for an issue whose later runs succeeded, so neither may gate a
//! dispatch. What the retry decision reads is the outcome of the most recent
//! run and the length of the trailing zero-output streak
//! ([`crate::coordination::retry_decision`]). This module keeps the two sets of
//! numbers visually adjacent so a reader never infers a hold from a total.

use std::collections::BTreeMap;

use serde::Serialize;

use super::record::RunRecord;
use super::report::round2;
use crate::coordination::{
    retry_decision, total_zero_output_runs, trailing_zero_output_streak, LatestRunOutcome,
    RetryDecision, RunOutcome,
};

/// Rows of the per-issue table rendered in text output; the JSON payload
/// carries every issue.
pub const TEXT_ISSUE_ROWS: usize = 20;

/// One issue's cumulative cost, shown next to the two numbers the retry
/// decision actually reads (#3983).
///
/// The hours are a reporting metric. Whether the issue is dispatchable is read
/// off `latest_outcome` and `trailing_zero_output_streak` — the outcome of the
/// most recent run and the zero-output runs that end there — never off
/// `gpu_hours` or `zero_output_runs`, both of which keep growing for an issue
/// whose later runs succeeded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IssueCost {
    /// Issue key: the run directory's `issue-<number>` prefix, or the run name
    /// itself when the directory carries no issue number.
    pub issue: String,
    /// Costed runs in scope for this issue.
    pub runs: u64,
    pub gpu_hours: f64,
    /// Terminal status of the most recent costed run, when it recorded one.
    pub latest_status: Option<String>,
    /// `produced` | `zero-output` — the most recent run's outcome.
    pub latest_outcome: &'static str,
    /// Consecutive zero-output runs ending at the most recent run. This is the
    /// number the retry decision reads.
    pub trailing_zero_output_streak: usize,
    /// Every zero-output run in scope. Reporting only; never a gate.
    pub zero_output_runs: usize,
    /// `redispatch` | `review`, as decided from the two fields above.
    pub retry_route: &'static str,
    /// Shorthand for `retry_route == "redispatch"`.
    pub dispatchable: bool,
}

/// The issue key for a run directory name: `issue-3192-attempt-3` and
/// `issue-3192` belong to the same issue; a name with no `issue-<number>`
/// prefix is its own key.
pub(crate) fn issue_key(run_name: &str) -> String {
    let Some(rest) = run_name.strip_prefix("issue-") else {
        return run_name.to_string();
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return run_name.to_string();
    }
    format!("issue-{digits}")
}

/// Whether a costed run left a verifiable artifact behind.
///
/// The fleet's explicit verdict wins: `NO-OUTPUT` is recorded for a run that
/// produced nothing reviewable, whatever its file counter says. Otherwise a run
/// that changed no files left nothing behind to review, and a run that changed
/// files did.
pub(crate) fn run_outcome(record: &RunRecord) -> RunOutcome {
    if record
        .status
        .as_deref()
        .is_some_and(|status| status.eq_ignore_ascii_case("NO-OUTPUT"))
    {
        return RunOutcome::ZeroOutput;
    }
    if record.changed_files > 0 {
        RunOutcome::ProducedOutput
    } else {
        RunOutcome::ZeroOutput
    }
}

/// Rank issues by cumulative GPU-hours and put the retry signals beside them.
///
/// Within an issue, runs are ordered oldest first by their finish (or start)
/// stamp — an unstamped run cannot be placed in time and sorts first — so the
/// last entry is the most recent run, and the streak counted back from it is
/// the trailing one. Ties keep scan (directory-name) order.
pub(crate) fn build_issue_costs(records: &[RunRecord]) -> Vec<IssueCost> {
    let mut groups: BTreeMap<String, Vec<&RunRecord>> = BTreeMap::new();
    for record in records.iter().filter(|record| record.is_costed()) {
        groups
            .entry(issue_key(&record.issue))
            .or_default()
            .push(record);
    }
    let mut costs: Vec<IssueCost> = groups
        .into_iter()
        .map(|(issue, mut group)| {
            group.sort_by(|a, b| {
                a.window_stamp()
                    .unwrap_or(i64::MIN)
                    .cmp(&b.window_stamp().unwrap_or(i64::MIN))
            });
            let outcomes: Vec<RunOutcome> =
                group.iter().map(|record| run_outcome(record)).collect();
            let gpu_hours: f64 = group
                .iter()
                .map(|record| record.gpu_hours().unwrap_or(0.0))
                .sum();
            let trailing_zero_output_streak = trailing_zero_output_streak(&outcomes);
            let latest = outcomes.last().copied();
            let latest_outcome = match latest {
                None => LatestRunOutcome::NoRuns,
                Some(RunOutcome::ProducedOutput) => LatestRunOutcome::ProducedOutput,
                Some(RunOutcome::ZeroOutput) => LatestRunOutcome::ZeroOutput,
            };
            // The decision reads the latest outcome and the trailing streak.
            // Nothing here consults `gpu_hours` or the total run count.
            let decision = retry_decision(latest_outcome, trailing_zero_output_streak);
            IssueCost {
                issue,
                runs: group.len() as u64,
                gpu_hours: round2(gpu_hours),
                latest_status: group
                    .last()
                    .and_then(|record| record.status.clone())
                    .filter(|status| !status.is_empty()),
                latest_outcome: decision.latest_outcome.as_str(),
                trailing_zero_output_streak: decision.trailing_zero_output_streak,
                zero_output_runs: total_zero_output_runs(&outcomes),
                retry_route: retry_route_label(&decision),
                dispatchable: decision.dispatchable(),
            }
        })
        .collect();
    // Largest spend first, so the table reads as a ranking; the issue key
    // breaks ties so two scans of the same tree render identically.
    costs.sort_by(|a, b| {
        b.gpu_hours
            .total_cmp(&a.gpu_hours)
            .then_with(|| a.issue.cmp(&b.issue))
    });
    costs
}

fn retry_route_label(decision: &RetryDecision) -> &'static str {
    if decision.dispatchable() {
        "redispatch"
    } else {
        "review"
    }
}
/// The per-issue ranking, with the retry signals beside the hours so a reader
/// never has to infer dispatchability from a cumulative total (#3983).
pub(crate) fn issue_table(by_issue: &[IssueCost]) -> String {
    if by_issue.is_empty() {
        return String::new();
    }
    let width = by_issue
        .iter()
        .map(|row| row.issue.len())
        .max()
        .unwrap_or(0)
        .max("issue".len());
    let mut out = format!(
        "  issues by cumulative GPU-hours (retry reads latest outcome and trailing streak, never the hours):\n"
    );
    out.push_str(&format!(
        "    {:width$}   runs  GPU-hours  {:<17}  {:<11}  {:>8}  {:>7}  {:>10}\n",
        "issue",
        "latest status",
        "latest",
        "trailing",
        "total-zo",
        "retry",
        width = width
    ));
    for row in by_issue.iter().take(TEXT_ISSUE_ROWS) {
        out.push_str(&format!(
            "    {:width$} {:>6}  {:>9.1}  {:<17}  {:<11}  {:>8}  {:>7}  {:>10}\n",
            row.issue,
            row.runs,
            row.gpu_hours,
            row.latest_status.as_deref().unwrap_or("-"),
            row.latest_outcome,
            row.trailing_zero_output_streak,
            row.zero_output_runs,
            row.retry_route,
            width = width
        ));
    }
    if by_issue.len() > TEXT_ISSUE_ROWS {
        out.push_str(&format!(
            "    … {} more issue(s) ({} in --json)\n",
            by_issue.len() - TEXT_ISSUE_ROWS,
            by_issue.len()
        ));
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    fn record(issue: &str, status: Option<&str>, changed_files: u64) -> RunRecord {
        let mut record = RunRecord::new(issue.to_string());
        record.status = status.map(str::to_string);
        record.changed_files = changed_files;
        // Only records with a terminal status and an `agent_secs` value are
        // costed, so the rollup sees them at all.
        record.agent_secs = Some(600);
        record
    }

    #[test]
    fn run_directories_of_one_issue_share_a_key() {
        assert_eq!(issue_key("issue-3192"), "issue-3192");
        assert_eq!(issue_key("issue-3192-attempt-3"), "issue-3192");
        assert_eq!(issue_key("issue-3192-retry-2026-09-01"), "issue-3192");
        // A name with no `issue-<number>` prefix is its own key.
        assert_eq!(issue_key("manual-sweep"), "manual-sweep");
        assert_eq!(issue_key("issue-abc"), "issue-abc");
    }

    #[test]
    fn an_explicit_no_output_verdict_beats_the_file_counter() {
        assert_eq!(
            run_outcome(&record("issue-1", None, 3)),
            RunOutcome::ProducedOutput
        );
        assert_eq!(
            run_outcome(&record("issue-1", Some("VERIFIED"), 0)),
            RunOutcome::ZeroOutput
        );
        assert_eq!(
            run_outcome(&record("issue-1", Some("NO-OUTPUT"), 4)),
            RunOutcome::ZeroOutput
        );
        assert_eq!(
            run_outcome(&record("issue-1", Some("no-output"), 4)),
            RunOutcome::ZeroOutput
        );
    }

    #[test]
    fn early_zero_output_runs_do_not_hold_an_issue_that_later_produced() {
        // Two early zero-output runs and a recent run that produced files:
        // 3 zero-output hours in total, 0 in the trailing chain.
        let records = [
            record("issue-7-attempt-1", Some("TIMEOUT"), 0),
            record("issue-7-attempt-2", Some("NO-OUTPUT"), 0),
            record("issue-7-attempt-3", Some("VERIFIED"), 6),
        ];
        let costs = build_issue_costs(&records);
        assert_eq!(costs.len(), 1);
        let cost = &costs[0];
        assert_eq!(cost.issue, "issue-7");
        assert_eq!(cost.zero_output_runs, 2);
        assert_eq!(cost.trailing_zero_output_streak, 0);
        assert_eq!(cost.latest_outcome, "produced");
        assert!(
            cost.dispatchable,
            "a producing latest run stays dispatchable"
        );
    }

    #[test]
    fn a_trailing_pair_holds_and_the_table_says_so() {
        let records = [
            record("issue-8", Some("VERIFIED"), 2),
            record("issue-8-retry", Some("NO-OUTPUT"), 0),
            record("issue-8-retry-2", Some("NO-OUTPUT"), 0),
        ];
        let costs = build_issue_costs(&records);
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].trailing_zero_output_streak, 2);
        assert_eq!(costs[0].retry_route, "review");
        assert!(!costs[0].dispatchable);

        let table = issue_table(&costs);
        assert!(table.contains("issues by cumulative GPU-hours"), "{table}");
        let row = table
            .lines()
            .find(|line| line.starts_with("    issue-8"))
            .expect("row");
        assert!(row.contains("review"), "{row}");
    }
}
