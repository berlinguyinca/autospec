//! Failure signatures over a rolling window of fleet agent runs (issue #3735).
//!
//! The three properties under test are the ones the issue asks for: counts
//! carry their denominator, a repeated signature flags itself against the
//! threshold, and runs that produced nothing are counted instead of dropped.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::failure_signatures::{
    analyze, is_slurm_noise, last_meaningful_line, normalize_signature_line, scan_runs_dir,
    FailureSignatureReport, RunOutcome, RunRecord, SignatureCount, NO_OUTPUT_SIGNATURE,
    NO_STATUS_SIGNATURE,
};

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "autospec-failure-signatures-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temp dir is created");
    path
}

fn failed_run(id: &str, stderr: &str) -> RunRecord {
    RunRecord::new(id, RunOutcome::Failed, stderr)
}

fn completed_run(id: &str) -> RunRecord {
    RunRecord::new(id, RunOutcome::Completed, "")
}

/// Repeats `signature` across `count` of `total` runs, newest first.
fn window(signature: &str, failing: usize, total: usize) -> Vec<RunRecord> {
    let mut runs: Vec<RunRecord> = (0..failing)
        .map(|index| failed_run(&format!("run-{index}"), &format!("{signature}\n")))
        .collect();
    runs.extend((failing..total).map(|index| completed_run(&format!("run-{index}"))));
    runs
}

fn find<'a>(report: &'a FailureSignatureReport, signature: &str) -> Option<&'a SignatureCount> {
    report
        .signatures
        .iter()
        .find(|entry| entry.signature == signature)
}

#[test]
fn signature_masks_paths_and_numbers_but_keeps_the_message() {
    let raw =
        "/scratch/gw-as-3735-22766006/repo/src/main.c:42:10: error: cannot allocate 1048576 bytes";
    let signature = normalize_signature_line(raw);
    assert!(
        signature.contains("<path>/main.c"),
        "directory components must be masked, got {signature}"
    );
    assert!(
        !signature.contains("3735") && !signature.contains("1048576"),
        "numbers must be masked, got {signature}"
    );
    assert!(
        signature.contains("cannot allocate"),
        "the message identifies the defect and must survive, got {signature}"
    );

    // Same defect at a different path, line and size groups with itself.
    let other = normalize_signature_line(
        "/var/lib/slurm/job-99/src/main.c:7:3: error: cannot allocate 2097152 bytes",
    );
    assert_eq!(signature, other, "one defect must produce one signature");
}

#[test]
fn signature_masks_addresses_uuids_and_dates() {
    let signature = normalize_signature_line(
        "segfault at 0x7ffd12345678 ip 3f2a8b1c-0d9e-4a5b-8c7d-6e5f4a3b2c1d on 2026-07-29",
    );
    assert!(signature.contains("<addr>"), "got {signature}");
    assert!(signature.contains("<uuid>"), "got {signature}");
    assert!(signature.contains("<date>"), "got {signature}");
}

#[test]
fn slurm_noise_never_becomes_the_signature() {
    let noise =
        "slurmstepd: error: *** JOB 1204 ON cuda-node-7 CANCELLED AT 2026-07-29T03:11:42 ***";
    assert!(is_slurm_noise(noise), "slurm step daemon line is noise");
    assert!(is_slurm_noise("srun: error: Node 12 failure"));

    let stderr = "Traceback (most recent call last):\n  File \"train.py\", line 3, in <module>\nMemoryError\nslurmstepd: error: *** JOB 1204 ON cuda-node-7 CANCELLED ***\n";
    assert_eq!(last_meaningful_line(stderr), Some("MemoryError"));
    assert_eq!(
        RunRecord::new("run-1", RunOutcome::Failed, stderr).signature(),
        "MemoryError"
    );
}

#[test]
fn counts_are_reported_against_the_window_denominator() {
    let report = analyze(
        ".autospec/runs",
        &window("oom-killer: <path>/train.py:<n> killed process", 62, 190),
        200,
        5.0,
        10,
    );
    assert_eq!(report.runs, 190, "the denominator is the window size");
    assert_eq!(report.failed_runs, 62);
    let entry = find(&report, "oom-killer: <path>/train.py:<n> killed process")
        .expect("the repeated signature is listed");
    assert_eq!(entry.count, 62);
    assert!(
        (entry.share_percent - 32.6).abs() < 0.1,
        "62/190 is 32.6%, got {}",
        entry.share_percent
    );
    assert!(entry.systemic, "32.6% of runs is above the 5% threshold");
    assert!(report.to_text().contains("62 of 190 runs failed"));
}

#[test]
fn a_single_failure_is_not_systemic() {
    let report = analyze(
        ".autospec/runs",
        &window("transient: connection reset by peer", 1, 190),
        200,
        5.0,
        10,
    );
    let entry = find(&report, "transient: connection reset by peer").expect("listed");
    assert!(!entry.systemic, "1/190 is below the threshold");
    assert!(!report.systemic);
}

#[test]
fn a_boundary_count_at_the_threshold_is_not_systemic() {
    // Exactly 5% of 200 runs is not "more than 5%".
    let report = analyze(
        ".autospec/runs",
        &window("disk quota exceeded", 10, 200),
        200,
        5.0,
        10,
    );
    assert!(
        !find(&report, "disk quota exceeded")
            .expect("listed")
            .systemic
    );
    let report = analyze(
        ".autospec/runs",
        &window("disk quota exceeded", 11, 200),
        200,
        5.0,
        10,
    );
    assert!(
        find(&report, "disk quota exceeded")
            .expect("listed")
            .systemic
    );
}

#[test]
fn no_output_runs_are_their_own_bucket_and_stay_in_the_denominator() {
    let mut runs: Vec<RunRecord> = vec![
        failed_run("run-1", ""),
        failed_run("run-2", "   \n"),
        failed_run(
            "run-3",
            "slurmstepd: error: *** JOB 1 ON node CANCELLED ***\n",
        ),
    ];
    runs.extend((3..20).map(|index| completed_run(&format!("run-{index}"))));

    let report = analyze(".autospec/runs", &runs, 200, 5.0, 10);
    assert_eq!(
        report.runs, 20,
        "silent runs must not shrink the denominator"
    );
    assert_eq!(report.unsigned_runs, 3);
    let entry = find(&report, NO_OUTPUT_SIGNATURE).expect("no-output bucket is listed");
    assert_eq!(entry.count, 3);
    assert!(entry.systemic, "3/20 is 15%, above the threshold");
    assert!(report.to_text().contains(NO_OUTPUT_SIGNATURE));
}

#[test]
fn runs_without_a_status_file_are_counted_not_dropped() {
    let runs = vec![
        RunRecord::new(
            "run-1",
            RunOutcome::Missing,
            "killed before writing anything",
        ),
        RunRecord::new("run-2", RunOutcome::Missing, ""),
        completed_run("run-3"),
    ];
    let report = analyze(".autospec/runs", &runs, 200, 5.0, 10);
    assert_eq!(report.runs, 3);
    assert_eq!(
        report.failed_runs, 2,
        "an absent status file is not a success"
    );
    let entry = find(&report, NO_STATUS_SIGNATURE).expect("bucket is listed");
    assert_eq!(entry.count, 2);
    assert!(entry.systemic, "2/3 is 66.7%");
}

#[test]
fn the_window_is_rolling_and_drops_the_oldest_runs() {
    // Ten old runs share a signature; the window of five must not see them.
    let mut runs: Vec<RunRecord> = (0..5)
        .map(|index| completed_run(&format!("new-{index}")))
        .collect();
    runs.extend(
        (0..10).map(|index| failed_run(&format!("old-{index}"), "removed dependency: module foo")),
    );
    let report = analyze(".autospec/runs", &runs, 5, 5.0, 10);
    assert_eq!(report.runs, 5);
    assert_eq!(report.failed_runs, 0);
    assert!(find(&report, "removed dependency: module foo").is_none());
}

#[test]
fn signatures_are_ordered_by_count_and_truncated_to_top() {
    let mut runs: Vec<RunRecord> = Vec::new();
    runs.extend((0..5).map(|i| failed_run(&format!("a{i}"), "signature alpha")));
    runs.extend((0..3).map(|i| failed_run(&format!("b{i}"), "signature beta")));
    runs.extend((0..1).map(|i| failed_run(&format!("c{i}"), "signature gamma")));
    let report = analyze(".autospec/runs", &runs, 200, 5.0, 2);
    let listed: Vec<&str> = report
        .signatures
        .iter()
        .map(|e| e.signature.as_str())
        .collect();
    assert_eq!(listed, vec!["signature alpha", "signature beta"]);
    assert_eq!(report.truncated, 1);
}

#[test]
fn json_report_carries_the_denominator_and_the_systemic_flag() {
    let report = analyze(
        ".autospec/runs",
        &window("npm ERR! code ELIFECYCLE", 30, 100),
        100,
        5.0,
        10,
    );
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).expect("valid JSON");
    assert_eq!(json["runs"], 100);
    assert_eq!(json["failed_runs"], 30);
    assert_eq!(json["threshold_percent"], 5.0);
    assert_eq!(json["systemic"], true);
    assert_eq!(json["signatures"][0]["count"], 30);
    assert_eq!(json["signatures"][0]["systemic"], true);
    assert_eq!(
        json["signatures"][0]["signature"],
        "npm ERR! code ELIFECYCLE"
    );
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
fn scanning_reads_status_files_and_stderr_logs_per_run_directory() {
    let root = temp_dir("scan");
    write_run(
        &root,
        "run-1",
        Some("{\"status\":\"failed\"}"),
        Some("boom: no space left on device\nslurmstepd: error: *** JOB 1 ***\n"),
    );
    write_run(
        &root,
        "run-2",
        Some("{\"exit_code\":137}"),
        Some("killed\n"),
    );
    write_run(&root, "run-3", Some("{\"status\":\"completed\"}"), None);
    write_run(&root, "run-4", None, Some("nothing was recorded\n"));

    let runs = scan_runs_dir(&root).expect("scan succeeds");
    assert_eq!(runs.len(), 4);
    let by_id = |id: &str| runs.iter().find(|run| run.id == id).expect("run found");
    assert_eq!(by_id("run-1").outcome, RunOutcome::Failed);
    assert_eq!(by_id("run-1").signature(), "boom: no space left on device");
    assert_eq!(
        by_id("run-2").outcome,
        RunOutcome::Failed,
        "a non-zero exit code is a failure"
    );
    assert_eq!(by_id("run-3").outcome, RunOutcome::Completed);
    assert_eq!(
        by_id("run-4").outcome,
        RunOutcome::Missing,
        "a run that never wrote a status file is its own bucket"
    );

    let report = analyze(root.display().to_string().as_str(), &runs, 200, 5.0, 10);
    assert_eq!(report.runs, 4);
    assert_eq!(
        report.failed_runs, 3,
        "failed, killed and status-less all count"
    );
    assert_eq!(report.unsigned_runs, 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_plain_status_file_is_understood() {
    let root = temp_dir("plain-status");
    let dir = root.join("run-1");
    fs::create_dir_all(&dir).expect("run dir");
    fs::write(dir.join("status"), "killed\n").expect("status");
    fs::write(dir.join("stderr.log"), "CUDA out of memory\n").expect("stderr");
    let runs = scan_runs_dir(&root).expect("scan succeeds");
    assert_eq!(runs[0].outcome, RunOutcome::Failed);
    assert_eq!(runs[0].signature(), "CUDA out of memory");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_missing_runs_directory_reports_no_runs_instead_of_failing() {
    let missing = temp_dir("missing").join("does-not-exist");
    let runs = scan_runs_dir(&missing).expect("an empty fleet is reportable");
    assert!(runs.is_empty());
    let report = analyze(".autospec/runs", &runs, 200, 5.0, 10);
    assert!(report.is_empty());
    assert!(!report.systemic);
    assert!(report.to_text().contains("no agent runs found"));
}
