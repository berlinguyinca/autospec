//! The pass sizes its batch to its own deadline (#4607).
//!
//! No conversion pass has ever finished its batch: the batch size and the
//! deadline were chosen independently and never compared against the
//! observed cost of a single patch, so every pass ended by being killed
//! mid-gate (rc=143) after one verdict out of a dozen candidates — and the
//! candidates it never reached were re-selected and re-gated from scratch
//! on the next pass. Throughput was not low; it was zero.
//!
//! The fix is the invariant the issue names: given a deadline, the pass
//! stops *starting* new items when the remaining time is less than what an
//! item has been observed to cost, and finishes the one in flight. The cost
//! that governs the decision is the one observed in this pass — the first
//! item on a cold cache and the tenth on a warm one differ by an order of
//! magnitude, so a compiled-in constant would be wrong in exactly the two
//! regimes that matter. And a deadline is not enforced by killing the work:
//! being terminated mid-gate is indistinguishable from a hang and discards
//! everything the gate had computed.

use std::time::Duration;

/// Whether the pass should stop starting new items now.
///
/// `remaining` is the time left on the deadline (already saturated at
/// zero: a passed deadline leaves nothing, and nothing does not cover any
/// positive cost). `last_cost` is the wall time of the most recently
/// completed item in this pass, or `None` before the first item finishes.
///
/// With no observed cost there is nothing to compare the remainder
/// against, and the first item is what produces the estimate — so the
/// answer is no: a pass that has measured nothing starts its first item,
/// whatever the deadline.
pub(super) fn should_defer(remaining: Duration, last_cost: Option<Duration>) -> bool {
    match last_cost {
        Some(cost) => remaining < cost,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_observed_cost_starts_the_first_item() {
        // The deadline can already be exhausted before the first item
        // finishes; that is the in-flight item's problem, not a reason to
        // have skipped measuring anything at all.
        assert!(!should_defer(Duration::from_secs(0), None));
        assert!(!should_defer(Duration::from_secs(3600), None));
    }

    #[test]
    fn remaining_below_the_observed_cost_defers() {
        assert!(should_defer(Duration::from_secs(5), Some(Duration::from_secs(6))));
        assert!(should_defer(Duration::ZERO, Some(Duration::from_secs(1))));
    }

    #[test]
    fn remaining_at_least_the_observed_cost_starts() {
        assert!(!should_defer(Duration::from_secs(6), Some(Duration::from_secs(6))));
        assert!(!should_defer(Duration::from_secs(7), Some(Duration::from_secs(6))));
    }

    #[test]
    fn a_warm_cache_cost_defers_less_than_a_cold_one() {
        // The order-of-magnitude spread between the first (cold) item and
        // later (warm) items is why the estimate is the observed one: the
        // same remainder starts a warm item and defers a cold one.
        let remainder = Duration::from_secs(30);
        assert!(!should_defer(remainder, Some(Duration::from_secs(20))));
        assert!(should_defer(remainder, Some(Duration::from_secs(300))));
    }
}
