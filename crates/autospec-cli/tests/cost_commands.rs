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

/// #3940 correction: the pre-redispatch archive is a command, not an
/// operator habit. The re-dispatching runner calls `cost archive <issue>`
/// before it removes the run directory, and the report must then count
/// every run of the re-dispatched issue.
#[test]
fn archive_subcommand_archives_before_a_redispatch() {
    let out = out_dir_with(&[(
        "issue-1",
        &[("status", "NEW-TEST-FAILURES"), ("agent_secs", "7200")],
    )]);
    let out_arg = out.to_str().expect("utf-8 path");
    let cwd = temp_dir("cost-archive-cwd");

    let output = run_cost(&cwd, &["archive", "--out-dir", out_arg, "issue-1"]);
    assert!(output.status.success(), "archive should exit 0");
    let stdout = std::str::from_utf8(&output.stdout).expect("utf-8");
    assert!(
        stdout.contains("archived issue-1 -> ") && stdout.contains("archive/issue-1/run-1"),
        "text: {stdout}"
    );
    assert!(
        !out.join("issue-1").exists(),
        "run dir must be moved, not copied"
    );
    assert!(out.join("archive/issue-1/run-1/status.txt").is_file());

    // The re-dispatch wrote a new run; archiving again is run-2.
    let run2 = out.join("issue-1");
    fs::create_dir_all(&run2).expect("run dir");
    fs::write(
        run2.join("status.txt"),
        "status: VERIFIED\nagent_secs: 3600\n",
    )
    .expect("write");
    let output = run_cost(&cwd, &["archive", "--out-dir", out_arg, "issue-1"]);
    assert!(output.status.success(), "archive should exit 0");
    let stdout = std::str::from_utf8(&output.stdout).expect("utf-8");
    assert!(stdout.contains("archive/issue-1/run-2"), "text: {stdout}");

    // The report now counts both runs of the re-dispatched issue: 3h, not
    // the 1h the last run alone would say.
    let json_output = run_cost(&cwd, &["--out-dir", out_arg, "--json"]);
    assert!(json_output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json_output.stdout).expect("json");
    assert_eq!(value["cumulative"]["records"], 2);
    assert_eq!(value["cumulative"]["total_gpu_hours"], 3.0);
    // Both runs are archived copies now: run 1 from the first archive,
    // run 2 from the second — the live directory holds the next run, not
    // a record.
    assert_eq!(value["archived_records"], 2);

    // An issue with no run directory is fresh: exit 0, nothing moved.
    let output = run_cost(&cwd, &["archive", "--out-dir", out_arg, "issue-9"]);
    assert!(output.status.success());
    let stdout = std::str::from_utf8(&output.stdout).expect("utf-8");
    assert!(stdout.contains("fresh"), "text: {stdout}");
}

#[test]
fn archive_subcommand_rejects_bad_arguments() {
    let cwd = temp_dir("cost-archive-bad-args");
    for args in [
        vec!["archive"],                              // no issue
        vec!["archive", "issue-1", "issue-2"],        // two issues
        vec!["archive", "../escape"],                 // path traversal
        vec!["archive", "--no-such-flag", "issue-1"], // unknown flag
    ] {
        let output = autospec_binary()
            .current_dir(&cwd)
            .arg("cost")
            .args(&args)
            .output()
            .expect("run autospec");
        assert!(
            output.status.code() == Some(2),
            "{args:?} should exit 2, got {:?}",
            output.status.code()
        );
    }
}
