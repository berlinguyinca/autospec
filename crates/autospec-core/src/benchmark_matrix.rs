//! Provider-neutral benchmark matrix contract (issue #3328).
//!
//! A matrix compares supported Qwen3.8 quantizations, inference runtimes,
//! nodes, and speculative decoding by successful coding-task time. The
//! contract is provider-neutral: it validates matrix rows, gates cells to the
//! supported local set (preserving skip reasons), and reports the winner's
//! median successful issue time against the recorded baseline.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

/// Lowest supported quantization class (Q3).
pub const QUANTIZATION_MIN: u8 = 3;
/// Highest supported quantization class (Q8).
pub const QUANTIZATION_MAX: u8 = 8;

/// Inference runtimes the benchmark gate supports.
pub const SUPPORTED_RUNTIMES: &[&str] = &["mlx", "llama.cpp", "vllm", "sglang"];

/// The baseline every candidate's successful time is compared against.
#[derive(Debug, Clone, Deserialize)]
pub struct Baseline {
    pub quantization: String,
    pub runtime: String,
    pub node: String,
    pub profile: String,
    pub median_success_seconds: f64,
}

/// One benchmark candidate row.
#[derive(Debug, Clone, Deserialize)]
pub struct CandidateRow {
    pub candidate_id: String,
    pub quantization: String,
    pub runtime: String,
    pub node: String,
    pub profile: String,
    pub success: bool,
    #[serde(default)]
    pub speculative: bool,
    #[serde(default)]
    pub success_seconds: Option<f64>,
    #[serde(default)]
    pub draft_tokens: Option<u64>,
    #[serde(default)]
    pub accepted_draft_tokens: Option<u64>,
    #[serde(default)]
    pub quality: Option<f64>,
    #[serde(default)]
    pub cache_hit_ratio: Option<f64>,
    #[serde(default)]
    pub memory_gb: Option<f64>,
    #[serde(default)]
    pub gpu_utilization: Option<f64>,
    #[serde(default)]
    pub energy_wh: Option<f64>,
}

/// A decoded benchmark matrix.
#[derive(Debug, Clone, Deserialize)]
pub struct BenchmarkMatrix {
    pub baseline: Baseline,
    #[serde(default)]
    pub rows: Vec<CandidateRow>,
    #[serde(default)]
    pub supported_nodes: Option<Vec<String>>,
}

/// A validation finding tied to one candidate row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowError {
    pub candidate_id: String,
    pub reason: String,
}

impl RowError {
    pub fn new(candidate_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            candidate_id: candidate_id.into(),
            reason: reason.into(),
        }
    }

    pub fn render(&self) -> String {
        format!("{}: {}", self.candidate_id, self.reason)
    }
}

/// Whether one cell is executed or skipped, with the skip reason preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellOutcome {
    Run {
        candidate_id: String,
    },
    Skip {
        candidate_id: String,
        reason: String,
    },
}

impl CellOutcome {
    pub fn render(&self) -> String {
        match self {
            CellOutcome::Run { candidate_id } => format!("run: {candidate_id}"),
            CellOutcome::Skip {
                candidate_id,
                reason,
            } => format!("skip: {candidate_id} ({reason})"),
        }
    }
}

/// The final report: the winner's median successful issue time compared to
/// the baseline median.
#[derive(Debug, Clone, PartialEq)]
pub struct MatrixReport {
    pub winner: Option<String>,
    pub winner_median_success_seconds: Option<f64>,
    pub baseline_median_success_seconds: f64,
    pub delta_vs_baseline_seconds: Option<f64>,
    pub faster_than_baseline: Option<bool>,
    pub baseline_gate_passed: bool,
}

/// Load and decode a matrix file.
pub fn load_matrix(path: &Path) -> Result<BenchmarkMatrix, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))
}

/// Validate the matrix against the acceptance contract.
pub fn validate(matrix: &BenchmarkMatrix) -> Vec<RowError> {
    let mut errors = Vec::new();
    for row in &matrix.rows {
        for (field, value) in [
            ("quantization", &row.quantization),
            ("runtime", &row.runtime),
            ("node", &row.node),
            ("profile", &row.profile),
        ] {
            if value.trim().is_empty() {
                errors.push(RowError::new(
                    row.candidate_id.clone(),
                    format!("{field} must be identified"),
                ));
            }
        }
        if row.speculative && (row.draft_tokens.is_none() || row.accepted_draft_tokens.is_none()) {
            errors.push(RowError::new(
                row.candidate_id.clone(),
                "speculative row must record draft_tokens and accepted_draft_tokens",
            ));
        }
        if row.success && row.success_seconds.is_none() {
            errors.push(RowError::new(
                row.candidate_id.clone(),
                "successful row must record success_seconds",
            ));
        }
    }
    errors
}

/// The quantization class encoded by a name like `Q4_K_M` or `q8_0`.
pub fn quantization_class(name: &str) -> Option<u8> {
    let bytes = name.trim().as_bytes();
    if !matches!(bytes.first(), Some(b'Q') | Some(b'q')) {
        return None;
    }
    let digit = *bytes.get(1)?;
    digit.is_ascii_digit().then_some(digit - b'0')
}

/// True when the quantization class is within the supported Q3-Q8 range.
pub fn quantization_supported(name: &str) -> bool {
    quantization_class(name)
        .is_some_and(|class| (QUANTIZATION_MIN..=QUANTIZATION_MAX).contains(&class))
}

/// The baseline gate: speculative cells run only after it passes.
pub fn baseline_gate_passed(matrix: &BenchmarkMatrix) -> bool {
    matrix.baseline.median_success_seconds.is_finite()
        && matrix.baseline.median_success_seconds > 0.0
}

/// Gate every row to the supported local cell set, preserving skip reasons.
pub fn plan_cells(matrix: &BenchmarkMatrix) -> Vec<CellOutcome> {
    let gate = baseline_gate_passed(matrix);
    matrix
        .rows
        .iter()
        .map(|row| match cell_skip_reason(matrix, row, gate) {
            Some(reason) => CellOutcome::Skip {
                candidate_id: row.candidate_id.clone(),
                reason,
            },
            None => CellOutcome::Run {
                candidate_id: row.candidate_id.clone(),
            },
        })
        .collect()
}

fn cell_skip_reason(
    matrix: &BenchmarkMatrix,
    row: &CandidateRow,
    baseline_gate: bool,
) -> Option<String> {
    if !quantization_supported(&row.quantization) {
        return Some(format!(
            "quantization {} is outside the Q3-Q8 range",
            row.quantization
        ));
    }
    if !SUPPORTED_RUNTIMES.contains(&row.runtime.as_str()) {
        return Some(format!(
            "runtime {} is not a supported runtime",
            row.runtime
        ));
    }
    if let Some(nodes) = &matrix.supported_nodes {
        if !nodes.iter().any(|node| node == &row.node) {
            return Some(format!(
                "node {} is not in the supported node list",
                row.node
            ));
        }
    }
    if row.speculative && !baseline_gate {
        return Some("speculative decoding is gated behind the baseline gate".to_string());
    }
    None
}

/// The median of a set of values, or `None` when empty.
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        Some(sorted[mid])
    } else {
        Some((sorted[mid - 1] + sorted[mid]) / 2.0)
    }
}

/// Per-candidate median successful issue times, in candidate_id order.
pub fn candidate_medians(matrix: &BenchmarkMatrix) -> Vec<(String, f64)> {
    let mut by_candidate: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for row in matrix.rows.iter().filter(|row| row.success) {
        if let Some(seconds) = row.success_seconds {
            if seconds.is_finite() && seconds > 0.0 {
                by_candidate
                    .entry(row.candidate_id.clone())
                    .or_default()
                    .push(seconds);
            }
        }
    }
    by_candidate
        .into_iter()
        .filter_map(|(id, times)| median(&times).map(|value| (id, value)))
        .collect()
}

/// The winning candidate id: the successful candidate with the lowest median
/// successful issue time. A candidate wins only when `success: true`; with no
/// successful rows there is no winner. Ties break on candidate_id.
pub fn winning_candidate_id(matrix: &BenchmarkMatrix) -> Option<String> {
    candidate_medians(matrix)
        .into_iter()
        .min_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        })
        .map(|(id, _)| id)
}

/// Build the final report comparing the winner's median to the baseline.
pub fn report(matrix: &BenchmarkMatrix) -> MatrixReport {
    let winner = winning_candidate_id(matrix);
    let winner_median = winner
        .as_ref()
        .and_then(|id| {
            candidate_medians(matrix)
                .into_iter()
                .find(|(candidate, _)| candidate == id)
        })
        .map(|(_, value)| value);
    MatrixReport {
        delta_vs_baseline_seconds: winner_median
            .map(|value| value - matrix.baseline.median_success_seconds),
        faster_than_baseline: winner_median
            .map(|value| value < matrix.baseline.median_success_seconds),
        baseline_gate_passed: baseline_gate_passed(matrix),
        winner,
        winner_median_success_seconds: winner_median,
        baseline_median_success_seconds: matrix.baseline.median_success_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(json: &str) -> BenchmarkMatrix {
        serde_json::from_str(json).expect("matrix json decodes")
    }

    fn row(id: &str, success: bool, seconds: Option<f64>) -> String {
        format!(
            r#"{{"candidate_id":"{id}","quantization":"Q4_K_M","runtime":"mlx",
            "node":"m13-01","profile":"coding-standard","success":{success}{extra}}}"#,
            extra = seconds
                .map(|s| format!(",\"success_seconds\":{s}"))
                .unwrap_or_default()
        )
    }

    fn matrix(rows: &[&str]) -> BenchmarkMatrix {
        decode(&format!(
            r#"{{"baseline":{{"quantization":"Q4_K_M","runtime":"llama.cpp",
            "node":"rtx-4090-01","profile":"coding-standard",
            "median_success_seconds":240}},"rows":[{}]}}"#,
            rows.join(",")
        ))
    }

    #[test]
    fn ac1_row_must_identify_quantization_runtime_node_profile() {
        let matrix = matrix(&[
            r#"{"candidate_id":"blank","quantization":"Q4_K_M","runtime":"",
            "node":"m13-01","profile":"coding-standard","success":false}"#,
        ]);
        let errors = validate(&matrix);
        assert!(errors.iter().any(|error| {
            error.candidate_id == "blank" && error.reason == "runtime must be identified"
        }));
        // A missing field is a parse error, so the contract holds at decode time.
        let missing = r#"{"baseline":{"quantization":"Q4_K_M","runtime":"mlx","node":"n",
            "profile":"p","median_success_seconds":1},"rows":[
            {"candidate_id":"x","quantization":"Q4_K_M","runtime":"mlx",
             "profile":"p","success":false}]}"#;
        assert!(serde_json::from_str::<BenchmarkMatrix>(missing).is_err());
    }

    #[test]
    fn ac2_speculative_rows_record_draft_tokens() {
        let missing =
            matrix(&[&row("spec", true, Some(100.0)).replace("}", ",\"speculative\":true}")]);
        let errors = validate(&missing);
        assert!(errors.iter().any(|error| {
            error.candidate_id == "spec"
                && error.reason
                    == "speculative row must record draft_tokens and accepted_draft_tokens"
        }));

        let full = matrix(&[
            &row("spec", true, Some(100.0)).replace(
                "}",
                ",\"speculative\":true,\"draft_tokens\":4,\"accepted_draft_tokens\":3}",
            ),
            &row("plain", true, Some(120.0)),
        ]);
        assert!(validate(&full).is_empty());
    }

    #[test]
    fn ac3_candidate_wins_only_when_success_true() {
        // The fastest row is a failure, so it must not win.
        let fast_fails = matrix(&[
            &row("fast-failed", false, Some(1.0)),
            &row("slow-succeeded", true, Some(300.0)),
        ]);
        assert_eq!(
            winning_candidate_id(&fast_fails),
            Some("slow-succeeded".to_string())
        );

        let failed_matrix = matrix(&[&row("failed", false, None)]);
        assert_eq!(winning_candidate_id(&failed_matrix), None);
    }

    #[test]
    fn ac4_report_compares_median_success_time_to_baseline() {
        let matrix = matrix(&[
            &row("a", true, Some(100.0)),
            &row("a", true, Some(200.0)),
            &row("a", true, Some(300.0)),
        ]);
        let report = report(&matrix);
        assert_eq!(report.winner, Some("a".to_string()));
        assert_eq!(report.winner_median_success_seconds, Some(200.0));
        assert_eq!(report.baseline_median_success_seconds, 240.0);
        assert_eq!(report.delta_vs_baseline_seconds, Some(-40.0));
        assert_eq!(report.faster_than_baseline, Some(true));
    }

    #[test]
    fn plan_cells_runs_supported_cells_and_preserves_skip_reasons() {
        let mut json = format!(
            r#"{{"baseline":{{"quantization":"Q4_K_M","runtime":"llama.cpp",
            "node":"rtx-4090-01","profile":"coding-standard",
            "median_success_seconds":240}},"supported_nodes":["m13-01"],
            "rows":[{}]}}"#,
            [
                r#"{"candidate_id":"good","quantization":"Q8_0","runtime":"vllm",
                  "node":"m13-01","profile":"coding-standard","success":true,
                  "success_seconds":180}"#,
                r#"{"candidate_id":"bad-runtime","quantization":"Q4_K_M","runtime":"trtllm",
                  "node":"m13-01","profile":"coding-standard","success":false}"#,
                r#"{"candidate_id":"bad-quant","quantization":"Q2_K","runtime":"mlx",
                  "node":"m13-01","profile":"coding-standard","success":false}"#,
                r#"{"candidate_id":"bad-node","quantization":"Q4_K_M","runtime":"mlx",
                  "node":"other-host","profile":"coding-standard","success":false}"#,
            ]
            .join(",")
        );
        let matrix = decode(&json);
        json.clear();
        let outcomes = plan_cells(&matrix);
        assert_eq!(outcomes.len(), 4);
        assert!(outcomes.iter().any(
            |cell| matches!(cell, CellOutcome::Run { candidate_id } if candidate_id == "good")
        ));
        let reasons = format!("{:?}", outcomes);
        assert!(reasons.contains("runtime trtllm is not a supported runtime"));
        assert!(reasons.contains("quantization Q2_K is outside the Q3-Q8 range"));
        assert!(reasons.contains("node other-host is not in the supported node list"));
    }

    #[test]
    fn speculative_cells_wait_for_the_baseline_gate() {
        let spec_row = row("spec", true, Some(100.0)).replace(
            "}",
            ",\"speculative\":true,\"draft_tokens\":4,\"accepted_draft_tokens\":3}",
        );
        let gated = decode(&format!(
            r#"{{"baseline":{{"quantization":"Q4_K_M","runtime":"llama.cpp",
            "node":"rtx-4090-01","profile":"coding-standard",
            "median_success_seconds":0}},"rows":[{spec_row}]}}"#
        ));
        assert!(!baseline_gate_passed(&gated));
        let outcomes = plan_cells(&gated);
        assert!(matches!(
            &outcomes[0],
            CellOutcome::Skip { reason, .. }
                if reason == "speculative decoding is gated behind the baseline gate"
        ));
    }

    #[test]
    fn quantization_range_covers_q3_through_q8() {
        assert!(quantization_supported("Q3_K_S"));
        assert!(quantization_supported("q8_0"));
        assert!(!quantization_supported("Q2_K"));
        assert!(!quantization_supported("Q9_0"));
        assert!(!quantization_supported("F16"));
        assert_eq!(quantization_class("Q4_K_M"), Some(4));
    }

    #[test]
    fn median_handles_empty_odd_and_even_sets() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 10.0]), Some(7.0));
    }
}
