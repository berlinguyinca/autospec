//! `HeldTarget::hold_line` must name every issue involved in the hold.
//!
//! The module landed with a format string carrying two `{}` placeholders and a
//! single argument, so the crate did not compile and the conversion pass
//! recorded the patch as a build failure. A compile error is the cheap version
//! of this mistake; the expensive version is a placeholder that resolves to the
//! wrong value and prints a hold line naming one issue twice. These assertions
//! pin both identities so either failure is caught here rather than read as a
//! confusing operator message.

use autospec_core::shared_write_target::{HeldTarget, HoldReason};

#[test]
fn a_shared_target_hold_names_both_the_held_issue_and_the_one_it_waits_on() {
    let held = HeldTarget {
        issue: 4238,
        reason: HoldReason::SharedTarget {
            entry: "docs/invariants.md".to_owned(),
            waiting_on: 4151,
        },
    };

    let line = held.hold_line();

    // The held issue and the issue it waits on are different numbers, and the
    // line is only actionable if it names each one in the right place.
    assert!(
        line.contains("#4238 waits for #4151"),
        "hold line must name the held issue then the blocker: {line}"
    );
    assert!(
        line.contains("docs/invariants.md"),
        "hold line must name the contended entry: {line}"
    );
    assert!(
        line.contains("once #4151 merges"),
        "hold line must name the merge that releases it: {line}"
    );
}

#[test]
fn a_queue_depth_hold_names_the_issue_and_claims_no_blocker() {
    let held = HeldTarget {
        issue: 4238,
        reason: HoldReason::QueueDepth,
    };

    let line = held.hold_line();

    assert!(line.contains("#4238"), "must name the held issue: {line}");
    // A queue-depth hold involves no other issue; naming one would send an
    // operator looking for contention that does not exist.
    assert!(
        !line.contains("waits for #"),
        "a budget hold must not imply contention: {line}"
    );
}
