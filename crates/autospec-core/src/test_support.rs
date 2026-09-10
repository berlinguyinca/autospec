//! Test-only helper for publishing a fixture executable (issue #3500).
//!
//! Enabled by the `test-support` feature so that `autospec-cli`'s `src` unit
//! tests and its integration test binaries share ONE publisher implementation
//! instead of each carrying a local copy of the write-then-exec staging dance
//! (the ETXTBSY fix from #3495). This module is never part of the production
//! `autospec` binary: it is only compiled when `test-support` is enabled, which
//! happens only in `autospec-cli` test builds (via its `[dev-dependencies]`).

#[cfg(not(unix))]
use std::fs;
#[cfg(unix)]
use std::io::Write;
use std::path::Path;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};

/// Serial number for staged fixture names, so two threads staging the same
/// final path never collide on the intermediate one.
#[cfg(unix)]
static SHIM_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Publish an executable test fixture at `path` with mode `0o755` — the common
/// `gh`/`git`/`autospec` shell-script wrapper case.
pub fn write_executable(path: &Path, body: &str) {
    publish_executable(path, body.as_bytes(), 0o755);
}

/// Publish an executable test fixture at `path` with mode `0o755`, taking the
/// body as bytes so a caller can restore a fixture it captured with `fs::read`.
pub fn write_executable_bytes(path: &Path, body: &[u8]) {
    publish_executable(path, body, 0o755);
}

/// Publish an executable test fixture at `path` with an explicit octal `mode`.
///
/// Takes bytes so a caller can restore a fixture body it captured with `fs::read`
/// together with a mode it captured from `fs::metadata(..).permissions().mode()`
/// (e.g. `0o100755`). The mode is masked to the permission bits before `chmod`.
pub fn write_executable_mode(path: &Path, body: &[u8], mode: u32) {
    publish_executable(path, body, mode);
}

/// Publish `body` at `path` as an executable without this process ever holding a
/// write descriptor on the published inode.
///
/// `execve(2)` fails with `ETXTBSY` ("Text file busy") while *any* open file
/// description still has the target inode open for writing, and `open(2)` for
/// write fails the same way while the inode is being executed. In a
/// multi-threaded test binary neither condition is under the writing thread's
/// control: `Command::spawn` forks, and the child inherits a duplicate of every
/// descriptor the parent holds at fork time — `O_CLOEXEC` only takes effect at
/// `execve`, not at `fork`. So a fixture written with `fs::write` can have its
/// write descriptor pinned open inside an unrelated child spawned by another
/// test, and the exec that follows fails with "Text file busy" no matter how
/// unique the path is.
///
/// Staging the write in a short-lived child process and renaming the finished
/// file into place removes both halves:
///
/// * the parent never opens the published inode for writing, so no concurrently
///   forked child can inherit a write descriptor on it, and
/// * the published path is created by `rename(2)`, so a path that is currently
///   being executed is never reopened for write.
///
/// Measured in this repository with 4 writer threads x 800 publish-then-exec
/// iterations against 8 continuously spawning threads, unique paths throughout:
/// 933 `ETXTBSY` failures using `fs::write`, 598 using an in-process write plus
/// `rename`, and 0 using the staging below. Renaming alone is not enough — it
/// changes which name the inode is published under, not the fact that the parent
/// opened that inode for writing — which is why the write happens out of process.
#[cfg(unix)]
fn publish_executable(path: &Path, body: &[u8], mode: u32) {
    let mut staged_name = path
        .file_name()
        .expect("executable fixture file name")
        .to_os_string();
    staged_name.push(format!(
        ".staged-{}-{}",
        std::process::id(),
        SHIM_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let staged = path.with_file_name(staged_name);
    // `fs::metadata(..).permissions().mode()` hands back the full `st_mode`,
    // file-type bits included, so a caller restoring a captured mode passes
    // 0o100755 rather than 0o755. Mask to the permission bits before `chmod`
    // sees them.
    let permissions = mode & 0o7777;
    // $1 staged path, $2 published path, $3 octal mode; the body arrives on
    // STDIN, not in argv. An argv entry is a NUL-terminated C string, so a body
    // containing a NUL byte -- which every real executable does -- is rejected
    // with `InvalidInput: nul byte found in provided data` before /bin/sh is
    // ever spawned. That made this helper usable only for text bodies, and the
    // one caller that publishes a real binary (`fs::read("/bin/sleep")`) failed
    // on every run.
    //
    // `cat` copies stdin byte for byte, so the body still lands verbatim, and
    // the `mv` is a same-directory rename. The property the staging exists for
    // is preserved: the child, not the parent, opens the staged file for
    // writing, so no descriptor on the published inode is ever held here.
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("set -eu; cat > \"$1\"; chmod \"$3\" \"$1\"; mv -f \"$1\" \"$2\"")
        .arg("autospec-test-fixture")
        .arg(&staged)
        .arg(path)
        .arg(format!("{permissions:o}"))
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn executable fixture publisher");
    child
        .stdin
        .take()
        .expect("fixture publisher stdin")
        .write_all(body)
        .expect("write executable fixture body");
    let status = child.wait().expect("publish executable fixture");
    assert!(
        status.success(),
        "publish executable fixture {}: {status}",
        path.display()
    );
}

#[cfg(not(unix))]
fn publish_executable(path: &Path, body: &[u8], _mode: u32) {
    fs::write(path, body).expect("write executable fixture");
}
