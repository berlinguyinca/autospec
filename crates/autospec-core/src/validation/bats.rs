//! Bats fixture execution contract (#2568).
//!
//! Two invariants bind every fixture spawned by `autospec validate`:
//!
//! 1. **Offline lens pin.** [`super::command::bats_command`] exports
//!    `AUTOSPEC_REFINE_LENS_MODE=deterministic` for every fixture, so a
//!    script whose default lens mode is LLM-first auto (e.g.
//!    `scripts/refine-prompt.sh`) runs its deterministic template lenses and
//!    spawns zero external LLM processes. LLM-specific coverage opts in
//!    explicitly with `--lens-mode llm` plus a stubbed dispatcher on `PATH`.
//! 2. **Process-group hygiene.** Every fixture child is spawned into its own
//!    process group and registered here while it runs; an interrupt of
//!    `autospec validate` (SIGINT/SIGQUIT/SIGTERM) kills every registered
//!    group, so 100% of the fixture's descendant processes terminate with
//!    the validator instead of being orphaned.

use std::process::Command;

#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};
#[cfg(unix)]
use std::sync::Once;

/// Upper bound on concurrently tracked fixture process groups. The runner
/// bounds parallelism by job count, so 256 is unreachable in practice; the
/// registry is a lock-free fixed slot array because the interrupt handler
/// must read it from signal context where lock acquisition is forbidden.
#[cfg(unix)]
const MAX_TRACKED_GROUPS: usize = 256;

/// Pgids of fixture process groups that are alive right now. Slot value 0
/// means empty; any other value is the group id (equal to the leader's pid,
/// because each fixture is spawned with `setpgid(0, 0)`).
#[cfg(unix)]
static ACTIVE_PROCESS_GROUPS: [AtomicI32; MAX_TRACKED_GROUPS] =
    [const { AtomicI32::new(0) }; MAX_TRACKED_GROUPS];

#[cfg(unix)]
mod ffi {
    // Deliberately declared by hand instead of adding a `nix` feature to
    // autospec-core: core is compiled alone (e.g. `cargo test -p
    // autospec-core`) with only its `fs`-flavored dependencies, and the
    // final binary always links libc, so these symbols always resolve.
    extern "C" {
        pub(super) fn setpgid(pid: i32, pgid: i32) -> i32;
        pub(super) fn kill(pid: i32, sig: i32) -> i32;
        pub(super) fn signal(sig: i32, handler: usize) -> usize;
        pub(super) fn raise(sig: i32) -> i32;
    }

    pub(super) const SIG_DFL: usize = 0;
    pub(super) const SIGINT: i32 = 2;
    pub(super) const SIGQUIT: i32 = 3;
    pub(super) const SIGTERM: i32 = 15;
    pub(super) const SIGKILL: i32 = 9;
}

/// Put a fixture into its own process group before exec and make sure the
/// validator-wide interrupt handlers are installed. Runs from the executor
/// thread; the `pre_exec` closure itself only issues `setpgid(0, 0)`, which
/// is on the async-signal-safe list and therefore legal between fork and
/// exec.
#[cfg(unix)]
pub(crate) fn run_in_own_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            if ffi::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    install_interrupt_handlers();
}

#[cfg(not(unix))]
pub(crate) fn run_in_own_process_group(_command: &mut Command) {}

/// Register a live fixture process group. The returned guard unregisters the
/// group when dropped, i.e. when the fixture exits. Returns `None` (without
/// panicking the validator) if the slot table is exhausted.
#[cfg(unix)]
pub(crate) fn register_process_group(pgid: i32) -> Option<ProcessGroupRegistration> {
    for (slot, cell) in ACTIVE_PROCESS_GROUPS.iter().enumerate() {
        if cell
            .compare_exchange(0, pgid, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Some(ProcessGroupRegistration { slot, pgid });
        }
    }
    None
}

#[cfg(not(unix))]
pub(crate) fn register_process_group(_pgid: i32) -> Option<ProcessGroupRegistration> {
    Some(ProcessGroupRegistration)
}

/// Live-fixture guard: unregisters the group on drop.
#[cfg(unix)]
pub(crate) struct ProcessGroupRegistration {
    slot: usize,
    pgid: i32,
}

#[cfg(not(unix))]
pub(crate) struct ProcessGroupRegistration;

#[cfg(unix)]
impl Drop for ProcessGroupRegistration {
    fn drop(&mut self) {
        let _ = ACTIVE_PROCESS_GROUPS[self.slot].compare_exchange(
            self.pgid,
            0,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }
}

#[cfg(unix)]
fn install_interrupt_handlers() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        for sig in [ffi::SIGINT, ffi::SIGQUIT, ffi::SIGTERM] {
            unsafe {
                let _ = ffi::signal(sig, interrupt_handler as usize);
            }
        }
    });
}

/// Signal handler: kill every registered fixture process group, then restore
/// the default disposition and re-raise the same signal so the validator
/// still dies from the interrupt (correct wait status for the parent shell)
/// instead of from an ordinary exit code. Every call here
/// (`kill`, `signal`, `raise`, atomic loads) is async-signal-safe.
#[cfg(unix)]
extern "C" fn interrupt_handler(sig: i32) {
    terminate_registered_process_groups();
    unsafe {
        let _ = ffi::signal(sig, ffi::SIG_DFL);
        let _ = ffi::raise(sig);
    }
}

/// Kill every registered fixture group. A negative pid targets the whole
/// process group, so descendants that stayed in the fixture's group die
/// with it. Already-dead groups (ESRCH) are ignored.
#[cfg(unix)]
pub(crate) fn terminate_registered_process_groups() {
    for cell in ACTIVE_PROCESS_GROUPS.iter() {
        let pgid = cell.load(Ordering::SeqCst);
        if pgid > 0 {
            let _ = unsafe { ffi::kill(-pgid, ffi::SIGKILL) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::command::{bats_command, security_artifact_commands};
    use std::ffi::OsStr;
    use std::path::Path;
    use std::time::Duration;

    fn assert_lens_pinned(command: &crate::validation::command::ToolCommand) {
        let pinned = command
            .environment()
            .iter()
            .find(|(key, _)| key == OsStr::new("AUTOSPEC_REFINE_LENS_MODE"))
            .map(|(_, value)| value.clone());
        assert_eq!(
            pinned.as_deref(),
            Some(OsStr::new("deterministic")),
            "validate would dispatch a real model for a bats fixture"
        );
    }

    #[test]
    fn every_refine_contract_bats_command_pins_the_lens_offline() {
        // The refine contract runs every tests/refine/test_refine_*.bats
        // through bats_command; spot-check the suites that call
        // refine-prompt.sh without a self-contained stub.
        for suite in [
            "tests/refine/test_refine_path_security.bats",
            "tests/refine/test_refine_lens_llm.bats",
            "tests/qa/test_exhaustiveness.bats",
            "tests/release/test_release_worktree_assert.bats",
        ] {
            let command = bats_command(suite);
            assert_eq!(command.program(), Path::new("bats"));
            assert_eq!(command.args(), [OsStr::new(suite)]);
            assert_lens_pinned(&command);
        }
    }

    #[test]
    fn every_security_artifact_bats_command_pins_the_lens_offline() {
        for command in security_artifact_commands() {
            if command.program() == Path::new("bats") {
                assert_lens_pinned(&command);
            }
        }
    }

    /// Self-reexecution modes for the process-group fixture test. The
    /// re-executed test binary is filtered to this one test, so no other
    /// harness work runs in the fixture processes.
    #[cfg(unix)]
    const FIXTURE_MODE: &str = "AUTOSPEC_VALIDATE_PG_TEST_MODE";

    /// Where the sacrifice fixture writes its descendant's pid. The test
    /// harness captures test stdout into a private pipe that is lost when
    /// the fixture dies from its self-issued interrupt, so the pid must
    /// travel through the filesystem, not stdout.
    #[cfg(unix)]
    const FIXTURE_PIDFILE: &str = "AUTOSPEC_VALIDATE_PG_TEST_PIDFILE";

    #[cfg(unix)]
    fn fixture_command(exe: &Path, mode: &str) -> Command {
        // Test-harness name filter: run only this test in the child. The
        // harness registers tests by crate-relative module path, but on
        // current toolchains `module_path!()` includes the crate name, so
        // strip the leading `autospec_core::` when present.
        const FULL: &str = concat!(
            module_path!(),
            "::interrupt_terminates_all_fixture_descendants"
        );
        let filter = FULL.strip_prefix("autospec_core::").unwrap_or(FULL);
        let mut command = Command::new(exe);
        command.env(FIXTURE_MODE, mode).arg(filter);
        command
    }

    #[cfg(unix)]
    #[test]
    fn interrupt_terminates_all_fixture_descendants() {
        use std::os::unix::process::{CommandExt, ExitStatusExt};

        let exe = std::env::current_exe().expect("test binary path");
        let mode = std::env::var(FIXTURE_MODE).unwrap_or_default();

        if mode == "descendant" {
            // Long-lived leaf process. Sleep in short ticks so a failed kill
            // surfaces as a clean exit within a bounded time instead of a
            // hang.
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            return;
        }

        if mode == "sacrifice" {
            // A validator-shaped fixture: spawn one descendant of its own,
            // report its pid, register this fixture's process group (this
            // process is a group leader, so its pgid is its own pid), then
            // interrupt itself. The handler must kill the whole group
            // before this process dies.
            let descendant = fixture_command(&exe, "descendant")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("fixture descendant must start");
            std::fs::write(
                std::env::var(FIXTURE_PIDFILE).expect("pidfile path"),
                descendant.id().to_string(),
            )
            .expect("pidfile write");

            install_interrupt_handlers();
            let Some(registration) = register_process_group(std::process::id() as i32) else {
                eprintln!("process-group slot table exhausted in test fixture");
                std::process::exit(71);
            };
            let _ = registration;
            unsafe {
                ffi::raise(ffi::SIGTERM);
            }
            // Unreachable on the happy path: the handler kills the group and
            // re-raises the interrupt with the default disposition. The
            // fallback sleep only bounds a broken handler so the driver
            // fails its assertion promptly.
            std::thread::sleep(Duration::from_secs(5));
            std::process::exit(0);
        }

        // Driver: spawn the fixture in its own process group, exactly like
        // `run_in_own_process_group` does for real fixtures.
        let pidfile = std::env::temp_dir().join(format!(
            "autospec-pg-test-{}-{}.pid",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let mut fixture = fixture_command(&exe, "sacrifice");
        fixture
            .env(FIXTURE_PIDFILE, &pidfile)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        unsafe {
            fixture.pre_exec(|| {
                if ffi::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = fixture
            .spawn()
            .expect("fixture must start in its own process group");

        // Wait for the fixture to report its descendant's pid over the
        // filesystem (the harness swallows test stdout when the fixture
        // dies from its self-issued interrupt). Check the pidfile before
        // reaping: the fixture dies within milliseconds of reporting.
        let mut pid_text = String::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            pid_text = std::fs::read_to_string(&pidfile).unwrap_or_default();
            if !pid_text.trim().is_empty() && pid_text.trim().chars().all(|c| c.is_ascii_digit()) {
                break;
            }
            if let Some(status) = child.try_wait().expect("try_wait") {
                panic!("fixture died before reporting its descendant pid: {status}");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "fixture never reported its descendant pid"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let descendant_pid: i32 = pid_text.trim().parse().expect("descendant pid");

        let status = child.wait().expect("fixture must exit");
        assert!(
            status.signal().is_some(),
            "fixture must die from a signal (the interrupt or the group kill), not an ordinary exit"
        );
        let _ = std::fs::remove_file(&pidfile);

        // 100% of descendants: the grandchild must be gone too. It is
        // re-parented out of the dead fixture's process tree, so poll: a
        // brief zombie window is acceptable, a live process is not.
        let mut dead = false;
        for _ in 0..400 {
            if unsafe { ffi::kill(descendant_pid, 0) } == -1 {
                dead = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            dead,
            "a fixture descendant survived the interrupt (pid {descendant_pid})"
        );
    }
}
