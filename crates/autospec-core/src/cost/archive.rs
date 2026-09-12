//! Pre-redispatch archiving of the fleet run directory (issue #3940).
//!
//! The fleet harness lays one directory per issue under `out/`
//! (`out/issue-*/`) and begins every re-dispatch with `rm -rf "$OUT"`,
//! destroying the previous run's outcome. Counted over the per-job logs,
//! which survive, exactly half the fleet's runs were invisible to any
//! analysis over `status.txt` — and the surviving record of each issue is
//! the run that finally succeeded, so the accounting was systematically
//! biased toward success.
//!
//! This module makes the archive the default instead of an operator habit:
//! before a re-dispatch overwrites `out/<issue>`, the previous run's
//! directory is moved to `out/archive/<issue>/run-<n>` — never deleted —
//! and the cost scan reads the archived per-run records alongside the
//! live one. Cost accounting is then computed from immutable per-run
//! records, and a re-dispatch cannot erase the outcome it replaces.
//!
//! Nothing in this module deletes: the plan inspects, the archive moves.
//! The redispatching runner calls [`archive_run`] (exposed as
//! `autospec cost archive <issue>`) before it removes anything, and a
//! refused archive (destination already present) must stop the
//! re-dispatch, not be worked around.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The directory under `out/` that holds archived pre-redispatch runs.
pub const ARCHIVE_DIR: &str = "archive";

/// What a re-dispatch must do to `out/<issue>` before it overwrites it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RedispatchPlan {
    /// `out/<issue>` is absent or empty: nothing to preserve.
    Fresh,
    /// `out/<issue>` holds a previous run's output: move it to `dest`
    /// before the re-dispatch. Never delete it.
    Archive { dest: PathBuf },
}

impl RedispatchPlan {
    /// Whether the re-dispatch must archive first.
    pub fn requires_archive(&self) -> bool {
        matches!(self, Self::Archive { .. })
    }
}

fn validated_issue(issue: &str) -> Result<(), String> {
    if issue == ARCHIVE_DIR {
        return Err(format!(
            "issue name {issue:?} is reserved for the pre-redispatch archive root; refusing to use it as a run directory name"
        ));
    }
    if issue.is_empty() || issue.starts_with('.') || issue.contains('/') || issue.contains('\\') {
        return Err(format!(
            "issue name {issue:?} is not a safe single path component; refusing to touch a path derived from it"
        ));
    }
    Ok(())
}

/// The archive root under an out directory: `out/archive/`.
pub fn archive_root(out_dir: &Path) -> PathBuf {
    out_dir.join(ARCHIVE_DIR)
}

/// Parse a `run-<n>` archive directory name; `None` for anything else.
fn run_index_of(name: &str) -> Option<u64> {
    name.strip_prefix("run-")?.parse::<u64>().ok()
}

fn existing_run_indexes(issue_archive: &Path) -> Vec<u64> {
    let Ok(entries) = fs::read_dir(issue_archive) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| run_index_of(&entry.file_name().to_string_lossy()))
        .collect()
}

/// Inspect `out/<issue>` and decide what a re-dispatch must do to the
/// previous run's directory.
///
/// A directory that exists and holds any entry — `status.txt`, the patch,
/// the logs — holds the previous run's outcome and must be archived, even
/// when the run never recorded a terminal status: the absence of a record
/// is itself evidence the cost report counts. An absent or empty directory
/// is fresh.
pub fn plan_redispatch(out_dir: &Path, issue: &str) -> Result<RedispatchPlan, String> {
    validated_issue(issue)?;
    let run_dir = out_dir.join(issue);
    if !run_dir.is_dir() {
        return Ok(RedispatchPlan::Fresh);
    }
    let entries = fs::read_dir(&run_dir)
        .map_err(|error| format!("cannot read run directory {}: {error}", run_dir.display()))?;
    let mut has_content = false;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("cannot read run directory {}: {error}", run_dir.display()))?;
        if !entry.file_name().to_string_lossy().starts_with('.') {
            has_content = true;
            break;
        }
    }
    if !has_content {
        return Ok(RedispatchPlan::Fresh);
    }
    let issue_archive = archive_root(out_dir).join(issue);
    let next = existing_run_indexes(&issue_archive)
        .into_iter()
        .max()
        .map_or(1, |n| n + 1);
    Ok(RedispatchPlan::Archive {
        dest: issue_archive.join(format!("run-{next}")),
    })
}

/// Archive the previous run of `issue` before a re-dispatch overwrites it,
/// returning the archive destination, or `Ok(None)` when there was nothing
/// to preserve.
///
/// The move is the default a re-dispatch performs instead of `rm -rf`: the
/// outcome lands in `out/archive/<issue>/run-<n>` and stays readable by the
/// cost scan. This function never deletes; the refuse-when-destination-exists
/// guard lives in [`move_to_archive`].
pub fn archive_run(out_dir: &Path, issue: &str) -> Result<Option<PathBuf>, String> {
    match plan_redispatch(out_dir, issue)? {
        RedispatchPlan::Fresh => Ok(None),
        RedispatchPlan::Archive { dest } => move_to_archive(&out_dir.join(issue), &dest).map(Some),
    }
}

/// Move a run directory to its archive destination, refusing when the
/// destination already exists.
///
/// The plan avoids existing `run-<n>` names, but anything else can create
/// the path between plan and move; a refused archive (an `Err`, non-zero
/// exit at the CLI) is the guard against it, because merging into or
/// overwriting an archive would recreate exactly the data loss this exists
/// to prevent.
pub fn move_to_archive(run_dir: &Path, dest: &Path) -> Result<PathBuf, String> {
    if dest.exists() {
        return Err(format!(
            "archive destination {} already exists; refusing to merge into or overwrite an archive",
            dest.display()
        ));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::rename(run_dir, dest).map_err(|error| {
        format!(
            "cannot move {} to {}: {error}",
            run_dir.display(),
            dest.display()
        )
    })?;
    if !dest.is_dir() {
        return Err(format!(
            "archive destination {} missing after the move",
            dest.display()
        ));
    }
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;

    fn temp_out() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("autospec-cost-archive-{}-{nanos}", process::id()));
        fs::create_dir_all(&path).expect("out dir");
        path
    }

    fn write_run(out: &Path, issue: &str, body: &str) {
        let dir = out.join(issue);
        fs::create_dir_all(&dir).expect("run dir");
        fs::write(dir.join("status.txt"), body).expect("status.txt");
    }

    #[test]
    fn plan_is_fresh_for_absent_or_empty_run_dirs() {
        let out = temp_out();
        fs::create_dir_all(out.join("issue-1")).expect("empty run dir");
        assert_eq!(
            plan_redispatch(&out, "issue-9").expect("absent"),
            RedispatchPlan::Fresh
        );
        assert_eq!(
            plan_redispatch(&out, "issue-1").expect("empty"),
            RedispatchPlan::Fresh
        );
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn plan_archives_a_populated_run_dir_and_numbers_successive_runs() {
        let out = temp_out();
        write_run(
            &out,
            "issue-2",
            "status: NEW-TEST-FAILURES\nagent_secs: 7200\n",
        );
        let expected = out.join("archive/issue-2/run-1");
        let plan = plan_redispatch(&out, "issue-2").expect("plan");
        assert_eq!(
            plan,
            RedispatchPlan::Archive {
                dest: expected.clone()
            }
        );
        assert!(plan.requires_archive());

        let dest = archive_run(&out, "issue-2")
            .expect("archive")
            .expect("archived");
        assert_eq!(dest, expected);
        assert!(
            !out.join("issue-2").exists(),
            "run dir must be moved, not copied"
        );
        assert!(dest.join("status.txt").is_file());

        // A re-dispatch wrote a new run; the next archive is run-2.
        write_run(&out, "issue-2", "status: VERIFIED\nagent_secs: 3600\n");
        let dest2 = archive_run(&out, "issue-2").expect("archive").unwrap();
        assert_eq!(dest2, out.join("archive/issue-2/run-2"));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn archive_preserves_everything_in_the_run_dir_not_just_the_status() {
        let out = temp_out();
        let dir = out.join("issue-3");
        fs::create_dir_all(&dir).expect("run dir");
        fs::write(dir.join("status.txt"), "status: TIMEOUT\nagent_secs: 90\n").expect("write");
        fs::write(dir.join("changes.patch"), "diff --git a/a b/a\n").expect("write");
        let dest = archive_run(&out, "issue-3").expect("archive").unwrap();
        assert!(dest.join("status.txt").is_file());
        assert!(dest.join("changes.patch").is_file());
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn move_to_archive_refuses_to_overwrite_an_existing_destination() {
        let out = temp_out();
        write_run(&out, "issue-4", "status: VERIFIED\nagent_secs: 90\n");
        let dest = out.join("archive/issue-4/run-1");
        fs::create_dir_all(&dest).expect("dest");
        fs::write(dest.join("status.txt"), "status: TIMEOUT\nagent_secs: 90\n").expect("write");
        let error = move_to_archive(&out.join("issue-4"), &dest).expect_err("must refuse");
        assert!(error.contains("already exists"), "{error}");
        // The run dir is untouched: the re-dispatch must stop, and the
        // pre-existing archive is unchanged.
        assert!(out.join("issue-4/status.txt").is_file());
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn archive_run_steers_around_an_existing_archive_entry() {
        // The plan numbers successive runs, so a pre-existing archive entry
        // is never the destination: run-1 taken by hand, the tool archives
        // to run-2 instead of refusing.
        let out = temp_out();
        write_run(&out, "issue-6", "status: VERIFIED\nagent_secs: 90\n");
        fs::create_dir_all(out.join("archive/issue-6/run-1")).expect("dest");
        let dest = archive_run(&out, "issue-6").expect("archive").unwrap();
        assert_eq!(dest, out.join("archive/issue-6/run-2"));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn archive_run_on_a_fresh_dir_creates_nothing() {
        let out = temp_out();
        assert_eq!(archive_run(&out, "issue-5").expect("archive"), None);
        assert!(!out.join(ARCHIVE_DIR).exists());
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn issue_names_that_escape_the_out_dir_are_refused() {
        let out = temp_out();
        for issue in ["", ".", "..", "a/b", "a\\b"] {
            let error = plan_redispatch(&out, issue).expect_err("must refuse");
            assert!(
                error.contains("safe single path component"),
                "{issue:?}: {error}"
            );
        }
        let reserved = plan_redispatch(&out, ARCHIVE_DIR).expect_err("must refuse");
        assert!(reserved.contains("reserved"), "{reserved}");
        let _ = fs::remove_dir_all(&out);
    }
}
