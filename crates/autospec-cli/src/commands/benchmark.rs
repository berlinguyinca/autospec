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

const USAGE: &str = "usage: autospec benchmark validate-matrix <matrix.json> [--json]";

pub fn run(args: &[String]) -> Result<(), String> {
    let Some((first, rest)) = args.split_first() else {
        return Err(USAGE.to_string());
    };
    if first != "validate-matrix" {
        return Err(USAGE.to_string());
    }
    validate_matrix(rest)
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
