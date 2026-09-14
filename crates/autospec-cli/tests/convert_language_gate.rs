//! The conversion pass's language gate (issue #4559): a patch's language is
//! decided before any branch is created, and a gate that cannot fail for a
//! patch must not be reported as passing it.
//!
//! Plan mode: shell-only / mixed / neither patches are held unevaluated with
//! reasons that name the ruling (#4447) and the deciding files, the
//! Rust/Go-only patch is the only one offered, and the summary line carries
//! the skips — never the idle `converted=0 held=0 skipped=0` line.
//!
//! Apply mode: a language-held patch is terminal — it is archived into
//! `language-held/`, its queue entry is freed, and the pass never sees it
//! again (a re-run enumerates nothing).

use std::fs;
use std::path::Path;
use std::process::Command;

fn autospec_bin() -> &'static str {
    // The test harness runs the compiled bin from the shared target dir.
    env!("CARGO_BIN_EXE_autospec")
}

fn run_convert(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(autospec_bin())
        .args(["convert"])
        .args(args)
        .current_dir(dir)
        .env("LLM", "/nonexistent-llm-for-this-test")
        .output()
        .expect("autospec convert runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A unified-diff body touching exactly the given paths.
fn patch_for(paths: &[&str]) -> String {
    let mut text = String::new();
    for path in paths {
        text.push_str(&format!("diff --git a/{path} b/{path}\n"));
        text.push_str(&format!("--- a/{path}\n"));
        text.push_str(&format!("+++ b/{path}\n"));
        text.push_str("+line\n");
    }
    text
}

fn write_patch(root: &Path, node: &str, issue: u64, paths: &[&str]) {
    let issue_dir = root.join(node).join("out").join(format!("issue-{issue}"));
    fs::create_dir_all(&issue_dir).expect("issue dir");
    fs::write(issue_dir.join("changes.patch"), patch_for(paths)).expect("patch");
}

/// The test's working directory, whose *parent* is unique to this test
/// (#4556): the pass's coverage question is the llm root's siblings, and
/// parallel tests in one binary share a PID — so a shared parent would make
/// each test see the others' llm roots as pipelines it never reached.
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "autospec-convert-lang-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// The llm root under this test's unique parent: its siblings are this
/// test's own artifacts only, so the coverage question is well-formed.
fn llm_root(work: &std::path::Path) -> std::path::PathBuf {
    let root = work.join("root").join("llm");
    fs::create_dir_all(&root).expect("llm root");
    root
}

#[test]
fn plan_mode_holds_ungatetable_languages_before_any_branch() {
    let root = llm_root(&temp_dir("plan"));
    write_patch(&root, "node-a", 101, &["scripts/x.sh"]); // shell only
    write_patch(
        &root,
        "node-a",
        102,
        &["crates/autospec-cli/src/convert.rs", "scripts/y.sh"],
    ); // mixed
    write_patch(
        &root,
        "node-a",
        103,
        &["crates/autospec-core/src/a.rs", "changelog.d/4559.md"],
    ); // rust + docs
    write_patch(&root, "node-a", 104, &["README.md"]); // neither

    let (code, stdout, _stderr) = run_convert(
        &root,
        &["--llm-root", root.to_str().unwrap(), "--base", "main"],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}");

    // Only the Rust/Go patch is offered.
    assert!(stdout.contains("FRESH #103"), "stdout:\n{stdout}");
    assert!(!stdout.contains("FRESH #101"), "stdout:\n{stdout}");
    assert!(!stdout.contains("FRESH #102"), "stdout:\n{stdout}");
    assert!(!stdout.contains("FRESH #104"), "stdout:\n{stdout}");

    // The shell-only hold names the ruling; the patch is never gated.
    let hold_101 = stdout
        .lines()
        .find(|l| l.contains("HOLD  #101"))
        .expect("HOLD line for #101:\n{stdout}");
    assert!(hold_101.contains("shell-only"), "{hold_101}");
    assert!(hold_101.contains("#4447"), "{hold_101}");
    assert!(hold_101.contains("scripts/x.sh"), "{hold_101}");
    assert!(hold_101.contains("unevaluated"), "{hold_101}");

    // The mixed hold names the shell file — the prompt signal.
    let hold_102 = stdout
        .lines()
        .find(|l| l.contains("HOLD  #102"))
        .expect("HOLD line for #102:\n{stdout}");
    assert!(hold_102.contains("mixed"), "{hold_102}");
    assert!(hold_102.contains("scripts/y.sh"), "{hold_102}");
    assert!(hold_102.contains("prompt signal"), "{hold_102}");

    // The neither hold is an explicit decision, not a default.
    let hold_104 = stdout
        .lines()
        .find(|l| l.contains("HOLD  #104"))
        .expect("HOLD line for #104:\n{stdout}");
    assert!(hold_104.contains("neither"), "{hold_104}");
    assert!(hold_104.contains("README.md"), "{hold_104}");
    assert!(hold_104.contains("explicit"), "{hold_104}");

    // The selection line reconciles: examined = fresh + all holds.
    let selection = stdout
        .lines()
        .find(|l| l.contains("conversion pass: examined=4"))
        .expect("selection line:\n{stdout}");
    assert!(selection.contains("fresh=1"), "{selection}");
    assert!(selection.contains("1 shell"), "{selection}");
    assert!(selection.contains("1 mixed"), "{selection}");
    assert!(selection.contains("1 neither"), "{selection}");

    // The bug this issue also closes: a pass that skipped everything must
    // not report the idle `skipped=0` counters.
    let outcome = stdout
        .lines()
        .find(|l| l.contains("convert: examined=4"))
        .expect("outcome line:\n{stdout}");
    assert!(outcome.contains("skipped=3"), "stdout:\n{stdout}");
    assert!(
        !stdout.contains("converted=0 held=0 skipped=0"),
        "stdout:\n{stdout}"
    );

    // No branch was created for a held patch: the gate was never run.
    assert!(
        !stdout.contains("  HELD  #101:"),
        "a plan hold must not write the apply HELD form:\n{stdout}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_json_plan_carries_the_language_holds() {
    let root = llm_root(&temp_dir("json"));
    write_patch(&root, "node-a", 101, &["scripts/x.sh"]);
    write_patch(&root, "node-a", 103, &["crates/autospec-core/src/a.rs"]);

    let (code, stdout, _stderr) = run_convert(
        &root,
        &[
            "--llm-root",
            root.to_str().unwrap(),
            "--base",
            "main",
            "--json",
        ],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("plan JSON:\n{stdout}");
    let held = value["language_held"]
        .as_array()
        .expect("language_held array");
    assert_eq!(held.len(), 1, "{stdout}");
    assert_eq!(held[0]["issue"], 101);
    assert_eq!(held[0]["language"], "shell");
    assert!(
        held[0]["reason"].as_str().unwrap().contains("#4447"),
        "{stdout}"
    );
    let fresh = value["fresh"].as_array().expect("fresh array");
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0]["issue"], 103);
    let _ = fs::remove_dir_all(&root);
}

/// Record the gate the pass will run: without a recorded gate the pass
/// refuses to judge at all (#4556), so a fixture that exercises the gate
/// records it the way a real checkout carries its registry file.
fn record_gate(dir: &Path) {
    let data = dir.join("data");
    fs::create_dir_all(&data).expect("data dir");
    fs::write(
        data.join("convert-gate-registry.json"),
        r#"{"schema":1,"repos":{"test/fake":{"base_ref":"main","stages":[["fmt","--check"],["build","@scope"],["clippy","--all-targets","@scope"],["test","--no-fail-fast","@scope"]]}}}"#,
    )
    .expect("registry");
}

fn init_git_repo(dir: &Path) {
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "-q", "-b", "main"]);
    fs::write(dir.join("README.md"), "x\n").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "-q", "-m", "base"]);
}

#[test]
fn apply_mode_archives_language_holds_and_never_reoffers_them() {
    let work = temp_dir("apply");
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);
    let status = Command::new("git")
        .args(["init", "-q", "--bare", origin.to_str().unwrap()])
        .current_dir(&repo)
        .status()
        .expect("git runs");
    assert!(status.success());
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&["push", "-q", "origin", "main"]);

    record_gate(&repo);

    let llm_root = llm_root(&work);
    write_patch(&llm_root, "node-a", 101, &["scripts/x.sh"]); // shell only
    write_patch(&llm_root, "node-a", 102, &["README.md"]); // neither

    // First run: both patches are held for language and, on --apply,
    // archived. No gate runs (there is nothing offerable), no PR opens.
    let (code, stdout, _stderr) = run_convert(
        &repo,
        &[
            "--apply",
            "--llm-root",
            llm_root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}");
    let archived = stdout.lines().filter(|l| l.contains("ARCHIVED")).count();
    assert_eq!(archived, 2, "stdout:\n{stdout}");
    // The archived reasons must name the deciding language, not degrade to
    // the empty-file verdict: the reason is read while the patch is still
    // on disk.
    let archived_101 = stdout
        .lines()
        .find(|l| l.contains("ARCHIVED #101"))
        .expect("ARCHIVED line for #101:\n{stdout}");
    assert!(archived_101.contains("shell-only"), "{archived_101}");
    assert!(archived_101.contains("scripts/x.sh"), "{archived_101}");
    let archived_102 = stdout
        .lines()
        .find(|l| l.contains("ARCHIVED #102"))
        .expect("ARCHIVED line for #102:\n{stdout}");
    assert!(archived_102.contains("neither"), "{archived_102}");
    let outcome = stdout
        .lines()
        .find(|l| l.contains("convert: examined=2"))
        .expect("outcome line:\n{stdout}");
    assert!(outcome.contains("skipped=2"), "stdout:\n{stdout}");

    // The patches left the enumeration path; the archive is the record.
    assert!(
        !llm_root.join("node-a/out/issue-101/changes.patch").exists(),
        "the shell patch must leave the buffer"
    );
    let archive_dir = llm_root.join("node-a/out/issue-101/language-held");
    let archived_files: Vec<_> = fs::read_dir(&archive_dir)
        .expect("language-held archive dir")
        .flatten()
        .collect();
    assert_eq!(archived_files.len(), 1, "the archive keeps the patch");

    // The terminal property: a re-run enumerates nothing for them. A pass
    // handed no candidates is a different state from a broken one, and both
    // differ from one that examined and held.
    let (code2, stdout2, _stderr2) = run_convert(
        &repo,
        &["--llm-root", llm_root.to_str().unwrap(), "--base", "main"],
    );
    assert_eq!(code2, 0, "stdout:\n{stdout2}");
    assert!(
        !stdout2.contains("#101") && !stdout2.contains("#102"),
        "archived patches must never be re-offered:\n{stdout2}"
    );

    // And no branch was created for either held patch: the gate never ran.
    let branches = Command::new("git")
        .args(["branch", "--list", "convert-*"])
        .current_dir(&repo)
        .output()
        .expect("git branch runs");
    assert!(
        String::from_utf8_lossy(&branches.stdout).trim().is_empty(),
        "no conversion branch may exist for a held patch:\n{}",
        String::from_utf8_lossy(&branches.stdout)
    );
    let _ = fs::remove_dir_all(&work);
}
