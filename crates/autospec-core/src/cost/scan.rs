//! Scan a fleet `out/` directory for per-run cost records.
//!
//! The fleet harness lays one directory per run under `out/`
//! (`out/issue-*/`), and a run that reached a terminal status leaves a
//! `status.txt` inside. The scan classifies every run directory into exactly
//! one of:
//!
//! - **records** — a `status.txt` that parsed (possibly incomplete);
//! - **no_record** — a run directory with no `status.txt` at all (the run
//!   died before it could say anything; counted so the denominator stays
//!   honest);
//! - **malformed** — a `status.txt` with a value this reader cannot
//!   interpret, kept with its reason so the operator can fix the harness.
//!
//! A missing `out/` directory yields an empty scan, not an error: an empty
//! fleet is a reportable state, not a failure to report.

use std::fs;
use std::path::Path;

use super::record::RunRecord;

use serde::Serialize;

/// One run directory whose `status.txt` could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MalformedRecord {
    /// Run directory name.
    pub issue: String,
    /// Why the file was unreadable (the offending line and key).
    pub reason: String,
}

/// Everything the scan found under the `out/` directory.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CostScan {
    /// Parsed run records (costed or incomplete), sorted by run name.
    pub records: Vec<RunRecord>,
    /// Run directories with no `status.txt`, sorted by name.
    pub no_record: Vec<String>,
    /// Run directories whose `status.txt` failed to parse, sorted by name.
    pub malformed: Vec<MalformedRecord>,
}

/// Read every run directory under `out_dir` into a [`CostScan`].
pub fn scan_out_dir(out_dir: &Path) -> Result<CostScan, String> {
    let mut scan = CostScan::default();
    if !out_dir.is_dir() {
        return Ok(scan);
    }
    let entries = fs::read_dir(out_dir)
        .map_err(|error| format!("cannot read out directory {}: {error}", out_dir.display()))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("cannot read out directory {}: {error}", out_dir.display()))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let status_file = path.join("status.txt");
        if !status_file.is_file() {
            scan.no_record.push(name);
            continue;
        }
        let content = fs::read_to_string(&status_file)
            .map_err(|error| format!("cannot read {}: {error}", status_file.display()))?;
        match RunRecord::parse(&name, &content) {
            Ok(record) => scan.records.push(record),
            Err(reason) => scan.malformed.push(MalformedRecord {
                issue: name,
                reason,
            }),
        }
    }
    scan.records.sort_by(|a, b| a.issue.cmp(&b.issue));
    scan.no_record.sort();
    scan.malformed.sort_by(|a, b| a.issue.cmp(&b.issue));
    Ok(scan)
}
