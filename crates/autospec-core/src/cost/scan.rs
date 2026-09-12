//! Scan a fleet `out/` directory for per-run cost records.
//!
//! The fleet harness lays one directory per issue under `out/`
//! (`out/issue-*/`), and a run that reached a terminal status leaves a
//! `status.txt` inside. Every re-dispatch overwrites that directory
//! (`rm -rf "$OUT"`), so the previous run's record survives only when it
//! was archived first (`out/archive/`, see [`super::archive`]); the scan
//! reads the archived per-run records alongside the live one, so cost
//! accounting is computed from immutable per-run records (#3940).
//!
//! The scan classifies every run directory into exactly one of:
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

use super::archive::ARCHIVE_DIR;
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
    /// Parsed records read from `out/archive/` (pre-redispatch copies): how
    /// much of the total is per-run history rather than one last run per
    /// issue.
    pub archived: u64,
}

/// Read every run directory under `out_dir` into a [`CostScan`].
///
/// Entry names are processed in sorted order so the scan (and everything
/// built on it) is deterministic for a given `out/` tree, including when
/// one issue has both a live record and an archived one.
pub fn scan_out_dir(out_dir: &Path) -> Result<CostScan, String> {
    let mut scan = CostScan::default();
    if !out_dir.is_dir() {
        return Ok(scan);
    }
    let mut names = Vec::new();
    let entries = fs::read_dir(out_dir)
        .map_err(|error| format!("cannot read out directory {}: {error}", out_dir.display()))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("cannot read out directory {}: {error}", out_dir.display()))?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.is_empty() && !name.starts_with('.') {
            names.push(name);
        }
    }
    names.sort();
    for name in names {
        let path = out_dir.join(&name);
        if name == ARCHIVE_DIR {
            scan_archive(&path, &mut scan)?;
            continue;
        }
        scan_run_dir(&name, &name, &path, false, &mut scan)?;
    }
    scan.records.sort_by(|a, b| a.issue.cmp(&b.issue));
    scan.no_record.sort();
    scan.malformed.sort_by(|a, b| a.issue.cmp(&b.issue));
    Ok(scan)
}

/// Read one run directory (live or archived) into the scan.
///
/// `issue` is the issue the record counts toward; `label` is what the
/// report names when the directory holds no `status.txt` (archived runs
/// are labelled by their full path so they stay distinguishable from the
/// live run of the same issue).
fn scan_run_dir(
    issue: &str,
    label: &str,
    run_dir: &Path,
    archived: bool,
    scan: &mut CostScan,
) -> Result<(), String> {
    let status_file = run_dir.join("status.txt");
    if !status_file.is_file() {
        scan.no_record.push(label.to_string());
        return Ok(());
    }
    let content = fs::read_to_string(&status_file)
        .map_err(|error| format!("cannot read {}: {error}", status_file.display()))?;
    match RunRecord::parse(issue, &content) {
        Ok(record) => {
            if archived {
                scan.archived += 1;
            }
            scan.records.push(record);
        }
        Err(reason) => scan.malformed.push(MalformedRecord {
            issue: issue.to_string(),
            reason,
        }),
    }
    Ok(())
}

/// Read the per-run records under `out/archive/`.
///
/// Two layouts hold an immutable per-run record:
///
/// - `archive/<issue>/run-<n>/status.txt` — what
///   [`super::archive::archive_run`] produces on every re-dispatch;
/// - `archive/<issue>/status.txt` — the whole run directory moved into the
///   archive by hand before a re-dispatch.
///
/// Both count as runs of `<issue>`; neither is ever overwritten, which is
/// the property the live `out/issue-*/` directories lack.
fn scan_archive(archive_root: &Path, scan: &mut CostScan) -> Result<(), String> {
    let mut issues = Vec::new();
    let entries = fs::read_dir(archive_root).map_err(|error| {
        format!(
            "cannot read archive directory {}: {error}",
            archive_root.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "cannot read archive directory {}: {error}",
                archive_root.display()
            )
        })?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.is_empty() && !name.starts_with('.') {
            issues.push(name);
        }
    }
    issues.sort();
    for issue in issues {
        let issue_dir = archive_root.join(&issue);
        // The manual-move layout: the whole run directory, `status.txt`
        // included, moved into the archive.
        if issue_dir.join("status.txt").is_file() {
            scan_run_dir(&issue, &format!("archive/{issue}"), &issue_dir, true, scan)?;
        }
        // The `run-<n>` bundle layout: each subdirectory is one immutable
        // per-run record.
        let Ok(sub_entries) = fs::read_dir(&issue_dir) else {
            continue;
        };
        let mut runs = Vec::new();
        for entry in sub_entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.is_empty() && !name.starts_with('.') {
                runs.push(name);
            }
        }
        runs.sort();
        for run in runs {
            scan_run_dir(
                &issue,
                &format!("archive/{issue}/{run}"),
                &issue_dir.join(&run),
                true,
                scan,
            )?;
        }
    }
    Ok(())
}
