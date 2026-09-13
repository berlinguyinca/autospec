//! Process-group guard for interrupted validation runs (issue #2568).
//!
//! `autospec validate` runs external fixture suites (bats → shell →
//! dispatcher) as children of the CLI process. By default those children
//! share the terminal's foreground process group, so a Ctrl-C kills
//! `validate` and the bats process *at the same instant* — and whatever the
//! fixture script had already spawned (a real LLM dispatcher, a sleep, a
//! test server) keeps running with no parent left to reap it. The historical
//! symptom: an interrupted validate left billable model processes behind,
//! and the operator had to hunt them down by hand.
//!
//! The guard inverts the ownership:
//!
//! 1. Every external check is spawned in its **own process group**
//!    (`setpgid(0, 0)` in `pre_exec`), so a terminal signal reaches
//!    `validate` alone.
//! 2. While a check is running, its process-group id is registered in a
//!    fixed-size array of atomics. That array is the only state the signal
//!    handler reads.
//! 3. On SIGINT/SIGTERM the handler SIGKILLs every registered group and
//!    re-raises the signal with the default disposition, so `validate` still
//!    exits with the conventional status (130/143) and the descendant
//!    fixture process groups are terminated — all of them, because SIGKILL
//!    cannot be caught or ignored.
//!
//! The handler is async-signal-safe by construction: it performs no
//! allocation, takes no lock, and calls only `kill` and `sigaction` (both
//! async-signal-safe per POSIX) plus atomic loads.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Condvar, Mutex};

/// How many external checks may be registered concurrently. The number of
/// concurrently *spawning* checks is bounded by the plan's external check
/// count; a `--jobs` value above this only makes a spawner wait for a free
/// slot (see [`register_group`]) rather than drop the guarantee.
const MAX_TRACKED_GROUPS: usize = 256;

/// Mirror of the live process-group ids for the signal handler. Slot values
/// are process-group ids; 0 marks an empty slot (group ids are positive).
static LIVE_GROUPS: [AtomicI32; MAX_TRACKED_GROUPS] =
    [const { AtomicI32::new(0) }; MAX_TRACKED_GROUPS];

/// Normal-thread bookkeeping for slot allocation. The handler never touches
/// this; it only reads [`LIVE_GROUPS`].
static SLOT_STATE: Mutex<Vec<i32>> = Mutex::new(Vec::new());
static SLOT_FREE: Condvar = Condvar::new();

/// Keeps one process-group registration alive. Dropping it (the check
/// finished, or the spawn failed) clears the slot so the handler stops
/// targeting a dead group.
pub struct GroupGuard {
    slot: usize,
    pgid: i32,
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        LIVE_GROUPS[self.slot].store(0, Ordering::SeqCst);
        let mut slots = SLOT_STATE
            .lock()
            .expect("interrupt guard slot state poisoned");
        if let Some(position) = slots.iter().position(|pgid| *pgid == self.pgid) {
            slots.swap_remove(position);
        }
        SLOT_FREE.notify_one();
    }
}

/// Registers a process group (the pid of a child that called
/// `setpgid(0, 0)`, so pid == pgid). Blocks while every slot is held, so the
/// termination guarantee never degrades under concurrency.
pub fn register_group(pgid: i32) -> Option<GroupGuard> {
    if pgid <= 0 {
        return None;
    }
    let mut slots = SLOT_STATE
        .lock()
        .expect("interrupt guard slot state poisoned");
    loop {
        if slots.len() < MAX_TRACKED_GROUPS {
            break;
        }
        // Every slot is held: wait for a check to finish and clear its slot.
        slots = SLOT_FREE
            .wait_while(slots, |slots| slots.len() >= MAX_TRACKED_GROUPS)
            .expect("interrupt guard slot state poisoned");
    }
    slots.push(pgid);
    let slot = slots.len() - 1;
    drop(slots);
    LIVE_GROUPS[slot].store(pgid, Ordering::SeqCst);
    Some(GroupGuard { slot, pgid })
}

/// SIGKILLs every currently registered fixture process group.
///
/// Exposed separately from the handler so tests can prove the kill behaviour
/// without killing the test process itself.
#[cfg(unix)]
pub fn kill_all_live_groups() {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    for slot in &LIVE_GROUPS {
        let pgid = slot.load(Ordering::SeqCst);
        if pgid > 0 {
            // A group that already exited is not an error.
            let _ = killpg(Pid::from_raw(pgid), Signal::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
pub fn kill_all_live_groups() {}

/// Installs the SIGINT/SIGTERM guard. Returns `false` when the platform
/// cannot provide process groups (non-unix) — callers treat that as "no
/// guarantee", which is also the historical behaviour there.
#[cfg(unix)]
pub fn install() -> bool {
    use nix::sys::signal::{kill, sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
    use nix::unistd::getpid;

    extern "C" fn on_interrupt(signum: i32) {
        // Async-signal-safe: atomics + killpg/kill + sigaction only. The only
        // values signum can take are the two this handler is installed for.
        kill_all_live_groups();
        let kind = match signum {
            nix::libc::SIGINT => Signal::SIGINT,
            nix::libc::SIGTERM => Signal::SIGTERM,
            _ => return,
        };
        // Restore the default disposition and re-raise, so the process dies
        // with the conventional signal status (130 for SIGINT, 143 for
        // SIGTERM) exactly as if no handler were installed.
        let default = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        // SAFETY: inside the async-signal-safe handler; `sigaction` and `kill`
        // are raw syscalls that take no locks and allocate nothing, so restoring
        // SIG_DFL and re-raising the fatal signal is safe in this context.
        unsafe {
            let _ = sigaction(kind, &default);
            let _ = kill(getpid(), kind);
        }
    }

    for kind in [Signal::SIGINT, Signal::SIGTERM] {
        let action = SigAction::new(
            SigHandler::Handler(on_interrupt),
            SaFlags::SA_NODEFER,
            SigSet::empty(),
        );
        // SAFETY: `sigaction` with a `SigHandler::Handler` is the only way to
        // receive SIGINT/SIGTERM; the handler is async-signal-safe (see above),
        // so registering it is sound.
        if unsafe { sigaction(kind, &action) }.is_err() {
            return false;
        }
    }
    true
}

#[cfg(not(unix))]
pub fn install() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_release_roundtrip() {
        let guard = register_group(4321).expect("a free slot is available at low load");
        let slot = guard.slot;
        assert_eq!(LIVE_GROUPS[slot].load(Ordering::SeqCst), 4321);
        drop(guard);
        assert_eq!(LIVE_GROUPS[slot].load(Ordering::SeqCst), 0);
    }

    #[test]
    fn kill_all_live_groups_is_a_noop_when_idle() {
        // Must not panic or signal anything when no group is registered.
        kill_all_live_groups();
    }

    #[test]
    fn install_is_idempotent() {
        assert!(install());
        assert!(install());
    }
}
