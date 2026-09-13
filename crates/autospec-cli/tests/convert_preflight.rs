//! The conversion pass's tool preconditions (issue #4589): the pass verifies
//! the tools its gate requires before judging anything. A missing tool is an
//! environment failure, not a verdict — one named FATAL, non-zero exit, and
//! nothing in the durable ledger, instead of one HELD record per patch that
//! would be indistinguishable from a genuine gate failure and could suppress
//! re-gating of work that was never gated.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn autospec_bin() -> &'static str {
    env!("CARGO_BIN_EXE_autospec")
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "autospec-convert-preflight-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A rust patch the gate can evaluate: the preflight is about the host, so
/// the fixture must not add a language hold to the picture.
fn write_patch(root: &Path, issue: u64) {
    let issue_dir = root
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

#[cfg(unix)]
fn find_on_path(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(tool);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// A `bin` directory holding symlinks to exactly the named tools, for a
/// child whose PATH says which tools the host has.
#[cfg(unix)]
fn restricted_bin(tag: &str, tools: &[&str]) -> PathBuf {
    let bin = temp_dir(&format!("bin-{tag}"));
    for tool in tools {
        let source = find_on_path(tool).unwrap_or_else(|| {
            panic!("{tool} is not on the test PATH, so it cannot be faked present")
        });
        std::os::unix::fs::symlink(source, bin.join(tool)).expect("symlink");
    }
    bin
}

fn run_convert(
    dir: &Path,
    args: &[&str],
    path: &str,
    wrapper: Option<&str>,
) -> (i32, String, String) {
    let mut command = Command::new(autospec_bin());
    command
        .args(["convert"])
        .args(args)
        .current_dir(dir)
        .env("PATH", path);
    if let Some(wrapper) = wrapper {
        command.env("AUTOSPEC_GATE_WRAPPER", wrapper);
    }
    let output = command.output().expect("autospec convert runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_missing_tool_is_one_fatal_before_any_judging_not_one_held_per_patch() {
    // The incident: a cron PATH without the toolchain produced 28 HELD
    // records, each naming the patch and none naming the host.
    let root = temp_dir("fatal");
    write_patch(&root, 77);

    let (code, stdout, stderr) = run_convert(
        &root,
        &[
            "--apply",
            "--llm-root",
            root.to_str().unwrap(),
            "--base",
            "main",
        ],
        "/nonexistent-path-for-this-test",
        None,
    );
    assert_eq!(code, 1, "stderr:\n{stderr}");
    assert!(
        stderr.starts_with("FATAL:"),
        "the single line names the failure: {stderr}"
    );
    // Nothing on the PATH, so the whole gate toolchain is named at once.
    assert!(stderr.contains("cargo, git, gh not on PATH"), "{stderr}");
    assert!(stderr.contains("nothing was recorded"), "{stderr}");

    // Invariant 2: the environment failure did not enter the durable ledger,
    // and no verdict of any kind was rendered.
    assert!(
        !root.join("held.txt").exists(),
        "no HELD ledger may be written"
    );
    assert!(!stdout.contains("HELD"), "{stdout}");
    assert!(
        !stdout.contains("######## convert:"),
        "no outcome line: {stdout}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_missing_tool_in_plan_mode_warns_without_refusing() {
    // A plan does not run the gate, so it is still useful on a broken host —
    // but it must say that --apply would refuse.
    let root = temp_dir("warn");
    write_patch(&root, 77);

    let (code, stdout, stderr) = run_convert(
        &root,
        &["--llm-root", root.to_str().unwrap(), "--base", "main"],
        "/nonexistent-path-for-this-test",
        None,
    );
    assert_eq!(code, 0, "stderr:\n{stderr}");
    assert!(
        stderr.contains("WARN: cargo, git, gh not on PATH; --apply would refuse"),
        "{stderr}"
    );
    // The plan itself is complete: the rust patch is offered.
    assert!(stdout.contains("FRESH #77"), "{stdout}");
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_wrapped_gate_needs_cargo_on_the_execution_host_not_the_submit_host() {
    // With AUTOSPEC_GATE_WRAPPER set the gate's cargo runs where the wrapper
    // places it (#4598); the submit host may legitimately lack it. git and gh
    // are still required here: the pass branches, applies, and opens PRs
    // locally.
    let git = find_on_path("git");
    let gh = find_on_path("gh");
    if git.is_none() || gh.is_none() {
        eprintln!("SKIP: git or gh not on the test PATH; the exemption cannot be faked");
        return;
    }
    let bin = restricted_bin("wrapped", &["git", "gh"]);
    let root = temp_dir("wrapped");
    write_patch(&root, 77);

    let (code, _stdout, stderr) = run_convert(
        &root,
        &[
            "--apply",
            "--llm-root",
            root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ],
        bin.to_str().unwrap(),
        Some("srun --"),
    );
    // The preflight must not have fired: no FATAL, and the pass proceeded far
    // enough to hit the (equally absent) remote — a different failure.
    assert!(!stderr.contains("FATAL:"), "{stderr}");
    assert_ne!(code, 1, "stderr:\n{stderr}");
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&bin);
}

#[cfg(unix)]
#[test]
fn a_complete_toolchain_passes_the_preflight_and_judges() {
    let (git, gh, cargo) = (
        find_on_path("git"),
        find_on_path("gh"),
        find_on_path("cargo"),
    );
    if git.is_none() || gh.is_none() || cargo.is_none() {
        eprintln!("SKIP: git, gh, or cargo not on the test PATH; presence cannot be faked");
        return;
    }
    let bin = restricted_bin("complete", &["git", "gh", "cargo"]);
    let root = temp_dir("complete");
    write_patch(&root, 77);

    let (code, _stdout, stderr) = run_convert(
        &root,
        &[
            "--apply",
            "--llm-root",
            root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ],
        bin.to_str().unwrap(),
        None,
    );
    // Every tool present: the precondition holds, so the failure (if any) is
    // the pass's own — the fixture has no remote — never the host's.
    assert!(!stderr.contains("FATAL:"), "{stderr}");
    assert_ne!(code, 1, "stderr:\n{stderr}");
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&bin);
}
