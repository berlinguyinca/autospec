//! A held patch on a closed issue is residue, not pending work (#4626).
//!
//! The incident that motivated this: a backlog of held patches where two
//! thirds were for issues that had already been closed. Nothing asked the
//! tracker, so each pass re-gated the same holds against a base that kept
//! moving — work claimed to be pending that did not exist.
//!
//! The invariant under test: when the issue is closed, the pass archives
//! the patch (the queue entry releases with it) and releases the hold
//! record — it never gates, never opens a PR, and the next pass sees
//! nothing to hold. When the issue is open, or its state cannot be read,
//! the hold survives: a hold is lifted by the fact of closure, never by
//! the absence of a state.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// A minimal git repo with a `main` branch pushed to a bare origin.
fn init_fixture_repo(repo: &Path, origin: &Path) {
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
}

/// The pass refuses to run without a recorded gate, even when nothing will
/// actually be gated (#4556): record the fixture's gate the way a real
/// checkout carries its registry file.
fn record_gate(repo: &Path) {
    let data = repo.join("data");
    fs::create_dir_all(&data).expect("data dir");
    fs::write(
        data.join("convert-gate-registry.json"),
        r#"{"schema":1,"repos":{"test/fake":{"base_ref":"main","stages":[["fmt","--check"],["build","@scope"]]}}}"#,
    )
    .expect("registry");
}

/// A trivial, applicable patch: one comment line, generated with
/// `git diff` so its blob hashes are real.
fn make_patch(repo: &Path) -> String {
    fs::write(
        repo.join("src/a.rs"),
        "pub fn a() -> u32 {\n    1\n}\n// the residue\n",
    )
    .unwrap();
    let patch = run_git_output(repo, &["diff"]);
    run_git(repo, &["checkout", "--", "src/a.rs"]);
    patch
}

fn write_patch(llm_root: &Path, issue: u64, text: &str) {
    let issue_dir = llm_root
        .join("node-a")
        .join("out")
        .join(format!("issue-{issue}"));
    fs::create_dir_all(&issue_dir).unwrap();
    fs::write(issue_dir.join("changes.patch"), text).unwrap();
}

/// The patch's input key, computed the way the pass computes it: the file
/// mtime in seconds since the epoch.
fn patch_mkey(path: &Path) -> String {
    let modified = fs::metadata(path).unwrap().modified().unwrap();
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string()
}

/// Record a hold for the issue, derived against the current base with the
/// patch's own key: `re_gate` finds nothing changed, so the hold stands
/// until something lifts it.
fn record_hold(llm_root: &Path, repo: &Path, issue: u64, patch_path: &Path) {
    let base_sha = run_git_output(repo, &["rev-parse", "HEAD"]);
    let record = format!(
        "{{\"issue\":{issue},\"patch_key\":\"{}\",\"base_sha\":\"{base_sha}\",\
         \"depends_on\":[],\"reason\":\"conflict: src/a.rs\"}}",
        patch_mkey(patch_path)
    );
    fs::write(llm_root.join("held.txt"), record + "\n").unwrap();
}

/// A stand-in for `gh` on PATH.
///
/// `state` is what the issue-state probe (`gh api .../issues/N --jq .state`)
/// returns: `closed`, `open`, or `fail` (the call fails). Every call is
/// appended to the log so the test can prove the pass never opened a PR.
fn install_fake_gh(work: &Path) -> PathBuf {
    let bin_dir = work.join("fakebin");
    fs::create_dir_all(&bin_dir).unwrap();
    let shim = bin_dir.join("gh");
    fs::write(
        &shim,
        "#!/bin/sh\necho \"$@\" >> \"$GH_LOG\"\ncase \"$1\" in\n\
         api)\n  case \"$GH_STATE\" in\n    fail) exit 1;;\n    *) echo \"$GH_STATE\";;\n  esac;;\n\
         pr)\n  echo \"[]\";;\n\
         repo)\n  echo \"test/fake\";;\n\
         *) exit 0;;\nesac\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin_dir
}

fn patch_path(llm_root: &Path, issue: u64) -> PathBuf {
    llm_root
        .join("node-a")
        .join("out")
        .join(format!("issue-{issue}"))
        .join("changes.patch")
}

fn superseded_dir(llm_root: &Path, issue: u64) -> PathBuf {
    llm_root
        .join("node-a")
        .join("out")
        .join(format!("issue-{issue}"))
        .join("superseded")
}

fn fixture_work(test: &str) -> PathBuf {
    let work = std::env::temp_dir().join(format!(
        "autospec-conv-closed-{test}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    work
}

fn run_convert(
    work: &Path,
    repo: &Path,
    llm_root: &Path,
    bin_dir: &Path,
    state: &str,
) -> (bool, String, String) {
    let mut path = bin_dir.to_string_lossy().into_owned();
    path.push(':');
    path.push_str(&std::env::var("PATH").unwrap_or_default());
    let output = Command::new(autospec_bin())
        .args([
            "convert",
            "--apply",
            "--llm-root",
            llm_root.to_str().unwrap(),
            "--base",
            "main",
            "--repo",
            "test/fake",
        ])
        .current_dir(repo)
        .env("LLM", "/nonexistent-llm-for-this-test")
        .env("PATH", &path)
        .env("GH_STATE", state)
        .env("GH_LOG", work.join("gh-log.txt").to_str().unwrap())
        .output()
        .expect("autospec convert runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The incident, fixed: the hold is lifted by the fact of closure — the
/// patch is archived, the hold record removed, no gate, no PR.
#[test]
fn a_held_patch_on_a_closed_issue_is_archived_and_its_hold_released() {
    let work = fixture_work("closed");
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    init_fixture_repo(&repo, &origin);
    record_gate(&repo);
    write_patch(&llm_root, 305, &make_patch(&repo));
    record_hold(&llm_root, &repo, 305, &patch_path(&llm_root, 305));
    let bin_dir = install_fake_gh(&work);

    let (ok, stdout, stderr) = run_convert(&work, &repo, &llm_root, &bin_dir, "closed");
    assert!(ok, "apply fails:\n{stdout}\n{stderr}");

    // The decision line names the issue and the reason.
    assert!(
        stdout.lines().any(|l| l.trim()
            == "CLOSED #305 (issue is closed: the patch is archived, its hold released)"),
        "the closed line must name the issue:\n{stdout}"
    );
    // The pass never gated, never offered, never opened anything.
    assert!(
        !stdout.contains("START #305"),
        "a closed issue is never offered:\n{stdout}"
    );
    assert!(
        !stdout.contains("HOLD #305"),
        "a closed issue is never re-held:\n{stdout}"
    );
    // The accounting says what the line says.
    assert!(
        stdout.lines().any(|l| l.contains("closed=1")),
        "the outcome must count the closure:\n{stdout}"
    );

    // The patch left its enumeration path and landed in `superseded/`.
    assert!(
        !patch_path(&llm_root, 305).exists(),
        "the patch must leave the enumeration path"
    );
    let archived: Vec<String> = fs::read_dir(superseded_dir(&llm_root, 305))
        .expect("the superseded directory exists")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !archived.is_empty(),
        "the patch must be archived, not deleted: {archived:?}"
    );

    // The hold record is gone: the claim of pending work no longer exists.
    let held = fs::read_to_string(llm_root.join("held.txt")).unwrap_or_default();
    assert!(
        !held.contains("\"issue\":305"),
        "the hold record must be released:\n{held}"
    );

    // No PR, ever: the pass did not write to the tracker.
    let log = fs::read_to_string(work.join("gh-log.txt")).unwrap_or_default();
    assert!(
        !log.lines().any(|l| l.contains("pr create")),
        "a closed issue must not open a PR:\n{log}"
    );
    // And no branch on the origin for it.
    let heads = Command::new("git")
        .args(["ls-remote", "--heads", &origin.display().to_string()])
        .output()
        .expect("ls-remote runs")
        .stdout;
    let heads = String::from_utf8_lossy(&heads);
    assert!(
        !heads.contains("conv-305"),
        "no branch for a closed issue: {heads}"
    );

    let _ = fs::remove_dir_all(&work);
}

/// The control: the issue is open, so the hold stands. Nothing is
/// archived, nothing is released, and the next pass re-gates as before.
#[test]
fn a_held_patch_on_an_open_issue_stays_held() {
    let work = fixture_work("open");
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    init_fixture_repo(&repo, &origin);
    record_gate(&repo);
    write_patch(&llm_root, 305, &make_patch(&repo));
    record_hold(&llm_root, &repo, 305, &patch_path(&llm_root, 305));
    let bin_dir = install_fake_gh(&work);

    let (ok, stdout, stderr) = run_convert(&work, &repo, &llm_root, &bin_dir, "open");
    assert!(ok, "apply fails:\n{stdout}\n{stderr}");

    assert!(
        !stdout.contains("CLOSED #305"),
        "open is not closed:\n{stdout}"
    );
    assert!(
        patch_path(&llm_root, 305).exists(),
        "an open issue's patch must not be archived"
    );
    let held = fs::read_to_string(llm_root.join("held.txt")).unwrap_or_default();
    assert!(
        held.contains("\"issue\":305"),
        "an open issue's hold must survive:\n{held}"
    );
    let _ = fs::remove_dir_all(&work);
}

/// The fail-open half: the tracker cannot be asked (the call fails), and
/// the unknown answers "no" — the hold survives. Unknown never authorises
/// acting; the release needs the fact of closure, not its absence.
#[test]
fn an_unreadable_issue_state_leaves_the_hold_in_place() {
    let work = fixture_work("unreachable");
    let repo = work.join("repo");
    let origin = work.join("origin.git");
    let llm_root = work.join("llm");
    init_fixture_repo(&repo, &origin);
    record_gate(&repo);
    write_patch(&llm_root, 305, &make_patch(&repo));
    record_hold(&llm_root, &repo, 305, &patch_path(&llm_root, 305));
    let bin_dir = install_fake_gh(&work);

    let (ok, stdout, stderr) = run_convert(&work, &repo, &llm_root, &bin_dir, "fail");
    assert!(ok, "apply fails:\n{stdout}\n{stderr}");

    assert!(
        !stdout.contains("CLOSED #305"),
        "an unreadable state must not read as closed:\n{stdout}"
    );
    assert!(
        patch_path(&llm_root, 305).exists(),
        "an unreadable state must not archive the patch"
    );
    let held = fs::read_to_string(llm_root.join("held.txt")).unwrap_or_default();
    assert!(
        held.contains("\"issue\":305"),
        "an unreadable state must not release the hold:\n{held}"
    );
    let _ = fs::remove_dir_all(&work);
}
