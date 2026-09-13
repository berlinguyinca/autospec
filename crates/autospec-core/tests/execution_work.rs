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
    assert!(work.uncommitted_patch.is_some());
    assert!(!work.is_committed_only());

    let _ = fs::remove_dir_all(&root);
}

/// The discarded-runs case from #4582, verbatim: the agent staged its work and stopped.
/// `staged=3 commits_ahead=0` used to produce a file list and no patch, because the only
/// captured state was the committed one. Staging is an encoding of "the agent changed
/// these files", and capture must not depend on which encoding it found.
#[test]
fn staged_only_work_is_captured_as_a_patch() {
    let (root, base) = repository("staged-only");
    for name in ["one.rs", "two.rs", "three.rs"] {
        fs::write(root.join(name), format!("fn {name} {{}}\n")).expect("write the agent's change");
        git(&root, &["add", name]);
    }

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");
    assert_eq!(work.commits_ahead, 0);
    assert_eq!(work.uncommitted_paths.len(), 3);

    let durable = std::env::temp_dir().join(format!(
        "autospec-staged-capture-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    let written = work
        .write_patch(&durable, "invocation-staged")
        .expect("write the captured patch")
        .expect("staged work has a patch to write — this is the #4582 regression");
    let bytes = fs::read(&written).expect("read back the patch");
    let rendered = String::from_utf8_lossy(&bytes);
    for name in ["one.rs", "two.rs", "three.rs"] {
        assert!(
            rendered.contains(name),
            "patch is missing {name}: {rendered}"
        );
    }
    work.assert_captured(Some(&written))
        .expect("a captured patch authorises the deletion");

    let _ = fs::remove_dir_all(&durable);
    let _ = fs::remove_dir_all(&root);
}

/// Dirty-but-unstaged tracked changes are the same work in a different encoding.
#[test]
fn dirty_only_work_is_captured_as_a_patch() {
    let (root, base) = repository("dirty-only");
    fs::write(root.join("README.md"), "base, amended\n").expect("dirty the tracked file");

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");
    assert_eq!(work.commits_ahead, 0);
    let durable = std::env::temp_dir().join(format!(
        "autospec-dirty-capture-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    let written = work
        .write_patch(&durable, "invocation-dirty")
        .expect("write the captured patch")
        .expect("dirty work has a patch to write");
    let bytes = fs::read(&written).expect("read back the patch");
    let rendered = String::from_utf8_lossy(&bytes);
    assert!(rendered.contains("base, amended"), "{rendered}");

    let _ = fs::remove_dir_all(&durable);
    let _ = fs::remove_dir_all(&root);
}

/// Untracked files are in no diff of the index, so they need their own pass — and the
/// most common "work" an agent leaves is a brand-new file it never `git add`ed.
#[test]
fn untracked_only_work_is_captured_as_a_patch() {
    let (root, base) = repository("untracked-only");
    fs::write(root.join("fresh.rs"), "fn fresh() {}\n").expect("write the new file");

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");
    let durable = std::env::temp_dir().join(format!(
        "autospec-untracked-capture-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    let written = work
        .write_patch(&durable, "invocation-untracked")
        .expect("write the captured patch")
        .expect("an untracked file is work and has a patch to write");
    let bytes = fs::read(&written).expect("read back the patch");
    let rendered = String::from_utf8_lossy(&bytes);
    assert!(rendered.contains("fn fresh() {}"), "{rendered}");

    let _ = fs::remove_dir_all(&durable);
    let _ = fs::remove_dir_all(&root);
}

/// All three encodings at once: a commit, a staged file, a dirty edit, and a new file.
/// They are one work product, so they are one patch, in one file.
#[test]
fn a_mixed_state_is_captured_as_a_single_patch() {
    let (root, base) = repository("mixed-state");
    fs::write(root.join("committed.rs"), "fn committed() {}\n").expect("write");
    git(&root, &["add", "committed.rs"]);
    git(&root, &["commit", "--quiet", "-m", "first part"]);
    fs::write(root.join("staged.rs"), "fn staged() {}\n").expect("write");
    git(&root, &["add", "staged.rs"]);
    fs::write(root.join("README.md"), "base, amended\n").expect("dirty the base file");
    fs::write(root.join("fresh.rs"), "fn fresh() {}\n").expect("write the untracked file");

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");
    assert_eq!(work.commits_ahead, 1);
    let durable = std::env::temp_dir().join(format!(
        "autospec-mixed-capture-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    let written = work
        .write_patch(&durable, "invocation-mixed")
        .expect("write the captured patch")
        .expect("a mixed state has a patch to write");
    let bytes = fs::read(&written).expect("read back the patch");
    let rendered = String::from_utf8_lossy(&bytes);
    for needle in [
        "first part",
        "fn committed() {}",
        "fn staged() {}",
        "base, amended",
        "fn fresh() {}",
    ] {
        assert!(
            rendered.contains(needle),
            "patch is missing {needle:?}: {rendered}"
        );
    }

    let _ = fs::remove_dir_all(&durable);
    let _ = fs::remove_dir_all(&root);
}

/// The summary is never written without the artifact it summarises: every non-empty
/// detection yields a non-empty patch file, and only an empty detection yields none.
#[test]
fn a_non_empty_detection_always_yields_a_written_patch() {
    let (root, base) = repository("summary-needs-artifact");
    fs::write(root.join("a.rs"), "fn a() {}\n").expect("write");
    git(&root, &["add", "a.rs"]);

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");
    let durable = std::env::temp_dir().join(format!(
        "autospec-summary-artifact-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    let written = work
        .write_patch(&durable, "invocation-x")
        .expect("write the captured patch")
        .expect("non-empty work always has the artifact behind its summary");
    assert!(fs::metadata(&written).expect("metadata").len() > 0);

    let empty = ProducedWork::detect(&repository("summary-empty-artifact").0, &"HEAD")
        .expect("detect an empty tree");
    assert!(empty.is_empty());
    assert_eq!(
        empty
            .write_patch(&durable, "invocation-empty")
            .expect("no patch to write"),
        None,
        "an empty run writes no patch and no file that pretends otherwise"
    );

    let _ = fs::remove_dir_all(&durable);
    let _ = fs::remove_dir_all(&root);
}

/// The exit trap deletes the only copy, so the deletion is conditional on the capture:
/// work without a non-empty patch refuses to go away, and says what it is holding.
#[test]
fn the_deletion_guard_refuses_work_without_a_patch() {
    let (root, base) = repository("deletion-guard");
    fs::write(root.join("held.rs"), "fn held() {}\n").expect("write the work");
    git(&root, &["add", "held.rs"]);

    let work = ProducedWork::detect(&root, &base).expect("detect produced work");
    assert!(!work.is_empty());

    let missing = std::env::temp_dir().join(format!(
        "autospec-guard-missing-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    work.assert_captured(None)
        .expect_err("no patch at all must not authorise the deletion");
    let error = work
        .assert_captured(Some(&missing))
        .expect_err("work without a patch on disk refuses the deletion");
    assert!(
        error.contains("held.rs"),
        "the refusal names the work it holds: {error}"
    );

    let empty_file = std::env::temp_dir().join(format!(
        "autospec-guard-empty-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    fs::write(&empty_file, b"").expect("an empty patch file is no artifact");
    work.assert_captured(Some(&empty_file))
        .expect_err("an empty patch file is no artifact");

    let written = work
        .write_patch(empty_file.parent().unwrap(), "guard")
        .expect("write")
        .expect("the capture succeeds");
    work.assert_captured(Some(&written))
        .expect("a non-empty patch authorises the deletion");

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_file(&empty_file);
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
