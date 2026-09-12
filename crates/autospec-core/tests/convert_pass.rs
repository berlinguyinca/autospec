//! The conversion-pass guard (issue #3654).
//!
//! The worktree lock used to exclude only other converters. A human operator
//! could reset, clean, or prune the worktree while a conversion pass was in
//! flight, and the corrupted "HELD: build error" verdict was memoized into
//! the durable memo keyed on (patch hash, base sha). These tests cover the
//! four acceptance criteria: the marker, the maintenance refusal, the
//! conditional verdict commit, and the cheap logged invalidation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use autospec_core::convert_pass::{
    clean, maintain, reset, MaintenanceAction, MaintenanceError, Marker, MemoKey, Verdict,
    VerdictMemo,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A repository with one base commit, the way a conversion worktree starts.
fn repository(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "autospec-convert-pass-{name}-{}-{}",
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
    root
}

/// A memo file path outside any repository, the way durable state lives.
fn memo_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "autospec-convert-memo-{name}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ))
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

/// AC1: the marker names the holder and its PID while the pass runs, and is
/// gone when the pass ends.
#[test]
fn the_marker_names_holder_and_pid_and_is_released_when_the_pass_ends() {
    let root = repository("marker");
    let key = MemoKey::new("patch-1", "base-1");
    let mut memo = VerdictMemo::open(&memo_path("marker")).expect("open memo");

    let verdict =
        autospec_core::convert_pass::run_pass(&root, "operator@host", &key, &mut memo, || {
            assert!(
                Marker::held(&root),
                "the marker must be present while the pass is in flight"
            );
            let record = Marker::read(&root)
                .expect("read the marker")
                .expect("a record");
            assert_eq!(record.holder, "operator@host");
            assert_eq!(record.pid, std::process::id());
            Ok(Verdict::Clean)
        })
        .expect("a clean pass completes");

    assert!(verdict.is_clean());
    assert!(
        !Marker::held(&root),
        "the marker must be released when the pass ends"
    );
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_file(memo.path());
}

/// AC2: the moment a human holds the worktree, the shared maintenance
/// helpers refuse — naming the holder and PID — and leave the tree alone.
/// This is the exact scenario that produced the "HELD: build error" verdict.
#[test]
fn maintenance_refuses_a_held_worktree_and_leaves_it_untouched() {
    let root = repository("held");
    let held = Marker::acquire(&root, "operator@host").expect("the operator holds the tree");

    fs::write(root.join("README.md"), "operator's in-flight edit\n").expect("dirty the tree");
    fs::write(root.join("scratch.tmp"), "untracked\n").expect("add an untracked file");

    for action in [
        MaintenanceAction::Reset,
        MaintenanceAction::Clean,
        MaintenanceAction::Prune,
    ] {
        let refusal = maintain(&root, action).expect_err("a held worktree must refuse maintenance");
        match &refusal {
            MaintenanceError::Held {
                action: refused,
                holder,
                pid,
            } => {
                assert_eq!(refused, &action, "the refusal names the action");
                assert_eq!(holder, "operator@host", "the refusal names the holder");
                assert_eq!(*pid, std::process::id(), "the refusal names the PID");
            }
            other => panic!("expected a held refusal, got {other:?}"),
        }
    }

    assert_eq!(
        fs::read_to_string(root.join("README.md")).expect("read back"),
        "operator's in-flight edit\n",
        "reset must not have touched the tree"
    );
    assert!(
        root.join("scratch.tmp").is_file(),
        "clean must not have run"
    );
    drop(held);
    let _ = fs::remove_dir_all(&root);
}

/// AC2, other half: once the pass releases, the same helpers do their job.
#[test]
fn maintenance_runs_once_the_pass_releases() {
    let root = repository("released");
    {
        let held = Marker::acquire(&root, "pass").expect("acquire");
        drop(held);
    }

    fs::write(root.join("README.md"), "stray edit\n").expect("dirty the tree");
    fs::write(root.join("scratch.tmp"), "untracked\n").expect("add an untracked file");

    reset(&root).expect("reset proceeds after release");
    clean(&root).expect("clean proceeds after release");

    assert_eq!(
        fs::read_to_string(root.join("README.md")).expect("read back"),
        "base\n",
        "the stray edit must be gone"
    );
    assert!(
        !root.join("scratch.tmp").exists(),
        "the untracked file must be gone"
    );
    let _ = fs::remove_dir_all(&root);
}

/// A marker that exists but cannot be read names no one; the guard still
/// refuses, because failing open is how the corrupted verdict got made.
#[test]
fn an_unreadable_marker_fails_maintenance_closed() {
    let root = repository("unreadable");
    let marker_path = Marker::path(&root);
    fs::create_dir_all(marker_path.parent().expect("marker directory"))
        .expect("create marker directory");
    fs::write(&marker_path, "not json a human left behind").expect("write an unreadable marker");

    for action in [
        MaintenanceAction::Reset,
        MaintenanceAction::Clean,
        MaintenanceAction::Prune,
    ] {
        let refusal = maintain(&root, action).expect_err("must refuse");
        assert!(
            matches!(refusal, MaintenanceError::MarkerUnreadable { .. }),
            "an unreadable marker must fail closed, got {refusal:?}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// AC3: a completed pass commits its verdict, and the memo survives a reopen.
#[test]
fn a_completed_pass_commits_its_verdict_and_the_memo_survives_a_reopen() {
    let root = repository("commit-clean");
    let key = MemoKey::new("patch-1", "base-1");
    let memo_file = memo_path("commit-clean");
    let mut memo = VerdictMemo::open(&memo_file).expect("open memo");

    let verdict =
        autospec_core::convert_pass::run_pass(&root, "converter", &key, &mut memo, || {
            Ok(Verdict::Clean)
        })
        .expect("clean pass");
    assert_eq!(verdict, Verdict::Clean);

    let reopened = VerdictMemo::open(&memo_file).expect("reopen the memo");
    assert_eq!(
        reopened.get(&key),
        Some(&Verdict::Clean),
        "the verdict must be durable"
    );
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_file(&memo_file);
}

/// AC3, the load-bearing half: a negative verdict from a clean run is
/// trustworthy (the run finished), but a run that was interrupted is not,
/// and commits nothing.
#[test]
fn an_interrupted_run_commits_nothing_to_the_memo() {
    let root = repository("interrupted");
    let key = MemoKey::new("patch-1", "base-1");
    let memo_file = memo_path("interrupted");
    let mut memo = VerdictMemo::open(&memo_file).expect("open memo");

    let reason = autospec_core::convert_pass::run_pass(&root, "converter", &key, &mut memo, || {
        Err("build error: the tree was reset under us".to_string())
    })
    .expect_err("an interrupted run is an error");

    assert!(reason.contains("reset under us"), "{reason}");
    assert!(
        memo.get(&key).is_none(),
        "a corrupted run must not memoize its verdict"
    );
    assert!(
        fs::read_to_string(memo_file).is_err(),
        "no memo file may have been written at all"
    );
    assert!(
        !Marker::held(&root),
        "the marker must be released even when the run errors"
    );
    let _ = fs::remove_dir_all(&root);
}

/// A pass that panics is also interrupted: the marker is released and the
/// memo is untouched.
#[test]
fn a_panicking_run_releases_the_marker_and_commits_nothing() {
    let root = repository("panic");
    let key = MemoKey::new("patch-1", "base-1");
    let memo_file = memo_path("panic");
    let mut memo = VerdictMemo::open(&memo_file).expect("open memo");

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        autospec_core::convert_pass::run_pass(&root, "converter", &key, &mut memo, || {
            panic!("simulated interruption mid-pass")
        })
    }));
    assert!(outcome.is_err(), "the interruption must propagate");

    assert!(memo.get(&key).is_none(), "a panicked run must not memoize");
    assert!(
        !Marker::held(&root),
        "the marker must be released after a panic"
    );
    let rehold = Marker::acquire(&root, "again").expect("the worktree is usable again");
    drop(rehold);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_file(&memo_file);
}

/// A pass cannot start on a worktree someone else holds, and refuses without
/// touching the memo.
#[test]
fn a_held_worktree_refuses_the_pass_itself() {
    let root = repository("pass-held");
    let _held = Marker::acquire(&root, "operator@host").expect("the operator holds the tree");
    let key = MemoKey::new("patch-1", "base-1");
    let memo_file = memo_path("pass-held");
    let mut memo = VerdictMemo::open(&memo_file).expect("open memo");

    let refusal =
        autospec_core::convert_pass::run_pass(&root, "converter", &key, &mut memo, || {
            Ok(Verdict::Clean)
        })
        .expect_err("a held worktree must refuse the pass");

    assert!(
        refusal.contains("operator@host"),
        "the refusal names the holder: {refusal}"
    );
    assert!(
        refusal.contains(&std::process::id().to_string()),
        "the refusal names the PID"
    );
    assert!(memo.get(&key).is_none());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_file(&memo_file);
}

/// AC4: invalidation is cheap — one key drops, one log line, the other
/// keys stay put — and invalidating an absent key is a quiet no-op.
#[test]
fn invalidation_drops_one_key_and_logs_the_drop() {
    let memo_file = memo_path("invalidate");
    let mut memo = VerdictMemo::open(&memo_file).expect("open memo");
    let keep = MemoKey::new("patch-2", "base-2");
    let drop = MemoKey::new("patch-1", "base-1");
    memo.record(&keep, &Verdict::Clean).expect("record keep");
    memo.record(
        &drop,
        &Verdict::Failed {
            reason: "HELD: build error".into(),
        },
    )
    .expect("record the corrupted verdict");

    let mut log = Vec::new();
    let dropped = memo
        .invalidate(&drop, &mut |line| log.push(line.to_string()))
        .expect("invalidate");
    assert!(dropped, "a cached verdict was dropped");
    assert_eq!(log.len(), 1, "exactly one log line");
    assert!(
        log[0].contains(drop.id().as_str()),
        "the log line names the dropped key: {}",
        log[0]
    );

    assert!(memo.get(&drop).is_none(), "the dropped key is gone");
    assert_eq!(
        memo.get(&keep),
        Some(&Verdict::Clean),
        "the other key stays"
    );

    let reopened = VerdictMemo::open(&memo_file).expect("reopen");
    assert!(
        reopened.get(&drop).is_none(),
        "the drop must be durable, not just in memory"
    );

    let dropped_again = memo
        .invalidate(&drop, &mut |line| log.push(line.to_string()))
        .expect("invalidate again");
    assert!(!dropped_again, "absent key: nothing to drop");
    assert_eq!(log.len(), 1, "an absent key logs nothing");
    let _ = fs::remove_file(&memo_file);
}

/// The memo is keyed on (patch hash, base sha): the same patch against a
/// different base is a different verdict, and both survive side by side.
#[test]
fn the_memo_is_keyed_on_patch_hash_and_base_sha() {
    let memo_file = memo_path("keyed");
    let mut memo = VerdictMemo::open(&memo_file).expect("open memo");
    let on_old_base = MemoKey::new("patch-1", "sha-old");
    let on_new_base = MemoKey::new("patch-1", "sha-new");

    memo.record(&on_old_base, &Verdict::Clean)
        .expect("record old base");
    memo.record(
        &on_new_base,
        &Verdict::Failed {
            reason: "build error".into(),
        },
    )
    .expect("record new base");

    assert_eq!(memo.get(&on_old_base), Some(&Verdict::Clean));
    assert_eq!(
        memo.get(&on_new_base),
        Some(&Verdict::Failed {
            reason: "build error".into()
        })
    );
    let _ = fs::remove_file(&memo_file);
}
