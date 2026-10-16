//! `autospec benchmark delivery --results <PATH> [--json]`
//!
//! Prints the delivery benchmark (spec section 7): per-task p50/p95 against
//! the recorded standalone baseline, and A1-A9 as `pass`, `fail` or `not-run`.
//! The input is JSONL, one `RunResult` or `{"scenario": "A3", "passed": true}`
//! per line; a malformed line stops the command with exit 2 naming its line
//! number. The aggregation is `autospec_core::delivery_benchmark`.
//!
//! `autospec benchmark validate-matrix <matrix.json> [--json]`
//!
//! Validates a Qwen3.8 benchmark matrix against the provider-neutral
//! contract in `autospec_core::benchmark_matrix` (issue #3328): every row
//! identifies quantization, runtime, node, and profile; speculative rows
//! record draft and accepted-draft token counts; a candidate wins only when
//! `success: true`; and the final report compares the winner's median
//! successful issue time to the baseline.

use std::path::Path;

use autospec_core::benchmark_matrix;
use autospec_core::benchmark_matrix::CellOutcome;
use autospec_core::delivery_benchmark::{summarize, ArmSummary, RunResult, TaskSummary, SCENARIOS};

const USAGE: &str = "usage: autospec benchmark validate-matrix <matrix.json> [--json]\n       autospec benchmark delivery --results <PATH> [--json]";
const DELIVERY_USAGE: &str = "usage: autospec benchmark delivery --results <PATH> [--json]";

pub fn run(args: &[String]) -> Result<(), String> {
    let Some((first, rest)) = args.split_first() else {
        return Err(USAGE.to_string());
    };
    match first.as_str() {
        "validate-matrix" => validate_matrix(rest),
        "delivery" => delivery(rest),
        _ => Err(USAGE.to_string()),
    }
}

/// One accepted line of a delivery results file.
enum Row {
    Run(RunResult),
    Scenario { id: &'static str, passed: bool },
}

fn parse_row(line: &str) -> Result<Row, String> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
    if value.get("scenario").is_none() {
        return serde_json::from_value(value)
            .map(Row::Run)
            .map_err(|error| error.to_string());
    }
    let scenario = value
        .get("scenario")
        .and_then(serde_json::Value::as_str)
        .ok_or("scenario must be a string")?;
    let id = SCENARIOS
        .iter()
        .find(|known| known.id == scenario)
        .map(|known| known.id)
        .ok_or_else(|| format!("unknown scenario {scenario:?} (expected A1..A9)"))?;
    let passed = value
        .get("passed")
        .and_then(serde_json::Value::as_bool)
        .ok_or("scenario rows need a boolean `passed`")?;
    Ok(Row::Scenario { id, passed })
}

/// `pass` when every row for the scenario passed, `fail` when any failed,
/// `not-run` when there is none.
fn scenario_status(id: &str, rows: &[Row]) -> &'static str {
    let mut seen = false;
    for row in rows {
        if let Row::Scenario { id: row_id, passed } = row {
            if *row_id == id {
                if !passed {
                    return "fail";
                }
                seen = true;
            }
        }
    }
    if seen {
        "pass"
    } else {
        "not-run"
    }
}

fn secs(value: Option<f64>) -> String {
    value.map_or_else(|| "unmeasured".to_string(), |secs| format!("{secs}"))
}

fn pair<T: std::fmt::Display>(baseline: Option<T>, pi: Option<T>) -> String {
    let show = |value: Option<T>| value.map_or_else(|| "unmeasured".to_string(), |v| v.to_string());
    format!("{} / {}", show(baseline), show(pi))
}

fn rate(arm: &ArmSummary) -> Option<String> {
    arm.success_rate
        .map(|rate| format!("{}%", (rate * 100.0).round()))
}

/// One markdown row per task, in the columns of the final report's
/// `## Benchmarks` table.
fn render_rows(summaries: &[TaskSummary]) -> String {
    let mut out = String::new();
    for summary in summaries {
        let (baseline, pi) = (&summary.baseline, &summary.pi_autospec);
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            summary.task.as_str(),
            secs(baseline.p50_secs),
            secs(baseline.p95_secs),
            secs(pi.p50_secs),
            secs(pi.p95_secs),
            pair(baseline.median_tokens, pi.median_tokens),
            pair(baseline.median_retries, pi.median_retries),
            pair(baseline.median_disk_bytes, pi.median_disk_bytes),
            pair(rate(baseline), rate(pi)),
        ));
    }
    out
}

fn render_scenarios(rows: &[Row]) -> String {
    SCENARIOS
        .iter()
        .map(|scenario| format!("{} {}\n", scenario.id, scenario_status(scenario.id, rows)))
        .collect()
}

fn delivery(args: &[String]) -> Result<(), String> {
    let mut results: Option<&str> = None;
    let mut json = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--results" => {
                results = Some(iter.next().ok_or(DELIVERY_USAGE)?.as_str());
            }
            _ => return Err(DELIVERY_USAGE.to_string()),
        }
    }
    let path = results.ok_or(DELIVERY_USAGE)?;
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read results file {path}: {error}"))?;

    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row =
            parse_row(line).map_err(|error| format!("{path}: line {}: {error}", index + 1))?;
        rows.push(row);
    }
    let runs: Vec<RunResult> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Run(run) => Some(run.clone()),
            Row::Scenario { .. } => None,
        })
        .collect();
    let summaries = summarize(&runs);

    if json {
        let scenarios: Vec<serde_json::Value> = SCENARIOS
            .iter()
            .map(|scenario| {
                serde_json::json!({
                    "id": scenario.id,
                    "status": scenario_status(scenario.id, &rows),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({ "tasks": summaries, "scenarios": scenarios })
        );
    } else {
        print!("{}\n{}", render_rows(&summaries), render_scenarios(&rows));
    }
    Ok(())
}

fn validate_matrix(args: &[String]) -> Result<(), String> {
    let mut path: Option<&str> = None;
    let mut json = false;
    for arg in args {
        if arg == "--json" {
            json = true;
        } else if arg.starts_with('-') {
            return Err(USAGE.to_string());
        } else if path.is_none() {
            path = Some(arg.as_str());
        } else {
            return Err(USAGE.to_string());
        }
    }
    let Some(path) = path else {
        return Err(USAGE.to_string());
    };

    let matrix = benchmark_matrix::load_matrix(Path::new(path))?;
    let errors = benchmark_matrix::validate(&matrix);
    let cells = benchmark_matrix::plan_cells(&matrix);
    let report = benchmark_matrix::report(&matrix);
    if json {
        println!("{}", render_json(&errors, &cells, &report));
    } else {
        println!("{}", render_text(&errors, &cells, &report));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("matrix has {} validation error(s)", errors.len()))
    }
}

fn render_text(
    errors: &[benchmark_matrix::RowError],
    cells: &[CellOutcome],
    report: &benchmark_matrix::MatrixReport,
) -> String {
    let mut out = String::new();
    for error in errors {
        out.push_str(&format!("error: {}\n", error.render()));
    }
    for cell in cells {
        out.push_str(&format!("cell: {}\n", cell.render()));
    }
    out.push_str(&format!(
        "report: winner={winner} winner_median={winner_median} baseline_median={baseline_median} delta={delta} faster={faster}\n",
        winner = report
            .winner
            .clone()
            .unwrap_or_else(|| "none".to_string()),
        winner_median = report
            .winner_median_success_seconds
            .map(|value| format!("{value}"))
            .unwrap_or_else(|| "none".to_string()),
        baseline_median = report.baseline_median_success_seconds,
        delta = report
            .delta_vs_baseline_seconds
            .map(|value| format!("{value}"))
            .unwrap_or_else(|| "none".to_string()),
        faster = report
            .faster_than_baseline
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
    ));
    out
}

fn render_json(
    errors: &[benchmark_matrix::RowError],
    cells: &[CellOutcome],
    report: &benchmark_matrix::MatrixReport,
) -> serde_json::Value {
    serde_json::json!({
        "valid": errors.is_empty(),
        "errors": errors.iter().map(|error| serde_json::json!({
            "candidate_id": error.candidate_id,
            "reason": error.reason,
        })).collect::<Vec<_>>(),
        "cells": cells.iter().map(|cell| serde_json::json!({
            "candidate_id": cell_candidate_id(cell),
            "outcome": cell_outcome(cell),
            "reason": cell_reason(cell),
        })).collect::<Vec<_>>(),
        "report": {
            "winner": report.winner,
            "winner_median_success_seconds": report.winner_median_success_seconds,
            "baseline_median_success_seconds": report.baseline_median_success_seconds,
            "delta_vs_baseline_seconds": report.delta_vs_baseline_seconds,
            "faster_than_baseline": report.faster_than_baseline,
            "baseline_gate_passed": report.baseline_gate_passed,
        },
    })
}

fn cell_candidate_id(cell: &CellOutcome) -> &str {
    match cell {
        CellOutcome::Run { candidate_id } | CellOutcome::Skip { candidate_id, .. } => candidate_id,
    }
}

fn cell_outcome(cell: &CellOutcome) -> &'static str {
    match cell {
        CellOutcome::Run { .. } => "run",
        CellOutcome::Skip { .. } => "skip",
    }
}

fn cell_reason(cell: &CellOutcome) -> Option<&str> {
    match cell {
        CellOutcome::Run { .. } => None,
        CellOutcome::Skip { reason, .. } => Some(reason.as_str()),
    }
}
