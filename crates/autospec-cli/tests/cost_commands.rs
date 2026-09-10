//! End-to-end tests for `autospec cost` (issue #3940).
//!
//! The command must report GPU-hours by terminal status with shares, split
//! rework from productive hours, surface known-defect costs, flag buckets
//! above the share threshold, and reject bad arguments with exit 2.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

fn autospec_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn out_dir_with(records: &[(&str, &[(&str, &str)])]) -> PathBuf {
    let root = temp_dir("cost");
    for (issue, lines) in records {
        let dir = root.join(issue);
        fs::create_dir_all(&dir).expect("run dir");
        let body: String = lines
            .iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.join("status.txt"), body + "\n").expect("status.txt");
    }
    root
}

fn run_cost(dir: &Path, args: &[&str]) -> Output {
    autospec_binary()
        .current_dir(dir)
        .arg("cost")
        .args(args)
        .output()
        .expect("run autospec")
}

#[test]
fn reports_text_and_json() {
    let out = out_dir_with(&[
        ("issue-1", &[("status", "VERIFIED"), ("agent_secs", "3600")]),
        (
            "issue-2",
            &[
                ("status", "TIMEOUT"),
                ("agent_secs", "7200"),
                ("worker", "w-2"),
            ],
        ),
    ]);
    let out_arg = out.to_str().expect("utf-8 path");
    let cwd = temp_dir("cost-cwd");
    let output = run_cost(&cwd, &["--out-dir", out_arg]);
    assert!(output.status.success(), "cost should exit 0");
    let stdout = std::str::from_utf8(&output.stdout).expect("utf-8");
    assert!(stdout.contains("VERIFIED"), "text: {stdout}");
    assert!(stdout.contains("TIMEOUT"), "text: {stdout}");
    assert!(stdout.contains("3.0"), "text: {stdout}");

    let json_output = run_cost(&cwd, &["--out-dir", out_arg, "--json"]);
    assert!(json_output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json_output.stdout).expect("json");
    assert_eq!(value["command"], "cost");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["cumulative"]["records"], 2);
    assert_eq!(value["cumulative"]["total_gpu_hours"], 3.0);
    assert!(value["window"].is_null());
    // TIMEOUT is a known defect status (#3936).
    let defects = &value["cumulative"]["defects"];
    assert!(defects.as_array().is_some_and(|d| {
        d.iter()
            .any(|d| d["issue"] == "#3936" && d["gpu_hours"] == 2.0)
    }));
}

#[test]
fn bad_arguments_exit_2() {
    let cwd = temp_dir("cost-bad-args");
    for args in [
        vec!["--threshold-percent", "200"],
        vec!["--threshold-percent", "abc"],
        vec!["--since", "yesterday"],
        vec!["--no-such-flag"],
    ] {
        let output = run_cost(&cwd, &args);
        assert!(
            !output.status.success() && output.status.code() == Some(2),
            "{args:?} should exit 2, got {:?}",
            output.status.code()
        );
    }
}

#[test]
fn missing_directory_is_an_empty_report() {
    let cwd = temp_dir("cost-empty");
    let output = run_cost(&cwd, &["--out-dir", "does-not-exist"]);
    assert!(output.status.success(), "{:?}", output.status.code());
    let stdout = std::str::from_utf8(&output.stdout).expect("utf-8");
    assert!(stdout.contains("nothing to cost"), "text: {stdout}");
}

/// #3983: the per-issue ranking shows the latest run outcome and the trailing
/// zero-output streak beside the cumulative hours, so hours are never mistaken
/// for a hold.
#[test]
fn per_issue_ranking_shows_latest_outcome_beside_hours() {
    let out = out_dir_with(&[
        (
            "issue-3192-attempt-1",
            &[
                ("status", "TIMEOUT"),
                ("agent_secs", "7200"),
                ("changed_files", "0"),
                ("finished_at", "2026-09-01T00:00:00Z"),
            ],
        ),
        (
            "issue-3192-attempt-2",
            &[
                ("status", "TIMEOUT"),
                ("agent_secs", "7200"),
                ("changed_files", "0"),
                ("finished_at", "2026-09-01T02:00:00Z"),
            ],
        ),
        (
            "issue-3192-attempt-3",
            &[
                ("status", "VERIFIED"),
                ("agent_secs", "1800"),
                ("changed_files", "6"),
                ("finished_at", "2026-09-01T04:00:00Z"),
            ],
        ),
        (
            "issue-3805-attempt-1",
            &[
                ("status", "NO-OUTPUT"),
                ("agent_secs", "900"),
                ("changed_files", "0"),
                ("finished_at", "2026-09-01T02:00:00Z"),
            ],
        ),
        (
            "issue-3805-attempt-2",
            &[
                ("status", "NO-OUTPUT"),
                ("agent_secs", "900"),
                ("changed_files", "0"),
                ("finished_at", "2026-09-01T03:00:00Z"),
            ],
        ),
    ]);
    let out_arg = out.to_str().expect("utf-8 path");
    let cwd = temp_dir("cost-cwd");

    let stdout = std::str::from_utf8(&run_cost(&cwd, &["--out-dir", out_arg]).stdout)
        .expect("utf-8")
        .to_string();
    let held = stdout
        .lines()
        .find(|line| line.starts_with("    issue-3805 "))
        .unwrap_or_else(|| panic!("no per-issue row for issue-3805 in:\n{stdout}"));
    assert!(held.contains("NO-OUTPUT"), "row: {held}");
    assert!(held.contains("zero-output"), "row: {held}");
    assert!(held.contains("review"), "row: {held}");
    assert!(
        stdout.contains("issues by cumulative GPU-hours"),
        "{stdout}"
    );

    let json = std::str::from_utf8(&run_cost(&cwd, &["--out-dir", out_arg, "--json"]).stdout)
        .expect("utf-8")
        .to_string();
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    let rows = value["cumulative"]["by_issue"]
        .as_array()
        .expect("by_issue array");
    assert_eq!(rows.len(), 2);
    let recovered = rows
        .iter()
        .find(|row| row["issue"] == "issue-3192")
        .expect("issue-3192 row");
    assert_eq!(recovered["latest_status"], "VERIFIED");
    assert_eq!(recovered["latest_outcome"], "produced");
    assert_eq!(recovered["trailing_zero_output_streak"], 0);
    assert_eq!(recovered["zero_output_runs"], 2);
    assert_eq!(recovered["dispatchable"], true);
}
