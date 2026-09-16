//! #4644: a baseline whose failures are identical before and after is a
//! usable baseline, not a hold.
//!
//! The production incident (iw-74): a crate's `schema-gen` suite fails on
//! the execution host because `npm` is missing — at the base, before any
//! patch is applied, and again after. The old pass read the identical
//! pre/post failure as a verdict about the patch, recorded `UNKNOWN-NO-
//! BASELINE`, and held the patch for an environment it never caused.
//!
//! The pass must instead attribute the failure to the base: when a stage
//! fails after the patch AND the same stage fails at the base, the change
//! is unmeasured, not defective (#4610). The patch is skipped as base
//! unverifiable — never held — and the issue re-enters dispatch for a
//! later pass on a host that can measure it.
//!
//! This test drives the real `--apply` pass against a fixture workspace
//! whose one crate carries a consistently-failing environmental test (a
//! required tool the host does not provide), and asserts the verdict is
//! usable: the patch is skipped as base unverifiable, and nothing durable
//! records it as the change's fault.

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
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The base crate: gate-green for `build`/`clippy`/`fmt`, but with one
/// environmental test that fails at the base and again after any patch —
/// exactly the shape of a host that cannot run a suite (npm-missing
/// `schema-gen`). `build`, `clippy` and `fmt` do not run tests, so they
/// pass; only the test stage trips, identically before and after.
fn init_fixture_repo(repo: &Path, origin: &Path) {
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        repo.join("src/lib.rs"),
        "pub fn one() -> u32 {\n    1\n}\n\n#[test]\nfn the_base_tests() {\n    assert_eq!(one(), 1);\n}\n\n#[test]\nfn an_environmental_failure() {\n    // The host is missing the tool this suite needs (like `npm` for the\n    // schema-gen crate): a real failure of the environment, reproduced at\n    // the base and again after the patch -- never a verdict about the\n    // patch.\n    let tool = std::env::var(\"AUTOSPEC_REQUIRED_TOOL\");\n    assert!(\n        tool.map(|t| !t.is_empty()).unwrap_or(false),\n        \"required tool not present on the execution host\"\n    );\n}\n",
    )
    .unwrap();
    // Record the gate the pass will run: without one, --apply refuses to
    // judge at all (#4556).
    let data = repo.join("data");
    fs::create_dir_all(&data).expect("data dir");
    fs::write(
        data.join("convert-gate-registry.json"),
        r#"{"schema":1,"repos":{"test/fake":{"base_ref":"main","stages":[["fmt","--check"],["build","@scope"],["clippy","--all-targets","@scope"],["test","--no-fail-fast","@scope"]]}}}"#,
    )
    .expect("registry");
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
}

/// A stand-in for `gh`: the pass only needs `pr create` to succeed; there
/// is no remote interaction that must be faked with real answers here.
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

/// The patch: adds a passing test function, so the pass measures a baseline
/// (`tests_added > 0`) and the unchanged-count contradiction (#4532) is in
/// play — and yet the environmental failure must still be attributed to the
/// base, never held against the patch. The diff is generated by `git diff`
/// itself.
fn write_patch(llm_root: &Path, repo: &Path, issue: u64) {
    let lib = repo.join("src/lib.rs");
    let base = fs::read_to_string(&lib).unwrap();
    fs::write(
        &lib,
        format!("{base}\n#[test]\nfn the_patch_adds_a_passing_test() {{\n    assert_eq!(one(), 1);\n}}\n"),
    )
    .unwrap();
    let patch = run_git_output(repo, &["diff"]);
    run_git(repo, &["checkout", "-q", "--", "src/lib.rs"]);
    let issue_dir = llm_root
        .join("node-a")
        .join("out")
        .join(format!("issue-{issue}"));
    fs::create_dir_all(&issue_dir).unwrap();
    fs::write(issue_dir.join("changes.patch"), patch).unwrap();
}

/// Run the real `convert --apply` pass against the fixture.
fn run_apply(repo: &Path, llm_root: &Path, work: &Path) -> (i32, String, String) {
    let path = format!(
        "{}:{}",
        work.join("fakebin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::new(autospec_bin());
    command
        .args(["convert", "--apply", "--llm-root"])
        .arg(llm_root)
        .arg("--base")
        .arg("main")
        .arg("--repo")
        .arg("test/fake")
        .current_dir(repo)
        .env("LLM", "/nonexistent-llm-for-this-test")
        .env("PATH", &path)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command.output().expect("autospec convert --apply runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A crate whose suite consistently fails on this host is an environmental
/// failure at the base, not a verdict about the patch. The patch must be
/// skipped as base unverifiable — never held, never converted.
#[test]
fn a_consistently_failing_crate_skips_the_patch_not_holds_it() {
    let work = std::env::temp_dir().join(format!(
        "autospec-conv-baseline-unverifiable-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    init_fixture_repo(&repo, &origin);
    write_patch(&llm_root, &repo, 4644);
    install_fake_gh(&work);

    let (code, stdout, stderr) = run_apply(&repo, &llm_root, &work);
    assert_eq!(
        code, 0,
        "the pass judges and skips, it does not fail:\n{stdout}\nstderr:\n{stderr}"
    );

    // The verdict is usable: the patch is named as skipped because the base
    // is unverifiable, on its own line.
    assert!(
        stdout.contains("  DONE   #4644: skipped: base unverifiable"),
        "the patch is skipped as base unverifiable:\n{stdout}"
    );
    // ...and nothing records it as the change's fault. The summary's
    // `held=0 skipped=1` is the durable truth; a HELD line would be the
    // regression.
    assert!(
        !stdout.contains("HELD") && !stdout.contains(" held "),
        "the environmental failure must not hold the patch:\n{stdout}"
    );
    assert!(
        stdout.contains("held=0") && stdout.contains("skipped=1"),
        "the pass counts it as skipped, never held:\n{stdout}"
    );

    let _ = fs::remove_dir_all(&work);
}
