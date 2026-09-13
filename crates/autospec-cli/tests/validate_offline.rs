//! Issue #2568 — validator environment isolation and interrupt cleanup.
//!
//! All coverage here is Rust. Three properties are proven, end to end:
//!
//! 1. **Offline**: the refine-contract check that `autospec validate` runs
//!    completes with **zero external LLM processes**: sentinel `claude` and
//!    `codex` dispatchers sit on PATH, and any dispatch attempt would append
//!    to a log the test asserts is empty.
//! 2. **Non-vacuity control**: the same refine invocation in `auto`
//!    (LLM-first) mode, without validate's lens pin, DOES reach the sentinels
//!    — so test 1's empty log means "the pin worked", not "nothing would
//!    ever dispatch".
//! 3. **Interrupt cleanup**: interrupting the real `autospec validate`
//!    binary terminates every descendant fixture process group: it runs
//!    with a stand-in `bats` that spawns a grandchild and stays alive; the
//!    test sends SIGINT to the binary and asserts it exits with the
//!    conventional signal status and the grandchild is dead.
//!
//! Test 1 mutates the test *process* environment (the check runs
//! in-process), so it takes `ENV_LOCK`. Tests 2 and 3 pass everything
//! through the child's own environment and are safe to run concurrently.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use autospec_core::test_support::write_executable;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

/// Removes its directory on drop so a failing assert cannot leak fixtures.
struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn unique_tmp_dir(prefix: &str) -> (DirGuard, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("create scratch dir");
    (DirGuard(dir.clone()), dir)
}

/// Sentinel dispatchers live under $HOME, not a tmpdir, on purpose: the
/// dispatcher path-safety rule (itself a test target of the refine
/// path-security suite) refuses tmpdir dispatchers, so a tmpdir sentinel
/// could never observe a dispatch and the empty-log assertion would be
/// vacuous. No AUTOSPEC_LLM_DISPATCHER override is used. The invocation log
/// path is baked into the script so the sentinels need no environment.
fn sentinel_dir() -> (DirGuard, PathBuf, PathBuf) {
    let home = std::env::var("HOME").expect("HOME is set on unix hosts");
    let dir = Path::new(&home).join(format!(
        ".autospec-offline-sentinel-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("create sentinel dir under HOME");
    let invocation_log = dir.join("invocations.log");
    let script = format!(
        "#!/bin/bash\necho \"$0\" >> \"{}\"\n",
        invocation_log.display()
    );
    write_executable(&dir.join("claude"), &script);
    write_executable(&dir.join("codex"), &script);
    (DirGuard(dir.clone()), dir, invocation_log)
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/autospec-cli has a parent")
        .parent()
        .expect("the workspace root is two levels up")
        .to_path_buf()
}

#[test]
fn refine_contract_bats_suite_runs_without_dispatching_an_llm() {
    let _env_lock: MutexGuard<'_, ()> = ENV_LOCK
        .lock()
        .expect("env lock poisoned by a crashed test");

    let root = repo_root();
    let (_guard, sentinel_dir, invocation_log) = sentinel_dir();

    let previous_path = std::env::var("PATH").unwrap_or_default();
    let previous_lens_mode = std::env::var("AUTOSPEC_REFINE_LENS_MODE").ok();
    std::env::set_var(
        "PATH",
        format!("{}:{}", sentinel_dir.display(), previous_path),
    );
    std::env::remove_var("AUTOSPEC_REFINE_LENS_MODE");

    let result = std::panic::catch_unwind(|| {
        autospec_core::validation::ExternalCheck::AutospecRefineContract.run(
            "check_autospec_refine_contract",
            true,
            &root,
        )
    });

    // Restore the environment before asserting so a failure report is not
    // itself a polluted-environment incident.
    std::env::set_var("PATH", previous_path);
    match previous_lens_mode {
        Some(value) => std::env::set_var("AUTOSPEC_REFINE_LENS_MODE", value),
        None => std::env::remove_var("AUTOSPEC_REFINE_LENS_MODE"),
    }

    let result = result.expect("refine contract check panicked");
    assert!(
        result.is_success(),
        "refine contract check should pass on a healthy tree, got: {result:?}"
    );
    let dispatched = fs::read_to_string(&invocation_log)
        .unwrap_or_default()
        .lines()
        .count();
    assert_eq!(
        dispatched,
        0,
        "validate's refine check dispatched a real LLM process — the generic \
         fixture must stay offline (#2568). Sentinel log:\n{}",
        fs::read_to_string(&invocation_log).unwrap_or_default()
    );
}

#[test]
fn auto_mode_without_the_pin_dispatches_the_sentinel_control() {
    // The same refine invocation shape the path-security suite uses, without
    // validate's `AUTOSPEC_REFINE_LENS_MODE=deterministic` pin: `auto`
    // resolves LLM-first and must reach the sentinel. If this ever sees an
    // empty log, the sentinel cannot observe dispatches and the offline
    // test above is vacuous.
    let root = repo_root();
    let (_guard, sentinel_dir, invocation_log) = sentinel_dir();
    let (_scratch_guard, scratch) = unique_tmp_dir("validate-offline-control");

    let previous_path = std::env::var("PATH").unwrap_or_default();
    let output = Command::new("bash")
        .arg(root.join("scripts/refine-prompt.sh"))
        .args([
            "offline control probe prompt",
            "--rounds",
            "1",
            "--dry-run",
            "--artifact-dir",
        ])
        .arg(scratch.join("refinements"))
        .arg("--repo-root")
        .arg(&scratch)
        .arg("--memory-root")
        .arg(scratch.join("memory"))
        .current_dir(&root)
        .env(
            "PATH",
            format!("{}:{previous_path}", sentinel_dir.display()),
        )
        .env_remove("AUTOSPEC_REFINE_LENS_MODE")
        .env_remove("AUTOSPEC_LLM_DISPATCHER")
        .output()
        .expect("run refine in auto mode");

    let dispatched = fs::read_to_string(&invocation_log)
        .unwrap_or_default()
        .lines()
        .count();
    assert!(
        dispatched > 0,
        "auto-mode refine never reached the sentinel dispatcher — the \
         offline test's empty-log assertion would be vacuous (#2568). \
         refine exit: {:?}, stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn interrupting_validate_kills_every_fixture_process_group() {
    let root = repo_root();
    let (_fake_guard, fake_dir) = unique_tmp_dir("validate-interrupt-fake");
    let pid_file = fake_dir.join("fake-bats.pid");
    let grandchild_file = fake_dir.join("grandchild.pid");

    // The stand-in bats: record its own pid, spawn a long-lived grandchild,
    // and stay alive so its process group remains a live fixture group.
    write_executable(
        &fake_dir.join("bats"),
        &format!(
            "#!/bin/bash\n\
             echo $BASHPID > {pid}\n\
             sleep 300 &\n\
             echo $! > {gc}\n\
             wait\n",
            pid = pid_file.display(),
            gc = grandchild_file.display(),
        ),
    );

    let previous_path = std::env::var("PATH").unwrap_or_default();
    let mut child = Command::new(env!("CARGO_BIN_EXE_autospec"))
        // The child's own environment carries the fake PATH; the test
        // process environment is untouched, so this test is parallel-safe.
        .args(["validate", "--jobs=1"])
        .current_dir(&root)
        .env("PATH", format!("{}:{previous_path}", fake_dir.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the autospec binary");

    let cleanup = |gc_pid: Option<u32>| {
        // Best effort: kill the fixture group if the assertions below failed
        // before the interrupt did its job.
        if let Some(pid) = gc_pid {
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .output();
        }
    };

    // Wait until the first fake bats is running (the pid file appears once
    // the plan reaches the first check that executes a Bats suite).
    let deadline = Instant::now() + Duration::from_secs(180);
    let grandchild = loop {
        if let Ok(text) = fs::read_to_string(&grandchild_file) {
            break text
                .trim()
                .parse::<u32>()
                .expect("grandchild pid is numeric");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "validate never reached a Bats check within 180s — the \
                 interrupt fixture was not exercised"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let fake_bats_pid = fs::read_to_string(&pid_file)
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok());

    // SIGINT to validate alone: the fixture group is separate, so only the
    // guard can reach it. This is the Ctrl-C the guard exists for.
    Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT to validate");

    let status = child.wait().expect("wait for validate to exit");
    #[cfg(unix)]
    let signal = status
        .signal()
        .expect("validate should be terminated by the re-raised SIGINT");
    #[cfg(not(unix))]
    let signal = 0;
    assert_eq!(
        signal,
        libc_sigint(),
        "validate should exit from SIGINT (conventional status 130), got {status:?}"
    );

    // The grandchild must be dead: 100% of the descendant fixture process
    // groups are terminated by the interrupt.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if process_gone(grandchild) {
            break;
        }
        if Instant::now() > deadline {
            cleanup(Some(grandchild));
            panic!(
                "grandchild {grandchild} survived the interrupted validate — \
                 the fixture process group was not terminated (#2568)"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(pid) = fake_bats_pid {
        assert!(
            process_gone(pid),
            "fake bats {pid} survived the interrupted validate"
        );
    }
}

#[cfg(unix)]
fn libc_sigint() -> i32 {
    2
}

#[cfg(not(unix))]
fn libc_sigint() -> i32 {
    0
}

#[cfg(unix)]
fn process_gone(pid: u32) -> bool {
    // kill -0 probes liveness: a successful probe means the process exists.
    let probe = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .expect("probe process liveness");
    !probe.status.success()
}
