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
