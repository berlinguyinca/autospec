//! Reading run records off the filesystem, newest first.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::{RunOutcome, RunRecord};

/// How many trailing bytes of a stderr log are read. Failures report at the
/// end, and a run log can be arbitrarily large.
const STDERR_TAIL_BYTES: u64 = 256 * 1024;

/// Read every run record under `runs_dir`, newest first.
///
/// A run is a directory named after the run. Its outcome comes from
/// `status.json` (`{"status": "failed"}` or `{"exit_code": 137}`) or a plain
/// `status` file; its stderr from `stderr.log`, `stderr.txt`, `stderr`, the
/// first `*.err`, or the first `*.log`. A missing directory yields no runs
/// rather than an error: an empty fleet is a reportable state, not a failure
/// to report.
pub fn scan_runs_dir(runs_dir: &Path) -> Result<Vec<RunRecord>, String> {
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(runs_dir)
        .map_err(|error| format!("cannot read runs directory {}: {error}", runs_dir.display()))?;
    let mut found: Vec<(Option<std::time::SystemTime>, String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("cannot read runs directory {}: {error}", runs_dir.display())
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
        found.push((modified, name, path));
    }
    found.sort_by(|(left_time, left_name, _), (right_time, right_name, _)| {
        right_time
            .cmp(left_time)
            .then_with(|| right_name.cmp(left_name))
    });

    let mut records = Vec::with_capacity(found.len());
    for (_, name, path) in found {
        let outcome = read_status(&path);
        let stderr = read_stderr(&path)?;
        records.push(RunRecord::new(name, outcome, stderr));
    }
    Ok(records)
}

fn read_status(run_dir: &Path) -> RunOutcome {
    let json_path = run_dir.join("status.json");
    if let Ok(text) = fs::read_to_string(&json_path) {
        if let Some(outcome) = classify_json_status(&text) {
            return outcome;
        }
    }
    let plain_path = run_dir.join("status");
    if let Ok(text) = fs::read_to_string(&plain_path) {
        if let Some(outcome) = classify_status_word(text.trim()) {
            return outcome;
        }
    }
    RunOutcome::Missing
}

fn classify_json_status(text: &str) -> Option<RunOutcome> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if let Some(status) = value.get("status").and_then(|v| v.as_str()) {
        if let Some(outcome) = classify_status_word(status) {
            return Some(outcome);
        }
    }
    for key in ["exit_code", "exit", "code"] {
        if let Some(code) = value.get(key).and_then(|v| v.as_i64()) {
            return Some(outcome_for_code(code));
        }
    }
    None
}

fn outcome_for_code(code: i64) -> RunOutcome {
    if code == 0 {
        RunOutcome::Completed
    } else {
        RunOutcome::Failed
    }
}

fn classify_status_word(word: &str) -> Option<RunOutcome> {
    let word = word.trim().to_ascii_lowercase();
    if word.is_empty() {
        return None;
    }
    if let Ok(code) = word.parse::<i64>() {
        return Some(outcome_for_code(code));
    }
    match word.as_str() {
        "completed" | "complete" | "succeeded" | "success" | "passed" | "ok" | "done"
        | "finished" => Some(RunOutcome::Completed),
        "failed" | "failure" | "error" | "killed" | "timeout" | "timed_out" | "cancelled"
        | "canceled" | "oom" | "oomkilled" | "crashed" | "aborted" | "nonzero" | "non-zero" => {
            Some(RunOutcome::Failed)
        }
        _ => None,
    }
}

fn read_stderr(run_dir: &Path) -> Result<String, String> {
    let mut candidates: Vec<PathBuf> = ["stderr.log", "stderr.txt", "stderr"]
        .iter()
        .map(|name| run_dir.join(name))
        .collect();
    let mut extras: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(run_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && log_extension(&entry.file_name().to_string_lossy()) {
                extras.push(path);
            }
        }
    }
    extras.sort();
    candidates.extend(extras);
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        let text = read_tail(&path)?;
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }
    Ok(String::new())
}

fn log_extension(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".err") || name.ends_with(".log")
}

/// Read the last [`STDERR_TAIL_BYTES`] of a file; failures report at the end.
fn read_tail(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))?
        .len();
    let seeked = len > STDERR_TAIL_BYTES;
    if seeked {
        file.seek(SeekFrom::End(STDERR_TAIL_BYTES as i64 - len as i64))
            .map_err(|error| format!("cannot seek {}: {error}", path.display()))?;
    }
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&buffer).to_string();
    if !seeked {
        return Ok(text);
    }
    // A tail starts mid-line or splits a char boundary; drop the leading
    // partial line rather than reporting half a signature.
    let start = text.find('\n').map(|index| index + 1).unwrap_or(text.len());
    Ok(text[start..].to_string())
}
