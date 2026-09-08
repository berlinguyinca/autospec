//! End-to-end tests for `autospec doctor failures` (issue #3735).
//!
//! The command must report the counts with the denominator, flag the signature
//! that crossed the threshold, count runs that produced nothing, and carry the
//! verdict in its exit code.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

fn autospec_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

/// A runs directory with `failing` runs sharing one crash signature, `silent`
/// failed runs that wrote no stderr, and `ok_runs` clean runs.
fn runs_dir_with(failing: usize, silent: usize, ok_runs: usize) -> PathBuf {
    let root = temp_dir("runs");
    for index in 0..failing {
        let run = root.join(format!("run-{index:03}"));
        fs::create_dir_all(&run).expect("run dir");
        fs::write(run.join("status.json"), "{\"status\":\"failed\"}").expect("status");
        fs::write(
            run.join("stderr.log"),
            format!(
                "reading package list...\ngcc -c /scratch/node-{index}/src/main.c:42: error: cannot allocate 1048576 bytes\nslurmstepd: error: *** JOB {index} ON cuda-node-{index} CANCELLED ***\n"),
        )
        .expect("stderr");
    }
    for index in 0..silent {
        let run = root.join(format!("silent-{index:03}"));
        fs::create_dir_all(&run).expect("run dir");
        fs::write(run.join("status.json"), "{\"exit_code\":137}").expect("status");
    }
    for index in 0..ok_runs {
        let run = root.join(format!("ok-{index:03}"));
        fs::create_dir_all(&run).expect("run dir");
        fs::write(run.join("status.json"), "{\"status\":\"completed\"}").expect("status");
    }
    root
}

fn failures(runs: &Path, args: &[&str]) -> Output {
    autospec_binary()
        .arg("doctor")
        .arg("failures")
        .arg("--runs-dir")
        .arg(runs)
        .args(args)
        .output()
        .expect("autospec doctor failures runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn single_command_reports_counts_with_the_denominator() {
    let runs = runs_dir_with(62, 0, 128);
    let output = failures(&runs, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    let first_line = text.lines().next().expect("a summary line");
    assert!(
        first_line.contains("62 of 190 runs failed"),
        "counts must travel with the denominator, got: {first_line}"
    );
    assert!(first_line.contains("32.6%"), "got: {first_line}");
    assert!(
        text.contains("<path>/main.c:<n>: error: cannot allocate <n> bytes"),
        "numbers and paths are masked so one defect yields one signature, got:\n{text}"
    );
    let _ = fs::remove_dir_all(&runs);
}

#[test]
fn a_crossing_signature_is_surfaced_without_being_asked_for() {
    let runs = runs_dir_with(62, 0, 128);
    let output = failures(&runs, &[]);
    let text = stdout(&output);
    assert!(
        text.contains("SYSTEMIC"),
        "the crossing signature is marked in the report, got:\n{text}"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "a systemic signature exits 1 so a monitor can detect it"
    );
    let _ = fs::remove_dir_all(&runs);
}

#[test]
fn a_clean_window_exits_zero_and_names_no_signature() {
    let runs = runs_dir_with(0, 0, 50);
    let output = failures(&runs, &[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        !stdout(&output).contains("SYSTEMIC"),
        "got:\n{}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("0 systemic signature(s) above the 5.0% threshold"),
        "got:\n{}",
        stdout(&output)
    );
    let _ = fs::remove_dir_all(&runs);
}

#[test]
fn runs_that_produced_no_output_are_counted_not_dropped() {
    // 20 runs: 4 killed with no stderr, 16 clean. 4/20 is 20% — systemic.
    let runs = runs_dir_with(0, 4, 16);
    let output = failures(&runs, &[]);
    let text = stdout(&output);
    assert!(
        text.contains("<no output>"),
        "silent runs get their own bucket, got:\n{text}"
    );
    assert!(
        text.contains("4 of them produced no output"),
        "got:\n{text}"
    );
    assert!(
        text.contains("4 of 20 runs failed"),
        "the denominator still covers every run, got:\n{text}"
    );
    assert_eq!(output.status.code(), Some(1));
    let _ = fs::remove_dir_all(&runs);
}

#[test]
fn runs_without_a_status_file_get_their_own_bucket() {
    let runs = runs_dir_with(0, 0, 10);
    for index in 0..3 {
        let run = runs.join(format!("died-{index:03}"));
        fs::create_dir_all(&run).expect("run dir");
    }
    let output = failures(&runs, &[]);
    let text = stdout(&output);
    assert!(
        text.contains("<no status file>"),
        "a run killed before recording anything must appear, got:\n{text}"
    );
    assert!(text.contains("3 of 13 runs failed"), "got:\n{text}");
    let _ = fs::remove_dir_all(&runs);
}

#[test]
fn json_output_carries_counts_denominator_and_flags() {
    let runs = runs_dir_with(62, 3, 125);
    let output = failures(&runs, &["--json"]);
    let payload: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("report is valid JSON");
    assert_eq!(payload["runs"], 190);
    assert_eq!(payload["failed_runs"], 65);
    assert_eq!(payload["unsigned_runs"], 3);
    assert_eq!(payload["threshold_percent"], 5.0);
    assert_eq!(payload["systemic"], true);
    let top = &payload["signatures"][0];
    assert_eq!(top["count"], 62);
    assert_eq!(top["systemic"], true);
    assert!((top["share_percent"].as_f64().unwrap() - 32.6).abs() < 0.1);
    assert_eq!(payload["signatures"][1]["signature"], "<no output>");
    let _ = fs::remove_dir_all(&runs);
}

#[test]
fn the_window_threshold_and_top_are_configurable() {
    let runs = runs_dir_with(30, 0, 70);
    // Raising the threshold above 30% demotes the signature.
    let output = failures(&runs, &["--threshold-percent", "50", "--json"]);
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["systemic"], false);
    assert_eq!(output.status.code(), Some(0));

    // A window of 20 runs excludes the older ones entirely.
    let output = failures(&runs, &["--last", "20", "--json"]);
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["runs"], 20);
    assert_eq!(payload["failed_runs"], 0);

    // --top truncates the listing and says how much it dropped.
    let output = failures(&runs, &["--top", "1", "--json"]);
    let payload: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(payload["signatures"].as_array().expect("array").len(), 1);
    assert_eq!(payload["truncated"], 0);
    let _ = fs::remove_dir_all(&runs);
}

#[test]
fn a_missing_runs_directory_is_reported_not_an_error() {
    let missing = temp_dir("absent").join("no-such-runs");
    let output = failures(&missing, &[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("no agent runs found"));
}

#[test]
fn bad_arguments_exit_two_with_a_readable_message() {
    let runs = runs_dir_with(1, 0, 1);
    for (args, expected) in [
        (vec!["--last", "0"], "--last"),
        (vec!["--threshold-percent", "abc"], "--threshold-percent"),
        (vec!["--threshold-percent", "0"], "--threshold-percent"),
        (vec!["--nonsense"], "unknown"),
        (vec!["--runs-dir"], "requires a value"),
    ] {
        let output = failures(&runs, &args);
        assert_eq!(output.status.code(), Some(2), "args {args:?}");
        let message = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(message.contains(expected), "args {args:?}: {message}");
    }
    let _ = fs::remove_dir_all(&runs);
}

#[test]
fn help_lists_the_options_and_exit_codes() {
    let output = autospec_binary()
        .args(["doctor", "failures", "--help"])
        .output()
        .expect("help runs");
    let text = stdout(&output);
    for expected in [
        "--runs-dir",
        "--last",
        "--threshold-percent",
        "--top",
        "--json",
        "no status file",
    ] {
        assert!(text.contains(expected), "missing {expected} in:\n{text}");
    }
}
