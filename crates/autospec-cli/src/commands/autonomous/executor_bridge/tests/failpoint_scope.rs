// executor_bridge tests: failpoint thread scope — issue #3951.
//
// The consume-once failpoints are thread-scoped: a failpoint armed on one
// test thread must be invisible to every other parallel test thread. This
// test pins that property directly so a regression to a process-global
// `AtomicU8` fails loudly instead of flaking a random sibling test.

use crate::commands::autonomous::executor_bridge as bridge;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

#[test]
fn autonomous_executor_bridge_failpoints_are_thread_scoped() {
    let armed = Arc::new(AtomicBool::new(false));

    // Producer thread: arm the failpoint on its own thread, then confirm it
    // observes its own arm.
    let armed_producer = Arc::clone(&armed);
    let producer = thread::spawn(move || {
        bridge::BASE_DRIFT_FAILPOINT.with(|fp| fp.store(1));
        armed_producer.store(true, Ordering::SeqCst);
        bridge::BASE_DRIFT_FAILPOINT.with(|fp| fp.load() == 1)
    });

    // Wait for the producer to arm before asserting on this thread.
    while !armed.load(Ordering::SeqCst) {
        thread::yield_now();
    }

    assert_eq!(
        bridge::BASE_DRIFT_FAILPOINT.with(|fp| fp.load()),
        0,
        "thread-local failpoint leaked across threads"
    );

    assert!(
        producer.join().expect("producer thread must not panic"),
        "producer thread must observe its own failpoint arm"
    );
}

/// Parallel runs must produce identical results (issue #3778, AC4): four worker
/// threads concurrently arm the one-shot zero-effect-recovery failpoint on their own
/// threads and drive the exact consumer boundary, requiring the injected crash every
/// time. Under the old process-global `AtomicU8` cells a sibling thread's consumer
/// could swap the shared cell back to zero before this thread's boundary ran, silently
/// turning the crash into the success path. With thread-scoped cells every arm fires
/// on its own thread, no matter how many test threads run in parallel.
/// Arm one failpoint boundary on the current thread and require the injected
/// crash, proving this thread's arm was not consumed by a sibling thread.
fn drive_failpoint_once(boundary: bridge::ZeroEffectRecoveryFailpoint, label: &str) {
    bridge::ZERO_EFFECT_RECOVERY_FAILPOINT.with(|fp| fp.store(boundary as u8));
    bridge::zero_effect_recovery_failpoint(boundary, label)
        .expect_err("concurrent arm from another thread must not consume this fault");
}

#[test]
fn autonomous_executor_bridge_concurrent_arms_fire_on_their_own_thread() {
    let mut handles = Vec::new();
    for _ in 0..4 {
        handles.push(thread::spawn(|| {
            for _ in 0..32 {
                drive_failpoint_once(
                    bridge::ZeroEffectRecoveryFailpoint::AfterRepair,
                    "after repair",
                );
                drive_failpoint_once(
                    bridge::ZeroEffectRecoveryFailpoint::AfterTransfer,
                    "after transfer",
                );
            }
        }));
    }
    for handle in handles {
        handle.join().expect("worker thread must not panic");
    }
}
