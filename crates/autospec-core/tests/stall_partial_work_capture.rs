//! Partial-work capture: the section that used to lose the work.
//!
//! Two failure modes are pinned here. The first is the obvious one: capture had
//! to happen while the worktree still exists, against real git output. The
//! second is the one that caused the bug — an agent that commits its work leaves
//! a *clean* working tree, so a working-tree-only check reported real progress
//! as nothing.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use autospec_core::stall::{
    capture_partial_work, classify_work, read_tail, Artifact, ArtifactStore, GitWorktreeEvidence,
    WorkProduced,
};

fn temp_dir(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "autospec-stall-{}-{}-{:?}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    );
    path.push(unique);
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn git_program() -> OsString {
    std::env::var_os("AUTOSPEC_GIT_PROGRAM").unwrap_or_else(|| "git".into())
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new(git_program())
        .current_dir(repo)
        .args([
            "-c",
            "user.name=Stall Test",
            "-c",
            "user.email=stall@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("HOME", repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A repo with one commit on base, one commit ahead, one tracked edit and one
/// untracked file — the shape a committing agent actually leaves behind.
fn scratch_repo() -> (PathBuf, String) {
    let repo = temp_dir("repo");
    git(&repo, &["init", "--quiet"]);
    fs::write(repo.join("base.rs"), "fn base() {}\n").expect("write base");
    git(&repo, &["add", "base.rs"]);
    git(&repo, &["commit", "--quiet", "-m", "base commit"]);
    let base = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();

    fs::write(repo.join("feature.rs"), "fn feature() {}\n").expect("write feature");
    git(&repo, &["add", "feature.rs"]);
    git(&repo, &["commit", "--quiet", "-m", "implement the feature"]);

    fs::write(repo.join("base.rs"), "fn base() {}\nfn half_done() {}\n").expect("edit base");
    fs::write(repo.join("notes.md"), "unfinished thoughts\n").expect("write notes");
    (repo, base)
}

#[test]
fn commits_are_captured_even_though_they_leave_a_clean_look_at_first() {
    let (repo, base) = scratch_repo();
    let evidence = GitWorktreeEvidence::new(&repo, None);
    let work = capture_partial_work(&evidence, &base, 64 * 1024);

    assert!(work.capture_errors.is_empty(), "{:?}", work.capture_errors);
    assert_eq!(work.commits.len(), 1);
    assert_eq!(work.commits[0].subject, "implement the feature");
    assert!(work.commit_patch.contains("fn feature()"));
    assert!(
        work.working_tree_patch.contains("half_done"),
        "the uncommitted edit is missing: {}",
        work.working_tree_patch
    );
    assert!(
        work.working_tree_patch.contains("notes.md"),
        "untracked files must be captured too: {}",
        work.working_tree_patch
    );
    assert_eq!(
        work.work_produced(),
        WorkProduced::CommitsAndWorkingTree { count: 1 }
    );
}

#[test]
fn a_committing_agent_is_never_reported_as_having_produced_nothing() {
    // The exact regression: commits ahead, working tree clean after commit.
    let (repo, base) = scratch_repo();
    git(&repo, &["stash", "--include-untracked", "--quiet"]);
    let evidence = GitWorktreeEvidence::new(&repo, None);
    let work = capture_partial_work(&evidence, &base, 64 * 1024);

    assert_eq!(work.work_produced(), WorkProduced::Commits { count: 1 });
    assert!(!work.commit_patch.trim().is_empty());
    assert!(classify_work(1, false).produced());
    assert!(!classify_work(0, false).produced());
}

#[test]
fn one_unreadable_section_never_aborts_the_rest_of_the_capture() {
    // Not a git worktree at all: every git section fails, capture still returns.
    let dir = temp_dir("not-a-repo");
    let transcript = dir.join("session.jsonl");
    fs::write(&transcript, "line one\nline two\n").expect("write transcript");

    let evidence = GitWorktreeEvidence::new(&dir, Some(transcript.clone()));
    let work = capture_partial_work(&evidence, "HEAD", 64 * 1024);

    assert!(
        !work.capture_errors.is_empty(),
        "a failed git section must be recorded"
    );
    assert_eq!(work.commits.len(), 0);
    assert!(
        work.transcript_excerpt.contains("line two"),
        "transcript capture is independent of git: {:?}",
        work.transcript_excerpt
    );
    assert_eq!(work.transcript_bytes, 18);
}

#[test]
fn the_transcript_tail_is_the_end_of_the_file_and_valid_utf8() {
    let dir = temp_dir("tail");
    let path = dir.join("transcript.txt");
    // Multibyte characters straddle the tail boundary on purpose.
    let mut body = "a".repeat(500);
    body.push_str("héllo wörld — ✓ ✓ ✓");
    fs::write(&path, body.as_bytes()).expect("write transcript");

    let (tail, total) = read_tail(&path, 16).expect("read tail");
    assert_eq!(total, body.len() as u64);
    assert!(
        tail.len() <= 16,
        "tail exceeded the byte budget: {}",
        tail.len()
    );
    assert!(body.ends_with(&tail), "tail is not the end of the file");
    // Cutting mid-codepoint would panic here or produce replacement chars.
    assert!(!tail.contains('\u{fffd}'));

    let (all, _) = read_tail(&path, 100_000).expect("read whole");
    assert_eq!(all, body);
}

#[test]
fn artifacts_land_in_a_private_per_attempt_layout() {
    let root = temp_dir("store");
    let store = ArtifactStore::new(&root);

    let first = store
        .write(
            42,
            1,
            &Artifact {
                name: "attempt-1-commits.patch".into(),
                body: "patch one".into(),
            },
        )
        .expect("write attempt 1");
    store
        .write(
            42,
            2,
            &Artifact {
                name: "attempt-2-commits.patch".into(),
                body: "patch two".into(),
            },
        )
        .expect("write attempt 2");

    assert_eq!(
        first,
        root.join("issue-42")
            .join("attempt-1")
            .join("attempt-1-commits.patch")
    );
    assert_eq!(store.latest_attempt(42).expect("latest"), Some(2));
    assert_eq!(store.latest_attempt(43).expect("missing issue"), None);

    let latest = store.read_latest(42).expect("read latest");
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].body, "patch two");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_mode = fs::metadata(first.parent().unwrap())
            .expect("attempt dir")
            .permissions()
            .mode();
        let file_mode = fs::metadata(&first).expect("artifact").permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700, "attempt dir mode {dir_mode:o}");
        assert_eq!(file_mode & 0o777, 0o600, "artifact mode {file_mode:o}");
    }
}

#[test]
fn an_empty_worktree_produces_no_artifacts() {
    let (repo, base) = scratch_repo();
    git(&repo, &["stash", "--include-untracked", "--quiet"]);
    git(&repo, &["reset", "--hard", &base, "--quiet"]);
    let evidence = GitWorktreeEvidence::new(&repo, None);
    let work = capture_partial_work(&evidence, &base, 64 * 1024);

    assert_eq!(work.work_produced(), WorkProduced::None);
    assert!(
        work.artifacts(1).is_empty(),
        "nothing captured means nothing to attach: {:?}",
        work.artifacts(1)
    );
}
