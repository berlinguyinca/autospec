//! `autospec benchmark delivery`: the real binary over temp JSONL files.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

fn results_file(lines: &[&str]) -> PathBuf {
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "autospec-benchmark-delivery-{}-{serial}.jsonl",
        std::process::id()
    ));
    fs::write(&path, lines.join("\n")).expect("temp results file");
    path
}

fn delivery(path: &PathBuf, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["benchmark", "delivery", "--results"])
        .arg(path)
        .args(extra)
        .output()
        .expect("the autospec binary runs")
}

fn run_line(task: &str, arm: &str, secs: u32, success: bool) -> String {
    format!(
        r#"{{"task":"{task}","arm":"{arm}","time_to_verified_secs":{secs},"tokens":100,"retries":1,"disk_bytes":4096,"success":{success}}}"#
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("UTF-8 stdout")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("UTF-8 stderr")
}

#[test]
fn a_valid_results_file_prints_ten_benchmark_rows() {
    let lines = [
        run_line("trivial-fix", "baseline", 10, true),
        run_line("trivial-fix", "baseline", 20, true),
        run_line("trivial-fix", "baseline", 30, true),
        run_line("trivial-fix", "baseline", 40, true),
        run_line("trivial-fix", "pi-autospec", 8, true),
    ];
    let path = results_file(&lines.iter().map(String::as_str).collect::<Vec<_>>());

    let output = delivery(&path, &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    let rows: Vec<&str> = text.lines().filter(|line| line.starts_with("| ")).collect();
    assert_eq!(rows.len(), 10, "{text}");
    assert!(
        rows[0].starts_with(
            "| trivial-fix | 20 | 40 | 8 | 8 | 100 / 100 | 1 / 1 | 4096 / 4096 | 100% / 100% |"
        ),
        "{}",
        rows[0]
    );
    assert!(rows[1].contains("unmeasured"), "{}", rows[1]);
}

#[test]
fn only_a3_passing_prints_a3_pass_and_a1_not_run() {
    let path = results_file(&[r#"{"scenario":"A3","passed":true}"#]);

    let output = delivery(&path, &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.lines().any(|line| line == "A3 pass"), "{text}");
    assert!(text.lines().any(|line| line == "A1 not-run"), "{text}");
}

#[test]
fn any_failing_row_makes_a_scenario_fail() {
    let path = results_file(&[
        r#"{"scenario":"A5","passed":true}"#,
        r#"{"scenario":"A5","passed":false}"#,
    ]);

    let text = stdout(&delivery(&path, &[]));

    assert!(text.lines().any(|line| line == "A5 fail"), "{text}");
}

#[test]
fn a_malformed_line_exits_2_naming_its_line_number() {
    let good = run_line("crash", "baseline", 5, true);
    let path = results_file(&[
        &good,
        r#"{"scenario":"A1","passed":true}"#,
        "",
        "{not json",
        &good,
    ]);

    let output = delivery(&path, &[]);

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("line 4"), "{}", stderr(&output));
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
}

#[test]
fn a_row_of_the_wrong_shape_or_an_unknown_scenario_is_reported_not_skipped() {
    for (line, expected) in [
        (r#"{"task":"nope","arm":"baseline"}"#, "line 1"),
        (r#"{"scenario":"A10","passed":true}"#, "line 1"),
        (r#"{"scenario":"A2"}"#, "line 1"),
    ] {
        let path = results_file(&[line]);
        let output = delivery(&path, &[]);
        assert_eq!(output.status.code(), Some(2), "{line}");
        assert!(
            stderr(&output).contains(expected),
            "{line}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn json_output_carries_ten_tasks_and_nine_scenarios() {
    let path = results_file(&[
        &run_line("crash", "baseline", 5, true),
        r#"{"scenario":"A9","passed":true}"#,
    ]);

    let output = delivery(&path, &["--json"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("JSON stdout");
    assert_eq!(value["tasks"].as_array().expect("tasks").len(), 10);
    assert_eq!(value["scenarios"].as_array().expect("scenarios").len(), 9);
    assert_eq!(value["scenarios"][8]["status"], "pass");
    assert_eq!(value["scenarios"][0]["status"], "not-run");
    let crash = value["tasks"]
        .as_array()
        .expect("tasks")
        .iter()
        .find(|task| task["task"] == "crash")
        .expect("crash task");
    assert_eq!(crash["comparison"]["comparison"], "candidate-missing");
}

#[test]
fn missing_results_or_an_unreadable_file_exits_2() {
    let no_flag = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(["benchmark", "delivery"])
        .output()
        .expect("runs");
    assert_eq!(no_flag.status.code(), Some(2));
    assert!(stderr(&no_flag).contains("usage"), "{}", stderr(&no_flag));

    let missing = PathBuf::from("/nonexistent/autospec-benchmark-delivery.jsonl");
    let output = delivery(&missing, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("cannot read"),
        "{}",
        stderr(&output)
    );
}
