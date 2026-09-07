//! Provider-neutral context control: deterministic tool-result
//! normalization, large-file read protection, and automatic compaction at
//! the `60` / `72` / `88` percent occupancy thresholds.
//!
//! Everything here is pure and side-effect-free: callers persist the
//! artifact at the returned path and execute compaction of returned events,
//! mirroring the [`super::ContextMonitorEngine`] convention.

/// Cap on normalized lines injected from any single successful tool log.
pub const MAX_NORMALIZED_LINES: usize = 40;

/// Files strictly over this many lines require an explicit read range.
pub const MAX_FILE_LINES: usize = 1500;

/// Occupancy percentages (percent) at which compaction events fire.
pub const COMPACTION_THRESHOLDS: [u8; 3] = [60, 72, 88];

/// Tail size (log lines) injected for a failed command.
const FAILURE_TAIL_LINES: usize = 10;

/// Default directory where failed-command full logs are persisted.
const DEFAULT_ARTIFACT_DIR: &str = "artifacts/tool-logs";

/// A raw tool result awaiting normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// Human-readable command that produced the log.
    pub command: String,
    /// Process exit code; `0` means success.
    pub exit_code: i32,
    /// Full command output (stdout/stderr), unbounded.
    pub log: String,
    /// Optional directory prefix for the persisted full log of a failure.
    pub artifact_dir: Option<String>,
}

/// Deterministic, bounded view of a [`ToolOutput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedToolResult {
    /// True when `exit_code` is `0`.
    pub ok: bool,
    /// Original exit code, preserved on every result.
    pub exit_code: i32,
    /// Path the caller must persist the full log to; only set on failure.
    pub full_log_artifact: Option<String>,
    /// Bounded lines safe to inject into the session context.
    pub injected_lines: Vec<String>,
}

/// Normalize a successful or failed tool log into bounded injectable lines.
///
/// Success: a summary header plus the log head/tail, at most
/// [`MAX_NORMALIZED_LINES`] lines. Failure: a concise header and tail plus
/// `full_log_artifact`; the full log is never injected.
pub fn normalize_tool_output(output: &ToolOutput) -> NormalizedToolResult {
    let lines: Vec<String> = output.log.lines().map(String::from).collect();
    let total = lines.len();
    let ok = output.exit_code == 0;

    if ok {
        let mut injected = vec![format!(
            "ok command={} exit=0 lines={}",
            output.command, total
        )];
        if total > MAX_NORMALIZED_LINES - 2 {
            // Header + omission marker leave MAX_NORMALIZED_LINES - 2 slots.
            let body = MAX_NORMALIZED_LINES - 2;
            let head = body / 2;
            let tail = body - head;
            injected.extend(lines[..head].iter().map(String::from));
            injected.push(format!("... {} lines omitted ...", total - head - tail));
            injected.extend(lines[total - tail..].iter().map(String::from));
        } else {
            injected.extend(lines.iter().map(String::from));
        }
        return NormalizedToolResult {
            ok,
            exit_code: output.exit_code,
            full_log_artifact: None,
            injected_lines: injected,
        };
    }

    let artifact = format!(
        "{}/{}.log",
        output
            .artifact_dir
            .as_deref()
            .unwrap_or(DEFAULT_ARTIFACT_DIR),
        sanitize(&output.command)
    );
    let mut injected = vec![format!(
        "failed command={} exit={} lines={} artifact={}",
        output.command, output.exit_code, total, artifact
    )];
    let tail_start = total.saturating_sub(FAILURE_TAIL_LINES);
    if tail_start > 0 {
        injected.push(format!(
            "... {} lines omitted; full log at {} ...",
            tail_start, artifact
        ));
    }
    injected.extend(lines[tail_start..].iter().map(String::from));
    NormalizedToolResult {
        ok,
        exit_code: output.exit_code,
        full_log_artifact: Some(artifact),
        injected_lines: injected,
    }
}

/// Sanitize a command into a stable artifact file stem.
fn sanitize(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    for ch in command.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Decision for reading a file, given its line count and optional range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileReadPlan {
    /// The whole file fits under the configured limit.
    Whole,
    /// Read the explicit inclusive 1-based line range.
    Range { start: usize, end: usize },
    /// The file exceeds the limit (or the range is invalid): the caller
    /// must supply an explicit valid range.
    RangeRequired,
}

/// Plan a file read against the configured line limit.
///
/// Files of at most `max_lines` lines read whole. Files over the limit
/// (or an invalid `range`) require an explicit range with
/// `1 <= start <= end <= total_lines`.
pub fn plan_file_read(
    total_lines: usize,
    range: Option<(usize, usize)>,
    max_lines: usize,
) -> FileReadPlan {
    let range_ok = |range: Option<(usize, usize)>| match range {
        Some((start, end)) => {
            (1..=total_lines).contains(&start) && (start..=total_lines).contains(&end)
        }
        None => false,
    };
    match range {
        Some((start, end)) if range_ok(Some((start, end))) => FileReadPlan::Range { start, end },
        _ if total_lines <= max_lines => FileReadPlan::Whole,
        _ => FileReadPlan::RangeRequired,
    }
}

/// One automatic compaction request produced by [`CompactionTracker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionEvent {
    /// The threshold (percent) this event fired for.
    pub threshold_percent: u8,
    /// Occupancy (percent) that crossed the threshold.
    pub occupancy_percent: u8,
}

/// Tracks occupancy and fires at most one compaction event per report, at
/// the highest newly-crossed threshold.
///
/// Thresholds stay "reached" until occupancy drops below the lowest
/// threshold, which re-arms all of them — a compaction that drops the
/// session under 60 percent is what permits the next compaction.
#[derive(Debug, Clone)]
pub struct CompactionTracker {
    thresholds: [u8; 3],
    reached: [bool; 3],
}

impl Default for CompactionTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CompactionTracker {
    /// Tracker armed at the pinned [`COMPACTION_THRESHOLDS`].
    pub fn new() -> Self {
        Self {
            thresholds: COMPACTION_THRESHOLDS,
            reached: [false; 3],
        }
    }

    /// The active thresholds, lowest first.
    pub fn thresholds(&self) -> [u8; 3] {
        self.thresholds
    }

    /// Report current occupancy (percent).
    ///
    /// Returns exactly one [`CompactionEvent`] when this report newly
    /// crosses a threshold (the highest one crossed), otherwise an empty
    /// vec. Occupancy below the lowest threshold re-arms the tracker.
    pub fn record(&mut self, occupancy_percent: u8) -> Vec<CompactionEvent> {
        let crossed = (0..3)
            .rev()
            .find(|&i| occupancy_percent >= self.thresholds[i]);
        match crossed {
            Some(highest) if !self.reached[highest] => {
                for i in 0..=highest {
                    self.reached[i] = true;
                }
                vec![CompactionEvent {
                    threshold_percent: self.thresholds[highest],
                    occupancy_percent,
                }]
            }
            _ => {
                if occupancy_percent < self.thresholds[0] {
                    self.reached = [false; 3];
                }
                Vec::new()
            }
        }
    }
}

/// The anchors a compacted session must keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// The active goal.
    pub goal: String,
    /// The working diff (path summary or diff body).
    pub diff: String,
    /// The test command or status.
    pub tests: String,
    /// Outstanding failures.
    pub failures: Vec<String>,
}

/// Produce the compacted session summary: goal, diff, tests, and failures
/// survive; everything else in the transcript is dropped.
pub fn compact_session(snapshot: &SessionSnapshot) -> String {
    let failures = if snapshot.failures.is_empty() {
        "none".to_string()
    } else {
        snapshot.failures.join("; ")
    };
    format!(
        "## Compacted session\n- goal: {}\n- diff: {}\n- tests: {}\n- failures: {}\n",
        snapshot.goal, snapshot.diff, snapshot.tests, failures
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_collapses_separators() {
        assert_eq!(
            sanitize("cargo test -p autospec-core"),
            "cargo-test-p-autospec-core"
        );
        assert_eq!(sanitize("--weird --flag"), "weird-flag");
    }

    #[test]
    fn single_line_failed_log_has_no_omission_marker() {
        let result = normalize_tool_output(&ToolOutput {
            command: "echo boom".to_string(),
            exit_code: 2,
            log: "boom".to_string(),
            artifact_dir: None,
        });
        assert_eq!(
            result.injected_lines,
            vec![
                format!(
                "failed command=echo boom exit=2 lines=1 artifact=artifacts/tool-logs/echo-boom.log"
            ),
                "boom".to_string(),
            ]
        );
    }

    #[test]
    fn passing_log_that_fits_is_verbatim() {
        let log = (0..38)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = normalize_tool_output(&ToolOutput {
            command: "cargo fmt --check".to_string(),
            exit_code: 0,
            log,
            artifact_dir: None,
        });
        assert_eq!(result.injected_lines.len(), 39);
        assert!(!result.injected_lines.iter().any(|l| l.contains("omitted")));
    }
}
