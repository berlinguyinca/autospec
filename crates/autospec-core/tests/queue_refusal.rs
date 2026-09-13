//! Queue-entry validation at read time (#4536): a queue line that names work
//! but is not a positive issue number is refused with its line and reported —
//! never silently dropped, never dispatched. A job is never submitted for a
//! value that fails `^[0-9]+$` with `0` excluded.
//!
//! Also carries the dispatch-bound tick tests, moved from
//! `dispatch_pipeline.rs` so the ratchet-locked source file can shrink.

use autospec_core::dispatch_pipeline::{
    DispatchTick, EntryState, LifecycleLedger, QueueFile, SkipReason,
};

// ── Queue-entry validation (#4536) ─────────────────────────────────────────

#[test]
fn a_non_numeric_line_is_refused_with_its_line_number() {
    let file = QueueFile::parse("44\nabc\n45\n");
    assert_eq!(file.entries, vec![44, 45]);
    assert_eq!(file.refused, vec![(2, "abc".to_string())]);
}

#[test]
fn an_empty_line_names_no_work_and_is_not_a_refusal() {
    // The empty-string case, explicitly: a blank line — a value that never
    // completed writing — cannot become a dispatchable entry, and it is not
    // a refusal either (a blank line names no work).
    let file = QueueFile::parse("\n\n44\n");
    assert_eq!(file.entries, vec![44]);
    assert!(file.refused.is_empty());
    let tick = DispatchTick::run(&file, &LifecycleLedger::new());
    assert_eq!(tick.fresh_count(), 1);
}

#[test]
fn a_queue_of_only_blank_lines_dispatches_nothing_and_is_idle() {
    let file = QueueFile::parse("\n\n\n");
    assert!(file.entries.is_empty());
    assert!(file.refused.is_empty());
    let tick = DispatchTick::run(&file, &LifecycleLedger::new());
    assert!(tick.dispatched().is_empty());
    assert!(!tick.stalled());
    assert_eq!(
        tick.lines()[0],
        "dispatch tick: queue empty — nothing to dispatch"
    );
}

#[test]
fn zero_is_not_an_issue_number() {
    let file = QueueFile::parse("0\n44\n");
    assert_eq!(file.entries, vec![44]);
    assert_eq!(file.refused, vec![(1, "0".to_string())]);
}

#[test]
fn a_corrupted_line_is_refused_reported_and_never_dispatched() {
    let file = QueueFile::parse("44\n12x\n");
    let tick = DispatchTick::run(&file, &LifecycleLedger::new());
    assert_eq!(tick.dispatched().len(), 1);
    assert_eq!(tick.dispatched()[0].issue, 44);
    assert_eq!(tick.refused().to_vec(), vec![(2, "12x".to_string())]);
    let lines = tick.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("refused queue line 2") && line.contains("12x")),
        "the refusal must name the line and the content: {lines:?}"
    );
}

#[test]
fn a_queue_whose_lines_are_all_refused_is_a_stall() {
    let file = QueueFile::parse("abc\n");
    let tick = DispatchTick::run(&file, &LifecycleLedger::new());
    assert!(tick.dispatched().is_empty());
    assert!(tick.stalled());
    assert_eq!(
        tick.lines()[0],
        "dispatch tick: nothing dispatched — 1 queue line(s) refused"
    );
    let json = tick.to_json();
    assert!(
        json.contains("refused"),
        "the json carries the refusal: {json}"
    );
    assert!(json.contains("abc"));
}

#[test]
fn refusals_are_read_time_and_not_written_back() {
    // render writes queue content, not read-time findings: a refused line
    // must not survive a render round trip as if it were a real entry.
    let file = QueueFile::parse("44\n");
    let reparsed = QueueFile::parse(&file.render());
    assert_eq!(reparsed.entries, vec![44]);
    assert!(reparsed.refused.is_empty());
}

// ── Dispatch bound (#4451) ────────────────────────────────────────────────

const BOUND: u64 = 3;

fn queue_with(entries: &[u64]) -> QueueFile {
    QueueFile {
        entries: entries.to_vec(),
        ..Default::default()
    }
}

/// One dispatch cycle: run the tick over the ledger, then make the
/// tick's decisions durable.
fn dispatch_once(ledger: &mut LifecycleLedger, queue: &QueueFile, at: u64) -> DispatchTick {
    let tick = DispatchTick::run_bounded(queue, ledger, BOUND);
    tick.apply_to_ledger(ledger, at);
    tick
}

#[test]
fn fresh_dispatches_flag_in_flight_without_advancing_the_count() {
    let mut ledger = LifecycleLedger::new();
    let queue = queue_with(&[101, 102]);

    let tick = dispatch_once(&mut ledger, &queue, 100);
    assert_eq!(tick.fresh_count(), 2);
    // A running dispatch has not failed yet: only a no-patch outcome
    // advances the count.
    assert_eq!(ledger.attempts_of(101), 0);
    assert_eq!(ledger.in_flight_since(101), Some(100));

    // The next tick must not redispatch a run that is still going: the
    // "no patch because in flight" state is explicit, not inferred from
    // the absence of a file.
    let next = DispatchTick::run_bounded(&queue, &ledger, BOUND);
    assert_eq!(next.in_flight_count(), 2);
    assert!(next.dispatched().is_empty());
    assert_eq!(
        next.skipped()[0].reason,
        SkipReason::InFlight { dispatched_at: 100 }
    );
    assert_eq!(
        next.lines()[0],
        "dispatch tick: nothing dispatched — 2 entries skipped (2 in flight)"
    );
    // Nothing new to record: a second pass writes nothing, and a running
    // dispatch has not failed, so the count has not advanced.
    assert_eq!(next.apply_to_ledger(&mut ledger, 110), 0);
    assert_eq!(ledger.attempts_of(101), 0);
}

#[test]
fn failed_outcomes_advance_the_count_and_clear_in_flight() {
    let mut ledger = LifecycleLedger::new();
    let queue = queue_with(&[101]);

    for (attempt, at) in [100, 110, 120].into_iter().enumerate() {
        let tick = dispatch_once(&mut ledger, &queue, at);
        assert_eq!(tick.fresh_count(), 1, "attempt {attempt} dispatches");
        // While in flight, a second tick must not redispatch the run.
        let mid = DispatchTick::run_bounded(&queue, &ledger, BOUND);
        assert_eq!(
            mid.skipped()[0].reason,
            SkipReason::InFlight { dispatched_at: at }
        );
        // The run ends without a patch.
        let count = ledger.record_failed(101, at + 5).expect("failed recorded");
        assert_eq!(count, (attempt + 1) as u64);
        assert_eq!(ledger.in_flight_since(101), None);
    }

    // Three dispatches, no patch: the fourth is refused and the entry is
    // held, with the count and the reason, not redispatched.
    let held_tick = DispatchTick::run_bounded(&queue, &ledger, BOUND);
    assert!(held_tick.dispatched().is_empty());
    assert_eq!(
        held_tick.skipped()[0].reason,
        SkipReason::AttemptBoundExceeded {
            attempts: 3,
            bound: BOUND
        }
    );
    assert_eq!(
        held_tick.lines()[0],
        "dispatch tick: nothing dispatched — 1 entries skipped (1 over dispatch bound)"
    );
    assert_eq!(held_tick.apply_to_ledger(&mut ledger, 200), 1);
    let reason = ledger
        .record_of(101)
        .and_then(|record| record.held_reason.as_deref())
        .expect("the hold is durable");
    assert!(reason.contains("3 dispatches with no patch"), "{reason}");
    assert!(reason.contains("bound 3"), "{reason}");

    // The bound is checked before the hold: every later run keeps
    // reporting the skip as over the bound, never as a plain hold.
    let again = DispatchTick::run_bounded(&queue, &ledger, BOUND);
    assert_eq!(
        again.skipped()[0].reason,
        SkipReason::AttemptBoundExceeded {
            attempts: 3,
            bound: BOUND
        }
    );
    assert_eq!(again.over_bound_count(), 1);
}

#[test]
fn release_rearms_a_bound_held_entry_with_a_fresh_budget() {
    let mut ledger = LifecycleLedger::new();
    let queue = queue_with(&[101]);
    for at in [100, 110, 120] {
        let _ = dispatch_once(&mut ledger, &queue, at);
        ledger.record_failed(101, at + 5).expect("failed recorded");
    }
    let bound_tick = DispatchTick::run_bounded(&queue, &ledger, BOUND);
    assert_eq!(bound_tick.over_bound_count(), 1);
    bound_tick.apply_to_ledger(&mut ledger, 200);

    // Triage: the operator releases the hold. The entry gets a fresh
    // budget, not a continuation of the old count.
    assert!(ledger.release(101, 210));
    assert_eq!(ledger.attempts_of(101), 0);
    let rearmed = DispatchTick::run_bounded(&queue, &ledger, BOUND);
    assert_eq!(rearmed.fresh_count(), 1);

    // Without the release, the bound holds: releasing is the only path
    // back to dispatch.
    let mut stuck = LifecycleLedger::new();
    for at in [100, 110, 120] {
        let _ = dispatch_once(&mut stuck, &queue, at);
        stuck.record_failed(101, at + 5).expect("failed recorded");
    }
    let stuck_tick = DispatchTick::run_bounded(&queue, &stuck, BOUND);
    stuck_tick.apply_to_ledger(&mut stuck, 200);
    let still_bound = DispatchTick::run_bounded(&queue, &stuck, BOUND);
    assert_eq!(
        still_bound.over_bound_count(),
        1,
        "without release the bound holds"
    );
}

#[test]
fn a_produced_stamp_resets_attempts_and_clears_in_flight() {
    let mut ledger = LifecycleLedger::new();
    let queue = queue_with(&[101]);
    dispatch_once(&mut ledger, &queue, 100);
    ledger.record_failed(101, 110).expect("failed recorded");
    assert_eq!(ledger.attempts_of(101), 1);

    // The patch lands: the no-patch loop is broken and the count is
    // evidence no longer needed.
    assert!(ledger.record(101, EntryState::Produced, 120));
    assert_eq!(ledger.attempts_of(101), 0);
    assert_eq!(ledger.in_flight_since(101), None);
    let tick = DispatchTick::run_bounded(&queue, &ledger, BOUND);
    assert_eq!(tick.convert_count(), 1);
    assert!(tick.skipped().is_empty());
}

#[test]
fn over_bound_is_reported_even_when_other_entries_dispatch() {
    let mut ledger = LifecycleLedger::new();
    let queue = queue_with(&[101, 102]);
    for at in [100, 110, 120] {
        ledger.record_failed(101, at).expect("failed recorded");
    }

    let tick = DispatchTick::run_bounded(&queue, &ledger, BOUND);
    assert_eq!(tick.fresh_count(), 1, "102 is still dispatched");
    assert_eq!(tick.over_bound_count(), 1);
    assert_eq!(
        tick.lines()[0],
        "dispatch tick: 1 dispatched (1 fresh, 0 convert), 1 skipped (1 over dispatch bound)"
    );
}

#[test]
fn hold_preserves_the_attempt_count_and_in_flight_flag() {
    let mut ledger = LifecycleLedger::new();
    assert!(ledger.record_attempt(101, 100));
    assert_eq!(ledger.record_failed(101, 105), Some(1));
    assert!(ledger.record_attempt(101, 110));
    assert_eq!(ledger.record_failed(101, 115), Some(2));

    // A manual hold preserves the failures; the in-flight flag too.
    assert!(ledger.record_attempt(101, 120));
    assert!(ledger.hold(101, "waiting on dependency", 125));
    assert_eq!(ledger.attempts_of(101), 2);
    assert_eq!(ledger.in_flight_since(101), Some(120));

    // A failed outcome advances the count and clears the flag.
    assert_eq!(ledger.record_failed(101, 130), Some(3));
    assert_eq!(ledger.in_flight_since(101), None);
}

#[test]
fn failed_is_refused_for_terminal_and_stale_stamps() {
    let mut ledger = LifecycleLedger::new();
    assert!(ledger.record(110, EntryState::Converted, 100));
    assert_eq!(ledger.record_failed(110, 110), None);

    let mut stale = LifecycleLedger::new();
    assert!(stale.record_attempt(101, 200));
    assert_eq!(stale.record_failed(101, 100), None, "stale stamp refused");
    assert_eq!(stale.attempts_of(101), 0);
}

#[test]
fn attempt_fields_survive_the_durable_form_and_absent_fields_default() {
    let mut ledger = LifecycleLedger::new();
    assert!(ledger.record_attempt(101, 100));
    assert_eq!(ledger.record_failed(101, 105), Some(1));
    assert!(ledger.record_attempt(101, 110));
    assert!(ledger.hold(
        101,
        "dispatch bound: 1 dispatches with no patch (bound 3)",
        120
    ));

    let text = ledger.to_json();
    let back = LifecycleLedger::from_json(&text).expect("ledger json parses");
    assert_eq!(ledger, back);
    assert_eq!(back.attempts_of(101), 1);
    assert_eq!(back.in_flight_since(101), Some(110));

    // A ledger written before #4451 has no attempt fields at all: it
    // parses, and the entry reads as untried, never as failed.
    let old =
        LifecycleLedger::from_json(r#"{"records":{"101":{"state":"queued","recorded_at":5}}}"#)
            .expect("pre-4451 ledger parses");
    assert_eq!(old.attempts_of(101), 0);
    assert_eq!(old.in_flight_since(101), None);
    let tick = DispatchTick::run_bounded(&queue_with(&[101]), &old, BOUND);
    assert_eq!(tick.fresh_count(), 1);
}
