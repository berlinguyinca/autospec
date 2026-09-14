//! Per-pipeline coverage and the per-repository gate registry (issue #4556).
//!
//! The pass is specified over `$L/*/out/issue-*/changes.patch` but runs
//! against one pipeline's root; a run that reaches 1 of 4 pipelines must say
//! so and must not report success, and a count of zero from a directory that
//! was never opened must not be reportable as "nothing to convert". The gate
//! set is data: a repository with no recorded gate is refused rather than
//! guessed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn autospec_bin() -> &'static str {
    env!("CARGO_BIN_EXE_autospec")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "autospec-convert-coverage-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// One patch under a pipeline's node directory: the shape the pass's own
/// enumeration reads (`<llm-root>/<node>/out/issue-*/changes.patch`). The
/// patch touches Rust so the plan stays free of language holds and the
/// picture is exactly the coverage question.
fn write_patch(llm_root: &Path, pipeline: &str, issue: u64) {
    let issue_dir = llm_root
        .join(pipeline)
        .join("node-a")
        .join("out")
        .join(format!("issue-{issue}"));
    fs::create_dir_all(&issue_dir).expect("issue dir");
    fs::write(
        issue_dir.join("changes.patch"),
        "diff --git a/crates/core/src/a.rs b/crates/core/src/a.rs\n\
         --- a/crates/core/src/a.rs\n\
         +++ b/crates/core/src/a.rs\n\
         +line\n",
    )
    .expect("patch");
}

fn run_convert(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(autospec_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("autospec convert runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_run_reaching_one_of_four_pipelines_is_incomplete_and_exits_3() {
    let work = temp_dir("four");
    let llm = work.join("llm");
    write_patch(&llm, "autospec", 1);
    for issue in [2, 3] {
        write_patch(&llm, "iw", issue);
    }
    write_patch(&llm, "disp", 4);
    write_patch(&llm, "orch", 5);

    let (code, stdout, stderr) = run_convert(
        &work,
        &[
            "convert",
            "--llm-root",
            llm.join("autospec").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 3, "stdout:\n{stdout}\nstderr:\n{stderr}");
    // The summary line carries the coverage; the gaps are named per pipeline
    // (diagnostics: stderr, so a --json stdout stays one clean document).
    assert!(
        stdout.contains("coverage=1/4 pipelines"),
        "stdout:\n{stdout}"
    );
    assert!(
        stderr.contains("coverage gap: pipeline 'iw' holds 2"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("coverage gap: pipeline 'disp' holds 1"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("coverage gap: pipeline 'orch' holds 1"),
        "stderr:\n{stderr}"
    );
    // The status line is the failure message: stderr, with exit 3.
    assert!(
        stderr.contains("conversion pass incomplete: reached 'autospec'"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn a_run_handed_the_shared_root_reaches_every_pipeline() {
    let work = temp_dir("shared");
    let llm = work.join("llm");
    // A shared root's pipelines hold `out` directly: the pass's own
    // enumeration reads root/<pipeline>/out/issue-* and reaches every one.
    for (pipeline, issue) in [("autospec", 1u64), ("iw", 2)] {
        let issue_dir = llm
            .join(pipeline)
            .join("out")
            .join(format!("issue-{issue}"));
        fs::create_dir_all(&issue_dir).expect("issue dir");
        fs::write(
            issue_dir.join("changes.patch"),
            "diff --git a/crates/core/src/a.rs b/crates/core/src/a.rs\n+line\n",
        )
        .expect("patch");
    }

    let (code, stdout, _stderr) = run_convert(
        &work,
        &[
            "convert",
            "--llm-root",
            llm.to_str().unwrap(),
            "--shared-llm-root",
        ],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}");
    assert!(
        stdout.contains("coverage=2/2 pipelines"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn a_single_pipeline_has_no_coverage_question() {
    let work = temp_dir("single");
    let llm = work.join("llm");
    write_patch(&llm, "autospec", 1);

    let (code, stdout, _stderr) = run_convert(
        &work,
        &[
            "convert",
            "--llm-root",
            llm.join("autospec").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}");
    assert!(!stdout.contains("coverage="), "stdout:\n{stdout}");
    assert!(!stdout.contains("coverage gap"), "stdout:\n{stdout}");
}

#[test]
fn an_unreached_pipeline_with_no_patches_does_not_block_completeness() {
    let work = temp_dir("empty");
    let llm = work.join("llm");
    write_patch(&llm, "autospec", 1);
    fs::create_dir_all(llm.join("orch").join("out")).expect("empty pipeline");

    let (code, stdout, _stderr) = run_convert(
        &work,
        &[
            "convert",
            "--llm-root",
            llm.join("autospec").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}");
    assert!(
        stdout.contains("coverage=2/2 pipelines"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn the_json_plan_carries_the_coverage() {
    let work = temp_dir("json");
    let llm = work.join("llm");
    write_patch(&llm, "autospec", 1);
    write_patch(&llm, "iw", 2);

    let (code, stdout, _stderr) = run_convert(
        &work,
        &[
            "convert",
            "--json",
            "--llm-root",
            llm.join("autospec").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 3, "stdout:\n{stdout}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("plan JSON");
    assert_eq!(value["coverage"]["reached"], "autospec");
    assert_eq!(value["coverage"]["complete"], false);
    assert!(value["coverage"]["suffix"]
        .as_str()
        .unwrap()
        .contains("1/2"));
    assert!(value["summary"]
        .as_str()
        .unwrap()
        .contains("coverage=1/2 pipelines"));
}

#[test]
fn apply_refuses_a_repository_with_no_recorded_gate_before_judging() {
    let work = temp_dir("noga");
    let llm = work.join("llm");
    write_patch(&llm, "autospec", 1);

    // No data/convert-gate-registry.json under the checkout: the pass must
    // refuse before any patch is judged, and record nothing.
    let (code, _stdout, stderr) = run_convert(
        &work,
        &[
            "convert",
            "--apply",
            "--llm-root",
            llm.join("autospec").to_str().unwrap(),
            "--repo",
            "test/fake",
        ],
    );
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(
        stderr.contains("no gate established for test/fake"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("refuses to guess a gate"),
        "stderr:\n{stderr}"
    );
    assert!(
        !work.join("llm").join("held.txt").exists(),
        "nothing was recorded"
    );
}

#[test]
fn apply_refuses_a_repository_the_registry_does_not_name() {
    let work = temp_dir("named");
    let data = work.join("data");
    fs::create_dir_all(&data).expect("data dir");
    fs::write(
        data.join("convert-gate-registry.json"),
        r#"{"schema":1,"repos":{"someone/else":{"base_ref":"main","stages":[["test"]]}}}"#,
    )
    .expect("registry");
    let llm = work.join("llm");
    write_patch(&llm, "autospec", 1);

    let (code, _stdout, stderr) = run_convert(
        &work,
        &[
            "convert",
            "--apply",
            "--llm-root",
            llm.join("autospec").to_str().unwrap(),
            "--repo",
            "test/fake",
        ],
    );
    assert_eq!(code, 2, "stderr:\n{stderr}");
    assert!(
        stderr.contains("no gate recorded for test/fake"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn a_plan_warns_on_a_repository_with_no_recorded_gate_but_still_reports() {
    let work = temp_dir("planwarn");
    let llm = work.join("llm");
    write_patch(&llm, "autospec", 1);

    let (code, stdout, stderr) = run_convert(
        &work,
        &[
            "convert",
            "--llm-root",
            llm.join("autospec").to_str().unwrap(),
            "--repo",
            "test/fake",
        ],
    );
    // The plan is still useful (it says what would be judged) and the run is
    // whole (one pipeline, all reached): exit 0, the refusal only warned.
    assert_eq!(code, 0, "stdout:\n{stdout}");
    assert!(
        stderr.contains("no gate established for test/fake"),
        "stderr:\n{stderr}"
    );
    assert!(stdout.contains("examined=1"), "stdout:\n{stdout}");
}
