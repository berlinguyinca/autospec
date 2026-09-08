//! `autospec doctor failures` — failure signatures over a rolling window of
//! fleet agent runs.
//!
//! One run's log answers nothing useful (issue #3723). This subcommand counts
//! signatures across the last N runs and reports them with the denominator, so
//! "62 runs share this signature" reads as 62/190 rather than as a bare count,
//! and marks any signature above the repetition threshold systemic. Runs that
//! produced no output of their own are counted in their own buckets rather than
//! dropped from the arithmetic.

use std::path::{Path, PathBuf};

use autospec_core::failure_signatures::{
    analyze, scan_runs_dir, DEFAULT_RUNS_DIR, DEFAULT_THRESHOLD_PERCENT, DEFAULT_TOP,
    DEFAULT_WINDOW,
};

/// What the caller renders and which exit code to leave by.
pub struct Outcome {
    pub rendered: String,
    /// True when at least one signature crossed the repetition threshold. The
    /// caller exits 1 on this so a monitor can detect a systemic failure
    /// without scraping the table.
    pub systemic: bool,
}

/// Exit code when a signature crossed the threshold.
pub const SYSTEMIC_EXIT_CODE: i32 = 1;

const USAGE: &str = "autospec doctor failures [--runs-dir DIR] [--last N] [--threshold-percent N] [--top K] [--json]\n\n\
Count failure signatures over a rolling window of fleet agent runs.\n\n\
Run records are read from a directory holding one subdirectory per run\n\
(default .autospec/runs). Each run directory may hold:\n\
  status.json  {\"status\": \"failed\"} or {\"exit_code\": 137}, or a plain 'status' file\n\
  stderr.log   the captured stderr (stderr.txt, stderr, *.err and *.log are also read)\n\n\
A run with no status file is counted under '<no status file>'; a failed run with\n\
no usable stderr line is counted under '<no output>'. Neither is dropped.\n\n\
OPTIONS:\n\
    --runs-dir DIR         runs directory (default .autospec/runs)\n\
    --last N               rolling window in runs (default 200)\n\
    --threshold-percent N  systemic above this share of the window (default 5)\n\
    --top K                signatures listed (default 10)\n\
    --json                 emit the report as JSON\n\n\
EXIT CODES:\n\
    0  no signature crossed the threshold\n\
    1  at least one signature is systemic\n\
    2  bad arguments or unreadable run records";

pub fn run(root: &Path, args: &[String]) -> Result<Outcome, String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(Outcome {
            rendered: USAGE.to_string(),
            systemic: false,
        });
    }

    let options = parse(args)?;
    let runs_dir = resolve_runs_dir(root, &options.runs_dir);
    let records = scan_runs_dir(&runs_dir)?;
    let report = analyze(
        &options.runs_dir,
        &records,
        options.window,
        options.threshold_percent,
        options.top,
    );

    let rendered = if options.json {
        report.to_json()
    } else {
        report.to_text()
    };
    Ok(Outcome {
        rendered,
        systemic: report.systemic,
    })
}

struct Options {
    runs_dir: String,
    window: usize,
    threshold_percent: f64,
    top: usize,
    json: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        runs_dir: DEFAULT_RUNS_DIR.to_string(),
        window: DEFAULT_WINDOW,
        threshold_percent: DEFAULT_THRESHOLD_PERCENT,
        top: DEFAULT_TOP,
        json: false,
    };
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--runs-dir" => options.runs_dir = take_value(args, &mut index, flag)?,
            "--last" => {
                let raw = take_value(args, &mut index, flag)?;
                options.window = parse_positive(raw, flag)?;
            }
            "--top" => {
                let raw = take_value(args, &mut index, flag)?;
                options.top = parse_positive(raw, flag)?;
            }
            "--threshold-percent" => {
                let raw = take_value(args, &mut index, flag)?;
                options.threshold_percent = parse_threshold(raw)?;
            }
            "--json" => options.json = true,
            other => return Err(format!("unknown doctor failures option: {other}")),
        }
        index += 1;
    }
    Ok(options)
}

const THRESHOLD_ERROR: &str = "--threshold-percent expects a number greater than 0 and at most 100";

/// Consume the value belonging to the flag at `*index`, advancing the cursor.
fn take_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

/// A share of the window: finite, greater than 0, at most 100.
fn parse_threshold(raw: String) -> Result<f64, String> {
    let value: f64 = raw
        .trim()
        .parse()
        .map_err(|_| THRESHOLD_ERROR.to_string())?;
    if value.is_finite() && value > 0.0 && value <= 100.0 {
        Ok(value)
    } else {
        Err(format!("{THRESHOLD_ERROR}, got {raw}"))
    }
}

fn parse_positive(raw: String, flag: &str) -> Result<usize, String> {
    raw.parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{flag} expects a positive integer, got {raw}"))
}

fn resolve_runs_dir(root: &Path, runs_dir: &str) -> PathBuf {
    let candidate = PathBuf::from(runs_dir);
    if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    }
}
