//! #4607: the pass sizes its batch to its own deadline.
//!
//! No conversion pass has ever finished its batch: the batch size and the
//! deadline were chosen independently and never compared against the
//! observed cost of a single patch, so every pass was killed mid-gate (the
//! rc=143 line) after one verdict out of a dozen, and the candidates it
//! never reached were re-gated from scratch next pass. These tests run the
//! real pass against a real fixture crate and a real cargo gate, with a
//! deadline the batch cannot meet, and assert that the pass declines to
//! start what it cannot finish — `deferred=N` on the outcome line, exit
//! zero, the in-flight item finished — instead of being killed mid-gate.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn autospec_bin() -> &'static str {
    env!("CARGO_BIN_EXE_autospec")
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

fn run_git_output(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

/// A minimal, gate-green Rust crate at a bare local origin.
fn init_fixture_repo(repo: &Path, origin: &Path) -> String {
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(repo.join("src/lib.rs"), "pub mod a;\n").unwrap();
    fs::write(repo.join("src/a.rs"), "pub fn a() -> u32 {\n    1\n}\n").unwrap();
    run_git(repo, &["init", "-q", "-b", "main"]);
    let status = Command::new("git")
        .args(["init", "-q", "--bare", origin.to_str().unwrap()])
        .current_dir(repo)
        .status()
        .expect("git runs");
    assert!(status.success());
    run_git(repo, &["add", "-A"]);
    run_git(repo, &["commit", "-q", "-m", "base"]);
    run_git(repo, &["remote", "add", "origin", origin.to_str().unwrap()]);
    run_git(repo, &["push", "-q", "origin", "main"]);
    run_git_output(repo, &["rev-parse", "main:src/lib.rs"])
}

/// A stand-in for `gh` on PATH: it succeeds and records nothing, so the
/// liveness check sees no live PR and a passing patch opens a PR.
fn install_fake_gh(work: &Path) -> PathBuf {
    let bin_dir = work.join("fakebin");
    fs::create_dir_all(&bin_dir).unwrap();
    let shim = bin_dir.join("gh");
    fs::write(&shim, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin_dir
}

fn write_patch(llm_root: &Path, issue: u64, text: &str) {
    let issue_dir = llm_root
        .join("node-a")
        .join("out")
        .join(format!("issue-{issue}"));
    fs::create_dir_all(&issue_dir).unwrap();
    fs::write(issue_dir.join("changes.patch"), text).unwrap();
}

/// The first patch: a module whose test sleeps three seconds, so the gate's
/// cost is measurable and comfortably above the deadline the test gives the
/// pass. The test itself passes — the point is the cost, not the verdict.
fn slow_patch(lib_blob: &str) -> String {
    format!(
        "diff --git a/src/lib.rs b/src/lib.rs\nindex {lib_blob}..0000000\n\
         --- a/src/lib.rs\n+++ b/src/lib.rs\n\
         @@ -1 +1,2 @@\n pub mod a;\n+pub mod slow;\n\
         diff --git a/src/slow.rs b/src/slow.rs\nnew file mode 100644\n\
         --- /dev/null\n+++ b/src/slow.rs\n\
         @@ -0,0 +1,4 @@\n+\
         #[test]\n+fn the_gate_cost_is_measurable() {{\n+    std::thread::sleep(std::time::Duration::from_secs(3));\n+}}\n"
    )
}

/// The second patch: trivial and gate-green. It is what the pass must
/// decline to start when the deadline is already spent — a patch it never
/// reaches is re-offered next pass, not lost.
fn second_patch(lib_blob: &str) -> String {
    format!(
        "diff --git a/src/lib.rs b/src/lib.rs\nindex {lib_blob}..0000000\n\
         --- a/src/lib.rs\n+++ b/src/lib.rs\n\
         @@ -1 +1,2 @@\n pub mod a;\n+pub mod c;\n\
         diff --git a/src/c.rs b/src/c.rs\nnew file mode 100644\n\
         --- /dev/null\n+++ b/src/c.rs\n\
         @@ -0,0 +1,3 @@\n+pub fn c() -> u32 {{\n+    3\n+}}\n"
    )
}

/// The pass's worktree path is a global per branch name
/// (`<temp>/autospec-conv-<branch>`), so two passes that run concurrently
/// over the same issue numbers would collide on the worktree directory. The
/// three tests in this binary run in parallel, so each gets its own pair of
/// issue numbers — and therefore its own worktrees.
fn build_fixture(work: &Path, slow_issue: u64) -> (PathBuf, u64) {
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    let lib_blob = init_fixture_repo(&repo, &origin);
    write_patch(&llm_root, slow_issue, &slow_patch(&lib_blob));
    write_patch(&llm_root, slow_issue + 1, &second_patch(&lib_blob));
    let _ = install_fake_gh(work);
    (repo, slow_issue)
}

/// Run the apply pass with the given extra args and env, capturing stdout.
fn run_pass(
    repo: &Path,
    llm_root: &Path,
    work: &Path,
    extra_args: &[&str],
    envs: &[(&str, &str)],
) -> (i32, String) {
    let mut path = work.join("fakebin").to_string_lossy().into_owned();
    path.push(':');
    path.push_str(&std::env::var("PATH").unwrap_or_default());
    let mut command = Command::new(autospec_bin());
    command
        .args(["convert", "--apply", "--llm-root"])
        .arg(llm_root)
        .arg("--base")
        .arg("main")
        .arg("--repo")
        .arg("test/fake")
        .args(extra_args)
        .current_dir(repo)
        .env("LLM", "/nonexistent-llm-for-this-test")
        .env("PATH", &path)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("autospec convert runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
    )
}

#[test]
fn a_pass_with_a_deadline_defers_what_it_cannot_finish() {
    let work = std::env::temp_dir().join(format!("autospec-conv-sizing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let (repo, slow_issue) = build_fixture(&work, 4601);
    let deferred_issue = slow_issue + 1;
    let llm_root = work.join("llm");

    // The deadline (2s) is less than the first item's cost (its gate runs a
    // test that sleeps 3s), so after the first item finishes there is no
    // time left for the second. The pass must finish the in-flight item,
    // decline to start the next one, and say so on the outcome line.
    let (code, stdout) = run_pass(&repo, &llm_root, &work, &["--deadline", "2"], &[]);
    assert_eq!(code, 0, "the pass ends itself; it is not killed:\n{stdout}");
    assert!(
        stdout.contains("examined=2"),
        "both candidates were examined:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("  CONVERT  #{slow_issue}")),
        "the in-flight item is finished, not abandoned:\n{stdout}"
    );
    assert!(
        !stdout.contains(&format!("  CONVERT  #{deferred_issue}"))
            && !stdout.contains(&format!("  HELD  #{deferred_issue}")),
        "the deferred item was never started — no verdict for it:\n{stdout}"
    );
    assert!(
        stdout.contains("deferred=1"),
        "the outcome names what was not reached:\n{stdout}"
    );
    assert!(
        stdout.contains("  DEFER  1 candidate(s) not started"),
        "the deferral is announced where the decisions are announced:\n{stdout}"
    );
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn a_pass_with_no_deadline_works_through_the_whole_batch() {
    // The control: without a deadline the pass does not size its batch —
    // it works through every candidate and reports zero deferred, so the
    // new counter cannot read as a default state.
    let work = std::env::temp_dir().join(format!(
        "autospec-conv-sizing-control-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let (repo, slow_issue) = build_fixture(&work, 4603);
    let llm_root = work.join("llm");

    let (code, stdout) = run_pass(&repo, &llm_root, &work, &[], &[]);
    assert_eq!(code, 0, "\n{stdout}");
    assert!(
        stdout.contains(&format!("  CONVERT  #{slow_issue}")),
        "\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("  CONVERT  #{}", slow_issue + 1)),
        "\n{stdout}"
    );
    assert!(
        stdout.contains("deferred=0"),
        "no deadline, nothing deferred:\n{stdout}"
    );
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn the_deadline_from_the_environment_sizes_the_batch_the_same_way() {
    // The scheduler that runs a scheduled pass is the one that knows the
    // deadline; the env is how it hands it over without the pass inventing
    // one. The flag and the env must agree on the behavior.
    let work =
        std::env::temp_dir().join(format!("autospec-conv-sizing-env-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let (repo, slow_issue) = build_fixture(&work, 4605);
    let llm_root = work.join("llm");

    let (code, stdout) = run_pass(
        &repo,
        &llm_root,
        &work,
        &[],
        &[("AUTOSPEC_CONVERT_DEADLINE", "2")],
    );
    assert_eq!(code, 0, "\n{stdout}");
    assert!(
        stdout.contains(&format!("  CONVERT  #{slow_issue}"))
            && !stdout.contains(&format!("  CONVERT  #{}", slow_issue + 1)),
        "\n{stdout}"
    );
    assert!(stdout.contains("deferred=1"), "\n{stdout}");
    let _ = fs::remove_dir_all(&work);
}
