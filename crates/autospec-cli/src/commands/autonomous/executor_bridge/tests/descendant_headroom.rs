//! The descendant-capture descriptor-headroom case, split out of
//! `descendant_spawn` so that module stays under the size threshold: the
//! test that needs the `DescendantLeader` fixture is the only user of it.

use super::support_base::test_environment;
use crate::commands::autonomous::executor_bridge as bridge;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
#[test]
fn executor_supervision_descendant_capture_reserves_descriptor_headroom() {
    // Break caught: a wide real process tree exhausted RLIMIT_NOFILE while descendant pidfds
    // were retained, so the fail-closed path itself had no descriptor left. Capture must stop
    // with the reserve still free instead of walking into EMFILE.
    let _environment = test_environment();
    // The leader (and its 12 sleeps) must not outlive the test on a panic
    // path: the old best-effort `wait` at the end of the body never ran when
    // an assertion failed first, leaving the leader behind as a defunct
    // child of the test binary (#4569).
    let leader = DescendantLeader::spawn();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut observed = 0usize;
    while Instant::now() < deadline {
        observed = bridge::process_table_entries()
            .expect("read process table")
            .into_iter()
            .filter(|(parent, _)| *parent == leader.id())
            .count();
        if observed >= 12 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        observed >= 12,
        "descendant tree never materialized: {observed}"
    );

    let mut set =
        bridge::OwnedProcessSet::from_forked_child(leader.id()).expect("capture tree leader");
    // The ceiling is the open count plus one reserve (32) plus four: the
    // capture opens one pidfd per descendant, so the budget check is meant
    // to fire with the full reserve still free.
    let ceiling = bridge::open_descriptor_count().expect("open descriptor count")
        + bridge::DESCENDANT_DESCRIPTOR_RESERVE
        + 4;
    bridge::set_descriptor_limit_override(ceiling);
    let constrained = set.capture_descendants_while_leader_live();
    bridge::set_descriptor_limit_override(0);

    let error = constrained.expect_err("a constrained descriptor budget must fail closed");
    assert!(error.contains("descriptor budget"), "{error}");
    // The assertion reads the number the budget check itself saw, embedded
    // in the error and read atomically at the moment of the check. The old
    // form re-read `free_descriptor_slots()` afterwards, which races the fd
    // churn of sibling tests in this binary: the check fires at 32 free,
    // and any descriptor a sibling opens between the check and the re-read
    // shows up as 31 — the verdict then depended on which targets ran beside
    // this one (#4557: green in a 225-target workspace run, red when the
    // target ran alone, same commit).
    let observed = error
        .split("budget: ")
        .nth(1)
        .and_then(|rest| rest.split(" slots free").next())
        .and_then(|digits| digits.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("cannot read the observed free slots: {error}"));
    assert!(
        observed <= bridge::DESCENDANT_DESCRIPTOR_RESERVE,
        "the budget check fired above the reserve: {observed}"
    );

    // The headroom the reserve exists for, proven the way it is real: after
    // the fail-closed stop the process can still open descriptors and
    // capture the whole tree unconstrained. That is the EMFILE-proof the
    // old absolute re-read was trying, and failing, to show.
    let mut guard = bridge::AdoptedProcessGuard::new(set);
    guard
        .processes_mut()
        .capture_descendants_while_leader_live()
        .expect("recapture the unconstrained tree");
    guard.terminate().expect("terminate the descendant tree");
}

/// A descendant-tree leader that owns its own termination (#4569): dropping
/// it — on a finished test, a failed assertion, or a panic — kills the
/// leader and waits for it, so it never outlives the test as a defunct
/// child of the test binary. The sleeps it forked are its children and
/// self-limit to 30s; the leader is the one that would zombie.
struct DescendantLeader {
    child: std::process::Child,
}

impl DescendantLeader {
    fn spawn() -> Self {
        Self {
            child: std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    "for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do /bin/sleep 30 & done; wait",
                ])
                .spawn()
                .expect("spawn descendant tree leader"),
        }
    }

    fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for DescendantLeader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
