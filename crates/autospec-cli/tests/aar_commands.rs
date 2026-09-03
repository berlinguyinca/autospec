//! `autospec aar` command surface (AAR spec acceptance criteria 4 and 12).

use std::process::Command;

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique as temp_dir;

/// Filesystem-touching tests share one lock: these commands write into a
/// worktree, and running them concurrently makes failures order-dependent.
static FILESYSTEM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = autospec().args(args).output().expect("autospec runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

const BUGFIX: &[&str] = &[
    "--title",
    "Fix panic in the queue parser on empty specs",
    "--body",
    "The parser panics on an empty document; reproduce and fix.",
    "--path",
    "crates/autospec-core/src/execution/queue_parser.rs",
];

#[test]
fn aar_is_listed_in_the_top_level_help() {
    let (success, stdout, _) = run(&["--help"]);

    assert!(success);
    assert!(stdout.contains("aar"));
}

#[test]
fn aar_without_a_subcommand_prints_usage() {
    let (success, stdout, _) = run(&["aar"]);

    assert!(success);
    assert!(stdout.contains("USAGE:"));
    assert!(stdout.contains("classify"));
    assert!(stdout.contains("memory"));
}

#[test]
fn an_unknown_subcommand_fails_with_a_diagnostic() {
    let (success, _, stderr) = run(&["aar", "teleport"]);

    assert!(!success);
    assert!(stderr.contains("unknown autospec aar subcommand"));
}

#[test]
fn classify_reports_the_class_risk_and_evidence() {
    let mut args = vec!["aar", "classify"];
    args.extend_from_slice(BUGFIX);

    let (success, stdout, stderr) = run(&args);

    assert!(success, "{stderr}");
    assert!(stdout.contains("task_class: bugfix"));
    assert!(stdout.contains("language: rust"));
    assert!(stdout.contains("evidence:"));
}

#[test]
fn classify_requires_a_title() {
    let (success, _, stderr) = run(&["aar", "classify", "--body", "no title here"]);

    assert!(!success);
    assert!(stderr.contains("--title is required"));
}

#[test]
fn an_unknown_option_fails_rather_than_being_ignored() {
    let (success, _, stderr) = run(&["aar", "classify", "--title", "x", "--nope", "1"]);

    assert!(!success);
    assert!(stderr.contains("unknown option: --nope"));
}

#[test]
fn an_option_missing_its_value_fails() {
    let (success, _, stderr) = run(&["aar", "classify", "--title"]);

    assert!(!success);
    assert!(stderr.contains("--title requires a value"));
}

#[test]
fn a_non_numeric_file_count_fails() {
    let (success, _, stderr) = run(&["aar", "classify", "--title", "x", "--files", "many"]);

    assert!(!success);
    assert!(stderr.contains("--files expects a number"));
}

#[test]
fn plan_prints_the_policy_and_its_explanation() {
    let mut args = vec!["aar", "plan"];
    args.extend_from_slice(BUGFIX);

    let (success, stdout, stderr) = run(&args);

    assert!(success, "{stderr}");
    assert!(stdout.contains("policy_version: aar-v1"));
    assert!(stdout.contains("roles:"));
    assert!(stdout.contains("reasoning:"));
    assert!(stdout.contains("retrieval:"));
    assert!(stdout.contains("Selected "));
}

#[test]
fn explain_prints_only_the_prose_explanation() {
    let mut args = vec!["aar", "explain"];
    args.extend_from_slice(BUGFIX);

    let (success, stdout, stderr) = run(&args);

    assert!(success, "{stderr}");
    assert!(stdout.starts_with("Selected "));
    assert!(!stdout.contains("policy_version:"));
}

#[test]
fn json_output_carries_the_auditable_decision_record() {
    let mut args = vec!["aar", "plan"];
    args.extend_from_slice(BUGFIX);
    args.push("--json");

    let (success, stdout, stderr) = run(&args);

    assert!(success, "{stderr}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(value["policy_version"], "aar-v1");
    assert_eq!(value["task_class"], "bugfix");
    assert!(value["rationale"].as_array().is_some_and(|entries| !entries.is_empty()));
    assert!(value["classification_evidence"]
        .as_array()
        .is_some_and(|entries| !entries.is_empty()));
}

#[test]
fn a_custom_policy_version_is_recorded_in_the_json() {
    let mut args = vec!["aar", "plan"];
    args.extend_from_slice(BUGFIX);
    args.extend_from_slice(&["--policy-version", "aar-2026-09-02", "--json"]);

    let (success, stdout, stderr) = run(&args);

    assert!(success, "{stderr}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(value["policy_version"], "aar-2026-09-02");
}

#[test]
fn rules_prints_the_harness_working_rules() {
    let (success, stdout, _) = run(&["aar", "rules"]);

    assert!(success);
    assert!(stdout.starts_with("Understand relevant code before editing."));
    assert!(stdout.contains("When acceptance criteria are satisfied, STOP."));
}

#[test]
fn a_body_file_supplies_the_work_item_body() {
    let _guard = FILESYSTEM_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = temp_dir("aar-body");
    let body = root.join("body.md");
    std::fs::write(&body, "The parser panics on an empty document.").expect("body written");

    let (success, stdout, stderr) = run(&[
        "aar",
        "classify",
        "--title",
        "Fix the crash",
        "--body-file",
        body.to_str().expect("utf-8 path"),
        "--path",
        "src/parser.rs",
    ]);

    assert!(success, "{stderr}");
    assert!(stdout.contains("task_class: bugfix"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_missing_body_file_fails_with_the_path_in_the_message() {
    let (success, _, stderr) = run(&[
        "aar",
        "classify",
        "--title",
        "x",
        "--body-file",
        "/nonexistent/body.md",
    ]);

    assert!(!success);
    assert!(stderr.contains("/nonexistent/body.md"));
}

#[test]
fn memory_init_scaffolds_every_durable_file_and_the_telemetry_directory() {
    let _guard = FILESYSTEM_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = temp_dir("aar-memory");

    let (success, stdout, stderr) = run(&[
        "aar",
        "memory",
        "init",
        "--worktree",
        root.to_str().expect("utf-8 path"),
    ]);

    assert!(success, "{stderr}");
    for file in [
        "task.md",
        "plan.md",
        "state.md",
        "findings.md",
        "decisions.md",
        "tests.md",
        "review.md",
    ] {
        assert!(
            root.join(".autospec").join(file).is_file(),
            ".autospec/{file} must exist"
        );
        assert!(stdout.contains(file));
    }
    assert!(root.join(".autospec/telemetry").is_dir());
    std::fs::remove_dir_all(&root).ok();
}

/// A second init must never erase an in-flight task's durable state.
#[test]
fn memory_init_is_idempotent_and_never_overwrites_existing_state() {
    let _guard = FILESYSTEM_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = temp_dir("aar-memory-idempotent");
    let worktree = root.to_str().expect("utf-8 path");
    run(&["aar", "memory", "init", "--worktree", worktree]);

    let findings = root.join(".autospec/findings.md");
    std::fs::write(&findings, "# Findings\n\n- the parser panics on empty input\n")
        .expect("state written");

    let (success, stdout, stderr) = run(&["aar", "memory", "init", "--worktree", worktree]);

    assert!(success, "{stderr}");
    assert!(stdout.contains("already present"));
    assert_eq!(
        std::fs::read_to_string(&findings).expect("findings readable"),
        "# Findings\n\n- the parser panics on empty input\n"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn memory_init_rejects_a_worktree_that_is_not_a_directory() {
    let (success, _, stderr) = run(&[
        "aar",
        "memory",
        "init",
        "--worktree",
        "/nonexistent/worktree",
    ]);

    assert!(!success);
    assert!(stderr.contains("is not a directory"));
}

#[test]
fn memory_without_a_subcommand_fails() {
    let (success, _, stderr) = run(&["aar", "memory"]);

    assert!(!success);
    assert!(stderr.contains("requires a subcommand"));
}
