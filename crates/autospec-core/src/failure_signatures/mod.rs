//! Failure-signature counting over a rolling window of fleet agent runs.
//!
//! A fleet of agent runs (autospec, inferweave, orchestrator) submits work to
//! a Slurm cluster. Every run leaves behind a run directory holding a status
//! file (when the agent got that far) and the captured stderr log. A single run
//! is one data point — see issue #3723 for why a per-run log is not a useful
//! unit of observability. The signal an operator needs is *repetition*: the
//! same crash signature in 62 of 190 recent runs is a broken dependency; the
//! same line once is a transient.
//!
//! This module turns a directory of run records into that report. Three
//! properties are guaranteed:
//!
//! - **The denominator travels with the count.** A bare "62" cannot be acted
//!   on; `62 / 190` can. Every [`SignatureCount`] carries the share of the
//!   window it came from, and the report carries the window size.
//! - **Threshold crossings mark themselves.** A signature whose share exceeds
//!   [`FailureSignatureReport::threshold_percent`] is flagged `systemic` in
//!   the report; nobody has to eyeball a log directory to find it.
//! - **Silent runs are counted, not dropped.** A run that died before writing
//!   a status file lands in [`NO_STATUS_SIGNATURE`]; a failed run that wrote no
//!   usable stderr line lands in [`NO_OUTPUT_SIGNATURE`]; a run killed by a
//!   walltime/budget timeout lands in [`TIMEOUT_NO_OUTPUT_SIGNATURE`], kept
//!   apart from [`NO_OUTPUT_SIGNATURE`] because "the budget was too small" is
//!   a different failure than "the code crashed silently" (issue #3690). All
//!   are ordinary buckets, so the runs that destroyed their own evidence
//!   become the loudest entries instead of disappearing from the arithmetic.
//!
//! Signature normalization keeps its cost where the value is: the last
//! non-Slurm stderr line, with numbers and paths masked (see
//! [`normalize_signature_line`]). The masked line is a *key for counting*, not
//! a replacement for the log — the report never claims to be the first bad
//! commit, only that one change's worth of signatures repeats.

mod report;
mod scan;
mod signature;

pub use report::{share_percent, FailureSignatureReport, SignatureCount};
pub use scan::scan_runs_dir;
pub use signature::{is_slurm_noise, last_meaningful_line, normalize_signature_line};

use std::collections::BTreeMap;

/// Default directory scanned for run records, matching the per-run layout under
/// `.autospec/runs/<run_id>/` written by `execution::result::ResultPaths`.
pub const DEFAULT_RUNS_DIR: &str = ".autospec/runs";

/// Default rolling window: how many most-recent runs the report counts.
pub const DEFAULT_WINDOW: usize = 200;

/// Default repetition threshold as a percentage of the runs in the window.
pub const DEFAULT_THRESHOLD_PERCENT: f64 = 5.0;

/// Default number of signatures listed in a report.
pub const DEFAULT_TOP: usize = 10;

/// Bucket for runs that never wrote a status file. The run that got killed
/// before it could record anything lands here.
pub const NO_STATUS_SIGNATURE: &str = "<no status file>";

/// Bucket for failed runs that wrote no usable stderr line.
pub const NO_OUTPUT_SIGNATURE: &str = "<no output>";

/// Bucket for runs killed by a walltime/budget timeout that wrote no usable
/// stderr line. Kept distinct from [`NO_OUTPUT_SIGNATURE`]: a silent timeout
/// means the agent's budget was too small for the job (issue #3690), while a
/// silent crash means the code is broken — the frontier reacts to these
/// differently (grow the budget versus fix the crash), so they must not
/// share a bucket.
pub const TIMEOUT_NO_OUTPUT_SIGNATURE: &str = "<timeout, no output>";

/// Status recorded for one run, read from its status file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The run reported success (status `completed`, `passed`, exit code 0…).
    Completed,
    /// The run reported failure (status `failed`, `killed`, non-zero exit…).
    Failed,
    /// The run was killed by a walltime/budget timeout: status `timeout` /
    /// `timed_out`, or exit code 124 (runner `timeout` kill) or 281 (Slurm
    /// walltime kill). Distinct from [`RunOutcome::Failed`]: the failure is
    /// the budget, not the job (issue #3690).
    Timeout,
    /// No status file, or one this reader could not interpret. Distinct from
    /// [`RunOutcome::Failed`]: an absent status file means the run never got
    /// to say anything about itself.
    Missing,
}

/// One agent run as read from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    /// Run identifier — the run directory name.
    pub id: String,
    /// Outcome recorded by the run's status file.
    pub outcome: RunOutcome,
    /// Captured stderr text (possibly a tail of the full log).
    pub stderr: String,
}

impl RunRecord {
    pub fn new(id: impl Into<String>, outcome: RunOutcome, stderr: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            outcome,
            stderr: stderr.into(),
        }
    }

    /// The signature this run contributes to the counts.
    pub fn signature(&self) -> String {
        match self.outcome {
            RunOutcome::Completed => String::new(),
            RunOutcome::Missing => NO_STATUS_SIGNATURE.to_string(),
            // A timeout that still produced a meaningful stderr line reports
            // that line; only the silent ones take the dedicated bucket.
            RunOutcome::Timeout => match last_meaningful_line(&self.stderr) {
                Some(line) => normalize_signature_line(line),
                None => TIMEOUT_NO_OUTPUT_SIGNATURE.to_string(),
            },
            RunOutcome::Failed => match last_meaningful_line(&self.stderr) {
                Some(line) => normalize_signature_line(line),
                None => NO_OUTPUT_SIGNATURE.to_string(),
            },
        }
    }
}

/// Count signatures over the last `window` runs.
///
/// `runs` is expected newest-first (what [`scan_runs_dir`] returns); the
/// rolling window is its first `window` entries. The denominator is the number
/// of runs in that window, so a window of 190 runs reports `/ 190` even when
/// only a handful failed.
pub fn analyze(
    runs_dir: &str,
    runs: &[RunRecord],
    window: usize,
    threshold_percent: f64,
    top: usize,
) -> FailureSignatureReport {
    let windowed = &runs[..runs.len().min(window)];
    let total = windowed.len();
    let failed_runs = windowed
        .iter()
        .filter(|run| run.outcome != RunOutcome::Completed)
        .count();

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for run in windowed {
        let signature = run.signature();
        if signature.is_empty() {
            continue;
        }
        *counts.entry(signature).or_insert(0) += 1;
    }

    // All silent-run buckets are counted here: the report must never make the
    // denominator smaller by losing the runs that destroyed their own evidence.
    let unsigned_runs = counts.get(NO_STATUS_SIGNATURE).copied().unwrap_or(0)
        + counts.get(NO_OUTPUT_SIGNATURE).copied().unwrap_or(0)
        + counts
            .get(TIMEOUT_NO_OUTPUT_SIGNATURE)
            .copied()
            .unwrap_or(0);

    let mut entries: Vec<SignatureCount> = counts
        .into_iter()
        .map(|(signature, count)| SignatureCount {
            share_percent: share_percent(count, total),
            systemic: crosses_threshold(count, total, threshold_percent),
            count,
            signature,
        })
        .collect();
    entries.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.signature.cmp(&b.signature))
    });

    let systemic = entries.iter().any(|entry| entry.systemic);
    let truncated = entries.len().saturating_sub(top);
    entries.truncate(top);

    FailureSignatureReport {
        runs_dir: runs_dir.to_string(),
        window,
        runs: total,
        failed_runs,
        unsigned_runs,
        threshold_percent,
        systemic,
        signatures: entries,
        truncated,
    }
}

/// Strictly more than `threshold_percent` of `total`, computed on integers to
/// keep the boundary free of float drift (`>5%` of 190 is 10 runs, not 9.5).
fn crosses_threshold(count: usize, total: usize, threshold_percent: f64) -> bool {
    total > 0 && (count as f64) * 100.0 > threshold_percent * (total as f64)
}
