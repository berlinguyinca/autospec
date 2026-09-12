//! Regression tests for the shared-worktree incident (issue #3608).
//!
//! Two conversion runs in one git worktree, each doing
//! `git checkout -B fix/issue-N origin/main` and `cargo test`, produced
//! fabricated failures for two patches that pass in isolation:
//!
//! | patch | verdict during the race | verdict re-run alone |
//! |---|---|---|
//! | 2605 | `build error -- test failed` | **passes** — PR #3599 |
//! | 2607 | `FAILED. 791 passed; 81 failed` | **passes** — PR #3600 |
//!
//! The tests here run in the configuration the incident required: two
//! processes, one checkout.

use autospec_core::worktree_lock::{
    acquire, contaminated_line, contaminated_verdicts, parse_lock_file, preflight,
    preflight_refusal_line, refusal_line, shared_checkout_findings, supersede, CheckoutVerdict,
    Lease, LockState, Phase, Preflight, ProcessRecord, ProcessRef, SupersedeOutcome,
};

const SHARED: &str = "/scratch/wt/shared";

fn process(pid: u32, cwd: &str, command: &str) -> ProcessRecord {
    ProcessRecord {
        pid,
        cwd: cwd.to_string(),
        command: command.to_string(),
    }
}

#[test]
fn the_detection_finds_two_processes_in_one_checkout_in_seconds() {
    let records = vec![
        process(111, SHARED, "convpass 2605"),
        process(222, SHARED, "convpass 2607"),
    ];
    // The second run's pre-flight check: one other process in the same
    // working directory is the whole defect.
    assert_eq!(
        preflight(&records, 222, SHARED),
        Preflight::Busy { pids: vec![111] }
    );
    let line = preflight_refusal_line(SHARED, &[111]);
    assert!(line.starts_with("refusing:"), "{line}");
    assert!(line.contains(SHARED), "{line}");
    assert!(line.contains("pid 111"), "{line}");

    // A worker on its own checkout is clean.
    assert_eq!(
        preflight(&records, 222, "/scratch/wt/worker-9"),
        Preflight::Clean
    );
}

#[test]
fn the_lock_refuses_rather_than_queues_and_names_the_holder() {
    let first = match acquire(SHARED, None, "convpass pid=111", 1000) {
        autospec_core::worktree_lock::AcquireOutcome::Held { lease } => lease,
        other => panic!("the first run must hold the lock: {other:?}"),
    };
    assert_eq!(first.phase, Phase::Checkout);

    let published = serde_json::to_string(&LockState {
        holder: first.holder.clone(),
        pid: 111,
        acquired_at: first.acquired_at,
    })
    .expect("lock state is serializable");
    let state = parse_lock_file(&published).expect("well-formed lock file");
    let state = state.expect("a non-empty lock file is a lock");

    match acquire(SHARED, Some(&state), "convpass pid=222", 1001) {
        autospec_core::worktree_lock::AcquireOutcome::Refused {
            holder, ref reason, ..
        } => {
            assert_eq!(holder, "convpass pid=111");
            assert_eq!(refusal_line(SHARED, &state), *reason);
            assert!(
                reason.contains("not queueing"),
                "refusal must refuse, not queue: {reason}"
            );
        }
        other => panic!("the second run must be refused: {other:?}"),
    }
}

#[test]
fn the_lock_is_held_for_the_whole_checkout_apply_test_cycle() {
    let mut lease = Lease::new(SHARED, "convpass pid=111", 111, 1000);
    // Per-command locking is the hole: releasing between checkout and
    // apply, or between apply and test, is refused with the phase named.
    let at_checkout = lease
        .release()
        .expect_err("releasing at checkout must fail");
    assert!(
        at_checkout.to_string().contains("checkout"),
        "{at_checkout}"
    );
    lease.advance().expect("checkout -> apply");
    let at_apply = lease.release().expect_err("releasing at apply must fail");
    assert!(at_apply.to_string().contains("apply"), "{at_apply}");
    lease.advance().expect("apply -> test");
    let at_test = lease.release().expect_err("releasing at test must fail");
    assert!(at_test.to_string().contains("test"), "{at_test}");
    lease.advance().expect("test -> done");
    lease
        .release()
        .expect("the completed cycle releases its lock");
}

#[test]
fn concurrent_workers_need_separate_checkouts() {
    // The incident: two workers, one checkout.
    let findings = shared_checkout_findings(&[("worker-0", SHARED), ("worker-1", SHARED)]);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].starts_with("SHARED_CHECKOUT:"), "{findings:?}");
    assert!(
        findings[0].contains("worker-0") && findings[0].contains("worker-1"),
        "{findings:?}"
    );

    // The fix: one checkout per worker.
    let ok = shared_checkout_findings(&[
        ("worker-0", "/scratch/wt/worker-0"),
        ("worker-1", "/scratch/wt/worker-1"),
    ]);
    assert!(ok.is_empty(), "{ok:?}");
}

#[test]
fn a_verdict_records_the_checkout_that_made_it_and_contamination_is_visible() {
    // A verdict without a recorded checkout is refused, never defaulted.
    assert!(CheckoutVerdict::new(2607, "passes", "", "convpass pid=222", 1200).is_none());

    // The incident's verdict: it names the checkout it ran in, so the
    // co-holder over that checkout is what identifies it.
    let verdict = CheckoutVerdict::new(
        2607,
        "FAILED. 791 passed; 81 failed",
        SHARED,
        "convpass pid=222",
        1200,
    )
    .expect("a verdict with a recorded checkout is valid");
    let co_holder = Lease::new(SHARED, "convpass pid=111", 111, 990);
    let flagged = contaminated_verdicts(
        std::slice::from_ref(&verdict),
        std::slice::from_ref(&co_holder),
    );
    assert_eq!(flagged, vec![verdict.clone()]);
    assert!(contaminated_line(&verdict, &co_holder).contains("contaminated"));

    // A verdict on a checkout nobody else held is not flagged.
    let solo = Lease::new("/scratch/wt/worker-1", "convpass pid=333", 333, 990);
    assert!(
        contaminated_verdicts(std::slice::from_ref(&verdict), std::slice::from_ref(&solo))
            .is_empty()
    );
}

#[test]
fn a_superseding_run_stops_the_old_one_first_and_says_so() {
    let old = ProcessRef {
        pid: 111,
        command: "convpass run 2605".to_string(),
    };
    let new = ProcessRef {
        pid: 222,
        command: "convpass run 2607".to_string(),
    };
    match supersede(old.clone(), new.clone(), true) {
        SupersedeOutcome::Clean { old, new, ref line } => {
            assert_eq!((old, new), (111, 222));
            assert!(line.contains("stopped before starting"), "{line}");
        }
        other => panic!("a supersede that stops the old job first is clean: {other:?}"),
    }
    match supersede(old, new, false) {
        SupersedeOutcome::StartedNewWithoutStoppingOld { old, new, ref line } => {
            assert_eq!((old, new), (111, 222));
            assert!(line.contains("still running"), "{line}");
            assert!(line.contains("stop the old job first"), "{line}");
        }
        other => panic!("starting without stopping is the incident: {other:?}"),
    }
}
