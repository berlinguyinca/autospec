//! Reservation budget and timeout handling (issue #3690).
//!
//! The runner's fixed 45-minute LIMIT killed every agent run inside an
//! 8-hour Slurm reservation, discarding the patch it had been building. The
//! four properties under test are the issue's four fixes: the budget is
//! derived from the reservation walltime, a budget that does not fit is
//! refused, a timed-out run's building patch is preserved, and a timeout with
//! no output is reported distinctly from an ordinary silent failure.

use std::fs;
use std::path::Path;

use autospec_core::failure_signatures::{
    analyze, scan_runs_dir, NO_OUTPUT_SIGNATURE, TIMEOUT_NO_OUTPUT_SIGNATURE,
};
use autospec_core::failure_signatures::{RunOutcome, RunRecord};
use autospec_core::reservation_budget::{
    classify_timeout_disposition, derive_agent_budget, parse_walltime, validate_budget,
    TimeoutDisposition,
};

fn temp_dir(label: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "autospec-reservation-budget-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir is created");
    path
}

#[test]
fn an_eight_hour_reservation_hosts_a_budget_far_past_the_old_forty_five_minutes() {
    let budget = derive_agent_budget(parse_walltime("8:00:00").unwrap()).unwrap();
    assert!(
        budget.budget_secs > 2700,
        "the old fixed 45-minute LIMIT was the bug; the derived budget must beat it, got {}s",
        budget.budget_secs
    );
    assert_eq!(budget.walltime_secs, 28800);
}

#[test]
fn a_budget_larger_than_its_reservation_is_refused() {
    // The old LIMIT (45 minutes) inside a 45-minute reservation: the run
    // would be killed before teardown even started.
    let error = validate_budget(45 * 60, 45 * 60).unwrap_err();
    assert!(error.to_string().contains("does not fit"));
    // The walltime minus overhead still fits.
    assert!(validate_budget(45 * 60, 45 * 60 - 600).is_ok());
}

#[test]
fn a_reservation_too_small_for_a_useful_run_is_refused_not_shrunk() {
    // 15 minutes minus overhead leaves under the minimum useful budget.
    assert!(derive_agent_budget(15 * 60).is_err());
}

#[test]
fn a_timed_out_runs_building_patch_is_never_discarded() {
    assert_eq!(
        classify_timeout_disposition(true),
        TimeoutDisposition::PreservePatch
    );
    assert!(!classify_timeout_disposition(false).preserves_patch());
}

#[test]
fn a_silent_timeout_is_reported_distinctly_from_a_silent_failure() {
    let runs = vec![
        RunRecord::new("run-1", RunOutcome::Timeout, ""),
        RunRecord::new("run-2", RunOutcome::Failed, "   \n"),
        RunRecord::new("run-3", RunOutcome::Completed, ""),
    ];
    let report = analyze(".autospec/runs", &runs, 200, 5.0, 10);
    assert_eq!(report.failed_runs, 2, "a timeout is not a success");
    assert_eq!(
        report.unsigned_runs, 2,
        "both silent buckets stay in the arithmetic"
    );
    let bucket = |signature: &str| -> usize {
        report
            .signatures
            .iter()
            .find(|entry| entry.signature == signature)
            .map(|entry| entry.count)
            .unwrap_or(0)
    };
    assert_eq!(
        bucket(TIMEOUT_NO_OUTPUT_SIGNATURE),
        1,
        "the timeout bucket is separate from the no-output bucket"
    );
    assert_eq!(bucket(NO_OUTPUT_SIGNATURE), 1);
    assert_eq!(
        RunRecord::new("run-1", RunOutcome::Timeout, "").signature(),
        TIMEOUT_NO_OUTPUT_SIGNATURE
    );
}

#[test]
fn a_timeout_that_still_wrote_stderr_reports_its_signature_line() {
    let record = RunRecord::new(
        "run-1",
        RunOutcome::Timeout,
        "killed: budget exhausted at step 42\n",
    );
    assert_eq!(record.signature(), "killed: budget exhausted at step <n>");
    assert_ne!(record.signature(), TIMEOUT_NO_OUTPUT_SIGNATURE);
}

/// Writes a run directory: optional status file plus a stderr log.
fn write_run(root: &Path, id: &str, status: Option<&str>, stderr: Option<&str>) {
    let dir = root.join(id);
    fs::create_dir_all(&dir).expect("run dir");
    if let Some(status) = status {
        fs::write(dir.join("status.json"), status).expect("status");
    }
    if let Some(stderr) = stderr {
        fs::write(dir.join("stderr.log"), stderr).expect("stderr");
    }
}

#[test]
fn the_scanner_maps_timeout_status_and_exit_codes_to_the_timeout_outcome() {
    let root = temp_dir("scan");
    write_run(&root, "run-1", Some("{\"status\":\"timeout\"}"), None);
    write_run(
        &root,
        "run-2",
        Some("{\"status\":\"timed_out\"}"),
        Some("budget\n"),
    );
    write_run(&root, "run-3", Some("{\"exit_code\":124}"), None);
    write_run(&root, "run-4", Some("{\"exit_code\":281}"), None);
    write_run(&root, "run-5", Some("{\"exit_code\":137}"), None);
    write_run(&root, "run-6", Some("{\"status\":\"failed\"}"), None);

    let runs = scan_runs_dir(&root).expect("scan succeeds");
    assert_eq!(runs.len(), 6);
    let by_id = |id: &str| runs.iter().find(|run| run.id == id).expect("run found");
    for id in ["run-1", "run-2", "run-3", "run-4"] {
        assert_eq!(by_id(id).outcome, RunOutcome::Timeout, "{id} is a timeout");
    }
    assert_eq!(
        by_id("run-5").outcome,
        RunOutcome::Failed,
        "a 137 kill (OOM/SIGKILL) is not a budget timeout"
    );
    assert_eq!(by_id("run-6").outcome, RunOutcome::Failed);

    // The silent 124/281 kills land in the timeout bucket, not no-output.
    let report = analyze(root.display().to_string().as_str(), &runs, 200, 5.0, 10);
    let bucket = report
        .signatures
        .iter()
        .find(|entry| entry.signature == TIMEOUT_NO_OUTPUT_SIGNATURE)
        .map(|entry| entry.count)
        .unwrap_or(0);
    assert_eq!(bucket, 3, "run-1, run-3 and run-4 are silent timeouts");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_plain_status_file_named_timeout_is_understood() {
    let root = temp_dir("plain-status");
    let dir = root.join("run-1");
    fs::create_dir_all(&dir).expect("run dir");
    fs::write(dir.join("status"), "timed_out\n").expect("status");
    let runs = scan_runs_dir(&root).expect("scan succeeds");
    assert_eq!(runs[0].outcome, RunOutcome::Timeout);
    assert_eq!(runs[0].signature(), TIMEOUT_NO_OUTPUT_SIGNATURE);
    let _ = fs::remove_dir_all(&root);
}
