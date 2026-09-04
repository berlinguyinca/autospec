// executor_bridge tests: shim publication (issue #3495).
//
// Split out of support_base.rs rather than added to it: that module is already past
// the size gate's ceiling, and an oversized file may not grow (see the note in
// tests.rs). Every fixture in this tree that writes a `#!/bin/sh` stand-in for `gh`,
// `codex`, a reviewer, or a dispatcher and then runs it publishes it through here.

#[cfg(unix)]
use super::support_base::test_root;
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::{Duration, Instant};

/// Serial number for staged shim names, so two threads staging the same final
/// path never collide on the intermediate one.
#[cfg(unix)]
static SHIM_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Publish an executable test fixture at `path` with mode 0o755.
pub(super) fn write_executable(path: &Path, body: &str) {
    publish_executable(path, body.as_bytes(), 0o755);
}

/// Publish an executable test fixture at `path` with an explicit mode.
///
/// Takes bytes so a caller can restore a fixture body it captured with `fs::read`.
pub(super) fn write_executable_mode(path: &Path, body: &[u8], mode: u32) {
    publish_executable(path, body, mode);
}

/// Publish `body` at `path` as an executable without this process ever holding a
/// write descriptor on the published inode.
///
/// `execve(2)` fails with `ETXTBSY` ("Text file busy") while *any* open file
/// description still has the target inode open for writing, and `open(2)` for write
/// fails the same way while the inode is being executed. In a multi-threaded test
/// binary neither condition is under the writing thread's control: `Command::spawn`
/// forks, and the child inherits a duplicate of every descriptor the parent holds at
/// fork time — `O_CLOEXEC` only takes effect at `execve`, not at `fork`. So a shim
/// written with `fs::write` can have its write descriptor pinned open inside an
/// unrelated child spawned by another test, and the exec that follows fails with
/// "Text file busy" no matter how unique the path is.
///
/// That is why per-case path derivation does not fix this: `test_root` already
/// derives a unique root from pid plus an atomic sequence, and the failure still
/// reproduced — on `main`, twice in one `cargo test --workspace` run, once in each of
/// the two binaries this module is compiled into.
///
/// Staging the write in a short-lived child process and renaming the finished file
/// into place removes both halves:
///
/// * the parent never opens the published inode for writing, so no concurrently
///   forked child can inherit a write descriptor on it, and
/// * the published path is created by `rename(2)`, so a path that is currently being
///   executed is never reopened for write.
///
/// Measured on this repository with 4 writer threads x 800 publish-then-exec
/// iterations against 8 continuously spawning threads, unique paths throughout: 933
/// `ETXTBSY` failures using `fs::write`, 598 using an in-process write plus `rename`,
/// and 0 using the staging below. Renaming alone is not enough — it changes which
/// name the inode is published under, not the fact that the parent opened that inode
/// for writing — which is why the write happens out of process.
#[cfg(unix)]
fn publish_executable(path: &Path, body: &[u8], mode: u32) {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

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
    // `fs::metadata(..).permissions().mode()` hands back the full `st_mode`, file-type
    // bits included, so a caller restoring a captured mode passes 0o100755 rather than
    // 0o755. Mask to the permission bits before chmod sees them.
    let permissions = mode & 0o7777;
    // $1 staged path, $2 body, $3 published path, $4 octal mode. `printf '%s'` does
    // not interpret escapes in its argument, so the body lands byte for byte, and the
    // `mv` is a same-directory rename.
    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg("set -eu; printf '%s' \"$2\" > \"$1\"; chmod \"$4\" \"$1\"; mv -f \"$1\" \"$3\"")
        .arg("executor-bridge-shim")
        .arg(&staged)
        .arg(OsStr::from_bytes(body))
        .arg(path)
        .arg(format!("{permissions:o}"))
        .status()
        .expect("publish executable fixture");
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

/// Publish and run one shim per iteration, returning every path whose exec failed.
#[cfg(unix)]
fn publish_and_run_shims(root: &Path, worker: usize, iterations: usize) -> Vec<String> {
    let mut busy = Vec::new();
    for iteration in 0..iterations {
        let shim = root.join(format!("shim-{worker}-{iteration}"));
        write_executable(&shim, "#!/bin/sh\nexit 0\n");
        if let Err(error) = Command::new(&shim).status() {
            busy.push(format!("{}: {error}", shim.display()));
        }
        let _ = fs::remove_file(&shim);
    }
    busy
}

/// Regression for the exec half of #3495: a shim published here must run even while
/// sibling threads are forking. Against `fs::write` this fails within a few
/// iterations, because those forks inherit the writer's descriptor.
///
/// The publishing threads are each other's fork traffic — no dedicated spawn-storm
/// thread — so the case proves the property without loading the rest of the suite
/// down while it runs.
#[cfg(unix)]
#[test]
fn executor_bridge_shims_execute_while_sibling_threads_spawn() {
    let root = test_root("shim-exec-race");
    let writers = (0..4)
        .map(|worker| {
            let root = root.clone();
            std::thread::spawn(move || publish_and_run_shims(&root, worker, 60))
        })
        .collect::<Vec<_>>();

    let busy = writers
        .into_iter()
        .flat_map(|writer| writer.join().expect("shim writer thread"))
        .collect::<Vec<_>>();
    assert!(
        busy.is_empty(),
        "published shim failed to execute: {busy:?}"
    );
}

/// Pins the second property of the publication contract: republishing a shim whose
/// earlier body is still running must succeed, and the next run must get the new
/// body. `prepared_draft_transaction` builds two adapters over one fixture root, so
/// the second `draft_pr_adapter_fixture` rewrites the `gh` the first one just ran.
///
/// Unlike the exec half above, this case also passes against a plain `fs::write`:
/// `execve` only denies writes to the file it maps as program text, and for a
/// `#!/bin/sh` fixture that is the interpreter, not the script. The residual
/// same-path hazard is the narrow window inside `execve` itself, which the rename
/// removes by construction. It is kept as a behavioural guard on the helper, not as
/// a reproduction of the reported failure.
#[cfg(unix)]
#[test]
fn executor_bridge_shims_republish_over_a_running_shim() {
    let root = test_root("shim-republish");
    let shim = root.join("gh");
    let started = root.join("started");
    let hold = root.join("hold");
    fs::write(&hold, "").expect("hold marker");
    write_executable(
        &shim,
        &format!(
            "#!/bin/sh\n: > '{}'\nwhile [ -e '{}' ]; do sleep 0.05; done\n",
            started.display(),
            hold.display()
        ),
    );

    let mut running = Command::new(&shim).spawn().expect("run published shim");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(started.exists(), "published shim never started");

    write_executable(&shim, "#!/bin/sh\nexit 0\n");

    fs::remove_file(&hold).expect("release hold marker");
    running.wait().expect("reap published shim");
    assert!(
        Command::new(&shim)
            .status()
            .expect("run republished shim")
            .success(),
        "republished shim must be the new body"
    );
}
