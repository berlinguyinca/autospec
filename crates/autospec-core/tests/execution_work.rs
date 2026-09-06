use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::execution::ProducedWork;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A repository with one base commit, and the base OID work is measured against.
fn repository(name: &str) -> (PathBuf, String) {
    let root = std::env::temp_dir().join(format!(
        "autospec-produced-work-{name}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create repository root");
    git(&root, &["init", "--quiet", "--initial-branch=main"]);
    git(&root, &["config", "user.email", "harness@example.invalid"]);
    git(&root, &["config", "user.name", "Harness"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    fs::write(root.join("README.md"), "base\n").expect("seed the base commit");
    git(&root, &["add", "README.md"]);
    git(&root, &["commit", "--quiet", "-m", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    (root, base)
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| panic!("run git {args:?}: {error}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// The false negative from #3563: an agent that commits leaves a clean tree, and a
/// tree-only counter reported five such runs as having produced nothing.
#[test]
fn an_agent_that_commits_its_work_is_detected_as_having_produced_work() {
    let (root, base) = repository("committed");
    fs::write(root.join("fix.rs"), "fn fix() {}\n").expect("write the agent's change");
    git(&root, &["add", "fix.rs"]);
    git(&root, &["commit", "--quiet", "-m", "fix: repair the gate"]);

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");

    assert!(
        !work.is_empty(),
        "a committing agent must not read as having produced nothing: {}",
        work.to_json()
    );
    assert!(
        work.uncommitted_paths.is_empty(),
        "the tree is clean, which is exactly why the tree-only check missed this"
    );
    assert_eq!(work.commits_ahead, 1);
    assert!(work.is_committed_only());
}

/// The commit is only half the fix: the workspace is ephemeral, so the patch — not a
/// count — is what survives teardown.
#[test]
fn committed_work_is_captured_as_a_patch_that_outlives_the_workspace() {
    let (root, base) = repository("captured");
    fs::write(root.join("fix.rs"), "fn fix() {}\n").expect("write the agent's change");
    git(&root, &["add", "fix.rs"]);
    git(&root, &["commit", "--quiet", "-m", "fix: repair the gate"]);

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");
    let patch = work
        .committed_patch
        .as_ref()
        .expect("committed work carries its patch");
    let rendered = String::from_utf8_lossy(patch);
    assert!(rendered.contains("fix: repair the gate"), "{rendered}");
    assert!(rendered.contains("fn fix() {}"), "{rendered}");

    let durable = std::env::temp_dir().join(format!(
        "autospec-captured-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    let written = work
        .write_patch(&durable, "invocation-1")
        .expect("write the captured patch")
        .expect("committed work has a patch to write");
    // Written outside the repository, so wiping the workspace cannot take it.
    assert!(!written.starts_with(&root));
    assert_eq!(fs::read(&written).expect("read back the patch"), *patch);

    let _ = fs::remove_dir_all(&durable);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn uncommitted_changes_still_count_as_produced_work() {
    let (root, base) = repository("uncommitted");
    fs::write(root.join("scratch.rs"), "fn scratch() {}\n").expect("write an untracked change");

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");

    assert!(!work.is_empty());
    assert_eq!(work.commits_ahead, 0);
    assert_eq!(work.uncommitted_paths, vec!["scratch.rs".to_string()]);
    assert!(work.committed_patch.is_none());
    assert!(!work.is_committed_only());

    let _ = fs::remove_dir_all(&root);
}

/// The verdict must still be reachable: a run that really did nothing reads as nothing.
#[test]
fn an_agent_that_produces_nothing_reads_as_no_output() {
    let (root, base) = repository("empty");

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");

    assert!(work.is_empty());
    assert_eq!(work.commits_ahead, 0);
    assert!(work.uncommitted_paths.is_empty());
    assert!(work.committed_patch.is_none());
    assert!(work.to_json().contains("\"produced_work\":false"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_unreadable_repository_is_an_error_rather_than_an_empty_result() {
    let missing = std::env::temp_dir().join(format!(
        "autospec-produced-work-absent-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));

    let error = ProducedWork::detect(&missing, "HEAD")
        .expect_err("a repository that cannot be inspected must not report zero work");

    assert!(error.contains("git"), "{error}");
}

/// The caller's verdict already discounts its own bookkeeping files. If detection does
/// not discount the same ones, a routine empty run reads as "work produced" — the mirror
/// image of the bug, and just as wrong.
#[test]
fn excluded_paths_are_not_counted_as_the_agents_work() {
    let (root, base) = repository("excluded");
    fs::create_dir_all(root.join(".autospec")).expect("create the harness scratch directory");
    fs::write(root.join(".autospec/executor-closeout.md"), "# Closeout\n")
        .expect("write the harness's own bookkeeping");

    let counted = ProducedWork::detect(&root, &base).expect("detect without exclusions");
    assert!(
        !counted.is_empty(),
        "without exclusions the harness's own file looks like work"
    );

    let work =
        ProducedWork::detect_excluding(&root, &base, &[":(exclude).autospec/executor-closeout.md"])
            .expect("detect with exclusions");

    assert!(
        work.is_empty(),
        "an excluded bookkeeping file must not read as produced work: {}",
        work.to_json()
    );

    let _ = fs::remove_dir_all(&root);
}

/// Uncommitted work that survived the commit step is still work, and is exactly the case
/// the executor's zero-effect path can reach with a clean index.
#[test]
fn uncommitted_work_outside_the_exclusions_is_still_counted() {
    let (root, base) = repository("excluded-partial");
    fs::create_dir_all(root.join(".autospec")).expect("create the harness scratch directory");
    fs::write(root.join(".autospec/executor-closeout.md"), "# Closeout\n")
        .expect("write the harness's own bookkeeping");
    fs::write(root.join("agent.rs"), "fn agent() {}\n").expect("write the agent's change");

    let work =
        ProducedWork::detect_excluding(&root, &base, &[":(exclude).autospec/executor-closeout.md"])
            .expect("detect with exclusions");

    assert_eq!(work.uncommitted_paths, vec!["agent.rs".to_string()]);
    assert!(!work.is_empty());

    let _ = fs::remove_dir_all(&root);
}

/// `git status -z` spends two records on a rename: the new path, then the original.
/// Counting the second as a change of its own would report one edit as two.
#[test]
fn a_rename_counts_once_rather_than_twice() {
    let (root, base) = repository("rename");
    git(&root, &["mv", "README.md", "GUIDE.md"]);

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");

    assert_eq!(work.uncommitted_paths, vec!["GUIDE.md".to_string()]);

    let _ = fs::remove_dir_all(&root);
}
