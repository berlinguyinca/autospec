//! `safe_publish` tests.
//!
//! Moved out of the crate's test module so the process-hygiene guard
//! (#4569) has room to live with the tests it protects: the incident was a
//! `tail -f` spawned here that outlived its test by 6h50m, following a
//! directory that had already been deleted, because the reap lived at the
//! end of the test body and never ran when an assertion failed first.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::safe_publish::{
    next_versioned_path, open_file_holders, publish_overwrite, publish_versioned, OpenFileHolder,
    OpenersStatus, SafePublishError,
};

fn test_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("autospec-safe-publish-{name}-{nonce}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// A reader holding a test file open: `tail -f` opens the file and keeps
/// the descriptor open while following it, mirroring a shell streaming a
/// long script.
///
/// The guard owns the follower's lifetime and the directory's deletion
/// (#4569): dropping it — on a finished test, a failed assertion, or a
/// panic — kills the follower, waits for it, and removes the directory
/// only then. The follower never outlives the test, never stays a zombie
/// under the test binary, and is never left following a path that has
/// already been deleted.
struct Holder {
    dir: PathBuf,
    child: Option<Child>,
}

impl Holder {
    fn spawn(dir: &Path, file: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            child: Some(Command::new("tail").arg("-f").arg(file).spawn().unwrap()),
        }
    }

    fn pid(&self) -> u32 {
        self.child.as_ref().expect("holder in flight").id()
    }

    /// Stop the follower and wait for it, keeping the directory for
    /// assertions on the cleared state; the drop still removes it.
    fn release(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        // Kill, wait, and only then remove the directory: the reverse order
        // is exactly the incident — a follower left on a path about to be
        // deleted, invisible because the path no longer exists.
        self.release();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn next_versioned_path_appends_two_to_a_fresh_name() {
    assert_eq!(
        next_versioned_path(Path::new("/x/iw-issue.sh")),
        PathBuf::from("/x/iw-issue-2.sh")
    );
    assert_eq!(
        next_versioned_path(Path::new("foo")),
        PathBuf::from("foo-2")
    );
}

#[test]
fn next_versioned_path_skips_existing_versions() {
    let dir = test_dir("next-version");
    fs::write(dir.join("foo-2.sh"), "x").unwrap();
    assert_eq!(
        next_versioned_path(&dir.join("foo.sh")),
        dir.join("foo-3.sh")
    );
    cleanup(&dir);
}

#[test]
fn next_versioned_path_continues_from_a_versioned_name() {
    let dir = test_dir("next-versioned");
    assert_eq!(
        next_versioned_path(&dir.join("foo-2.sh")),
        dir.join("foo-3.sh")
    );
    cleanup(&dir);
}

#[test]
fn open_file_holders_is_none_for_a_missing_file() {
    let dir = test_dir("missing");
    assert_eq!(
        open_file_holders(&dir.join("absent.sh")),
        OpenersStatus::None
    );
    cleanup(&dir);
}

#[test]
fn open_file_holders_is_none_for_a_free_file() {
    let dir = test_dir("free");
    let file = dir.join("script.sh");
    fs::write(&file, "echo one\n").unwrap();
    assert_eq!(open_file_holders(&file), OpenersStatus::None);
    cleanup(&dir);
}

#[test]
#[cfg(target_os = "linux")]
fn detects_a_live_reader_and_clears_after_exit() {
    let dir = test_dir("holders");
    let file = dir.join("script.sh");
    fs::write(&file, "echo one\n").unwrap();

    let mut holder = Holder::spawn(&dir, &file);
    let status = open_file_holders(&file);
    match status {
        OpenersStatus::Holders(holders) => {
            assert!(
                holders.iter().any(|h| h.pid == holder.pid()),
                "holder list {holders:?} must include the spawned reader"
            );
        }
        other => panic!("expected holders, got {other:?}"),
    }

    holder.release();
    assert_eq!(open_file_holders(&file), OpenersStatus::None);
}

#[test]
#[cfg(target_os = "linux")]
fn overwrite_refuses_while_a_reader_holds_the_file() {
    let dir = test_dir("refused");
    let file = dir.join("iw-issue.sh");
    fs::write(&file, "old content\n").unwrap();

    let holder = Holder::spawn(&dir, &file);
    match publish_overwrite(&file, b"new content\n") {
        Err(SafePublishError::Refused { path, holders }) => {
            assert_eq!(path, file);
            assert!(!holders.is_empty());
        }
        other => panic!("expected Refused, got {other:?}"),
    }
    // Original bytes are intact and no temp file was left behind.
    assert_eq!(fs::read_to_string(&file).unwrap(), "old content\n");
    let leftovers: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "iw-issue.sh")
        .collect();
    assert!(leftovers.is_empty(), "temp residue: {leftovers:?}");
    drop(holder);
}

#[test]
#[cfg(target_os = "linux")]
fn overwrite_of_a_free_file_succeeds_and_records_previous() {
    let dir = test_dir("overwrite-free");
    let file = dir.join("foo.sh");
    fs::write(&file, "old\n").unwrap();

    let receipt = publish_overwrite(&file, b"new\n").unwrap();
    assert_eq!(receipt.published_to, file);
    assert_eq!(receipt.previous.as_deref(), Some(file.as_path()));
    assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
    cleanup(&dir);
}

#[test]
fn overwrite_of_a_missing_file_is_a_plain_create() {
    let dir = test_dir("overwrite-missing");
    let file = dir.join("foo.sh");

    let receipt = publish_overwrite(&file, b"new\n").unwrap();
    assert_eq!(receipt.published_to, file);
    assert_eq!(receipt.previous, None);
    assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
    cleanup(&dir);
}

#[test]
#[cfg(not(target_os = "linux"))]
fn overwrite_fails_closed_when_detection_is_unavailable() {
    let dir = test_dir("overwrite-unsupported");
    let file = dir.join("foo.sh");
    fs::write(&file, "old\n").unwrap();

    match publish_overwrite(&file, b"new\n") {
        Err(SafePublishError::Indeterminate { path, reason }) => {
            assert_eq!(path, file);
            assert!(!reason.is_empty());
        }
        other => panic!("expected Indeterminate, got {other:?}"),
    }
    cleanup(&dir);
}

#[test]
fn versioned_publish_of_a_new_file_uses_the_canonical_name() {
    let dir = test_dir("versioned-new");
    let file = dir.join("foo.sh");

    let receipt = publish_versioned(&file, b"v1\n").unwrap();
    assert_eq!(receipt.published_to, file);
    assert_eq!(receipt.previous, None);
    assert_eq!(fs::read_to_string(&file).unwrap(), "v1\n");
    cleanup(&dir);
}

#[test]
fn versioned_publish_increments_across_calls() {
    let dir = test_dir("versioned-increment");
    let file = dir.join("foo.sh");
    fs::write(&file, "v1\n").unwrap();

    let first = publish_versioned(&file, b"v2\n").unwrap();
    let second = publish_versioned(&file, b"v3\n").unwrap();
    assert_eq!(first.published_to, dir.join("foo-2.sh"));
    assert_eq!(second.published_to, dir.join("foo-3.sh"));
    assert_eq!(fs::read_to_string(&dir.join("foo-2.sh")).unwrap(), "v2\n");
    assert_eq!(fs::read_to_string(&dir.join("foo-3.sh")).unwrap(), "v3\n");
    cleanup(&dir);
}

#[test]
#[cfg(target_os = "linux")]
fn versioned_publish_leaves_the_running_file_untouched() {
    let dir = test_dir("versioned-held");
    let file = dir.join("iw-issue.sh");
    fs::write(&file, "v1\n").unwrap();

    let holder = Holder::spawn(&dir, &file);
    let receipt = publish_versioned(&file, b"v2\n").unwrap();
    assert_eq!(receipt.published_to, dir.join("iw-issue-2.sh"));
    assert_eq!(receipt.previous.as_deref(), Some(file.as_path()));
    assert_eq!(fs::read_to_string(&receipt.published_to).unwrap(), "v2\n");
    // The file the reader is streaming is byte-identical.
    assert_eq!(fs::read_to_string(&file).unwrap(), "v1\n");

    drop(holder);
}

#[test]
fn publish_errors_carry_the_path_and_are_displayable() {
    let err = SafePublishError::Refused {
        path: PathBuf::from("/x/iw-issue.sh"),
        holders: vec![OpenFileHolder { pid: 4242, fd: 3 }],
    };
    let message = err.to_string();
    assert!(message.contains("/x/iw-issue.sh"), "{message}");
    assert!(message.contains("1"), "{message}");

    // A missing target directory is a plain Io error, not a panic.
    let missing = test_dir("io-error");
    let nested = missing.join("no").join("such").join("dir.sh");
    match publish_overwrite(&nested, b"x") {
        Err(SafePublishError::Io {
            operation,
            path,
            source: _,
        }) => {
            // The failure is reported at the temp file, which lives in the
            // target's (missing) directory.
            assert_eq!(path.parent(), nested.parent());
            assert!(!operation.is_empty());
        }
        other => panic!("expected Io error, got {other:?}"),
    }
    cleanup(&missing);
}

// ── #4569: the suite asserts its own process hygiene ────────────────────────

#[test]
#[cfg(target_os = "linux")]
fn a_panicking_test_still_reaps_its_holder_and_leaves_no_zombie() {
    // The incident: the old tests reaped the follower explicitly at the end
    // of the body, so a failed assertion left a `tail -f` running on a
    // deleted directory — one survived 6h50m. With the guard, the unwind
    // path does the killing and waiting.
    let dir = test_dir("panic-reap");
    let file = dir.join("script.sh");
    fs::write(&file, "echo one\n").unwrap();

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let holder = Holder::spawn(&dir, &file);
        // Simulated assertion failure with the holder in flight: the guard
        // drops here, on the unwind path.
        assert!(false, "simulated failure with the holder in flight");
        drop(holder);
    }));
    assert!(
        panicked.is_err(),
        "the simulated failure must have panicked"
    );

    assert!(
        !dir.exists(),
        "the guard removes the directory on the unwind path"
    );
    assert_eq!(
        defunct_children_of_self(),
        0,
        "a reaped holder must leave no defunct child under the test binary"
    );
}

/// The children of this test binary that have exited without being waited
/// on. A completed test must leave zero of them: a child that is never
/// `wait`ed stays a zombie for the parent's lifetime.
#[cfg(target_os = "linux")]
fn defunct_children_of_self() -> u32 {
    let mut count = 0;
    let tasks = match fs::read_dir("/proc/self/task") {
        Ok(tasks) => tasks,
        Err(_) => return 0,
    };
    for task in tasks.flatten() {
        let children = match fs::read_to_string(task.path().join("children")) {
            Ok(children) => children,
            Err(_) => continue,
        };
        for pid in children.split_whitespace() {
            let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
                Ok(stat) => stat,
                Err(_) => continue,
            };
            // Field 3 (state) follows the parenthesized comm, which may
            // contain spaces: take everything after the last ')'.
            let state = stat
                .rsplit_once(')')
                .map(|(_, rest)| rest.chars().next())
                .flatten();
            if state == Some('Z') {
                count += 1;
            }
        }
    }
    count
}
