//! The report shape: counts carrying their denominator, and its renderings.

use serde::Serialize;

/// One signature's count over the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignatureCount {
    pub signature: String,
    pub count: usize,
    /// `count / runs` as a percentage, where `runs` is the window denominator.
    pub share_percent: f64,
    /// True when `share_percent` is strictly above the report threshold.
    pub systemic: bool,
}

/// Failure signatures counted over a rolling window of runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FailureSignatureReport {
    /// Directory the run records were read from.
    pub runs_dir: String,
    /// Window size requested by the caller.
    pub window: usize,
    /// Denominator: runs actually counted (window capped by what exists).
    pub runs: usize,
    /// Runs in the window that did not report success — either they reported
    /// failure or they never got to report anything at all.
    pub failed_runs: usize,
    /// Runs in the window that produced no signature of their own
    /// ([`NO_STATUS_SIGNATURE`](super::NO_STATUS_SIGNATURE) plus
    /// [`NO_OUTPUT_SIGNATURE`](super::NO_OUTPUT_SIGNATURE)).
    pub unsigned_runs: usize,
    /// A signature above this share of `runs` is flagged systemic.
    pub threshold_percent: f64,
    /// True when at least one signature crossed the threshold.
    pub systemic: bool,
    /// Signatures sorted by count descending, truncated to `top`.
    pub signatures: Vec<SignatureCount>,
    /// Signatures counted but not listed in `signatures`.
    pub truncated: usize,
}

impl FailureSignatureReport {
    /// True when the window held no runs at all.
    pub fn is_empty(&self) -> bool {
        self.runs == 0
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|error| {
            format!("{{\"error\":\"failure signature report not serializable: {error}\"}}")
        })
    }

    /// Text rendering. The first line carries the whole headline — counts,
    /// denominator, threshold verdict — because that is all an operator
    /// skimming a monitor log reads.
    pub fn to_text(&self) -> String {
        if self.runs == 0 {
            return format!(
                "no agent runs found in {} — nothing to count",
                self.runs_dir
            );
        }
        let mut out = String::new();
        out.push_str(&self.headline());
        if self.unsigned_runs > 0 {
            out.push_str(&format!(
                " {} of them produced no output of their own (counted, not dropped).",
                self.unsigned_runs
            ));
        }
        if self.signatures.is_empty() {
            out.push_str("\nno failure signatures in the window.");
            return out;
        }
        out.push_str(&self.table());
        out
    }

    fn headline(&self) -> String {
        format!(
            "{} of {} runs failed ({:.1}%) in the last {} runs of {}; {} systemic signature(s) above the {:.1}% threshold.",
            self.failed_runs,
            self.runs,
            share_percent(self.failed_runs, self.runs),
            self.runs,
            self.runs_dir,
            self.systemic_count(),
            self.threshold_percent
        )
    }

    fn table(&self) -> String {
        let mut out = String::from("\n  count   share  status     signature");
        for entry in &self.signatures {
            out.push_str(&format!(
                "\n  {:>5}  {:>5}%  {:<9}  {}",
                entry.count,
                entry.share_percent,
                if entry.systemic { "SYSTEMIC" } else { "-" },
                entry.signature
            ));
        }
        if self.truncated > 0 {
            out.push_str(&format!(
                "\n  ({} more signatures not shown)",
                self.truncated
            ));
        }
        out
    }

    /// How many of the *listed* signatures crossed the threshold.
    fn systemic_count(&self) -> usize {
        self.signatures.iter().filter(|e| e.systemic).count()
    }
}

/// `count` as a percentage of `total`, `0` for an empty window.
pub fn share_percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (count as f64) * 100.0 / (total as f64)
    }
}
