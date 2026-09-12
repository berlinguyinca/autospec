//! Tests for [`autospec_core::hold_memo`] (issue #4360).
//!
//! The regression tests run in the configuration the incident required: one
//! session's `convpass.log` with 138 issues ever held, 188 HELD events, and
//! 50 redundant holds beyond the first — ~6h40m of repeated gating. #4015
//! was held six times and produced one distinct reason (the same conflict in
//! `queue_commands.rs` and `ready_queue.rs`), re-derived from scratch on six
//! separate passes; #4068 was held ten times with three distinct reasons,
//! the legitimate case the design must preserve.

use autospec_core::hold_memo::{
    format_duration, re_gate, wasted_secs, HoldRecord, OutcomeLedger, ReGateDecision,
};

/// The #4015 hold: one conflict, two named files, a recorded base sha.
fn hold_4015() -> HoldRecord {
    HoldRecord::new(
        4015,
        "patch-4015",
        "abc123",
        vec![
            "crates/autospec-cli/src/commands/queue_commands.rs".to_string(),
            "crates/autospec-cli/src/commands/ready_queue.rs".to_string(),
        ],
        "conflict in queue_commands.rs and ready_queue.rs",
    )
    .unwrap()
}

/// Record the incident's repeat offenders plus filler to reconstruct
/// 138 issues / 188 events / 50 beyond-first / 46 repeated-outcomes.
fn incident_ledger() -> OutcomeLedger {
    let mut ledger = OutcomeLedger::new();
    // #4068: 10 holds, 3 distinct reasons (the resolver was fixed twice).
    for key in ["r1", "r1", "r1", "r2", "r2", "r2", "r2", "r3", "r3", "r3"] {
        ledger.record(4068, key);
    }
    // #4015: 6 holds, 1 reason — the same conflict re-derived six times.
    for _ in 0..6 {
        ledger.record(4015, "conflict in queue_commands.rs and ready_queue.rs");
    }
    // #4198 and #4018: 4 holds, 2 reasons each.
    for key in ["x", "x", "y", "y"] {
        ledger.record(4198, key);
    }
    for key in ["p", "p", "q", "q"] {
        ledger.record(4018, key);
    }
    // #3992: 3 holds, 1 reason.
    for _ in 0..3 {
        ledger.record(3992, "build error");
    }
    // Filler: 133 issues, 161 events, one outcome each. The first 28 are
    // held twice (28 redundant); the rest once.
    for i in 0u64..133 {
        let issue = 5000 + i;
        if i < 28 {
            ledger.record(issue, "f");
            ledger.record(issue, "f");
        } else {
            ledger.record(issue, "f");
        }
    }
    ledger
}

// --- Invariant 1: record what a hold depended on ------------------------

#[test]
fn a_hold_record_without_a_patch_or_base_is_refused() {
    // A cache key with no base (or no patch) is the defect: a hold that
    // cannot say against what it was derived is refused, never defaulted.
    assert_eq!(HoldRecord::new(1, "", "abc123", vec![], "reason"), None);
    assert_eq!(HoldRecord::new(1, "   ", "abc123", vec![], "reason"), None);
    assert_eq!(HoldRecord::new(1, "patch-1", "", vec![], "reason"), None);
    assert_eq!(HoldRecord::new(1, "patch-1", "  ", vec![], "reason"), None);
}

#[test]
fn record_carries_the_base_and_the_files_the_reason_names() {
    let r = hold_4015();
    assert_eq!(r.issue, 4015);
    assert_eq!(r.patch_key, "patch-4015");
    assert_eq!(r.base_sha, "abc123");
    assert_eq!(r.depends_on.len(), 2);
    assert!(r
        .depends_on
        .contains("crates/autospec-cli/src/commands/queue_commands.rs"));
    assert!(r
        .depends_on
        .contains("crates/autospec-cli/src/commands/ready_queue.rs"));
    // Empty file names in the reason are dropped, not kept as a key.
    let r = HoldRecord::new(
        2,
        "p",
        "b",
        vec!["a.rs".to_string(), String::new(), "  ".to_string()],
        "r",
    )
    .unwrap();
    assert_eq!(r.depends_on.len(), 1);
}

// --- Invariant 2: a deterministic outcome is computed once per input -----

#[test]
fn unchanged_inputs_report_still_held() {
    // The #4015 case: the patch is unchanged, the base moved, but neither
    // of the two conflicting files did. The conflict cannot have resolved;
    // eight minutes of gating would only confirm the recorded conclusion.
    let r = hold_4015();
    let changed = vec![
        "README.md".to_string(),
        "crates/autospec-cli/src/commands/unrelated.rs".to_string(),
    ];
    let decision = re_gate(&r, "patch-4015", &changed);
    assert_eq!(
        decision,
        ReGateDecision::StillHeld {
            since_sha: "abc123".to_string()
        }
    );
    assert!(decision.is_still_held());
}

#[test]
fn a_moved_dependent_file_re_gates() {
    // The base moved and touched one of the conflicting files: the
    // conflict may have resolved (or changed shape) — re-derive.
    let r = hold_4015();
    let changed = vec!["crates/autospec-cli/src/commands/ready_queue.rs".to_string()];
    let decision = re_gate(&r, "patch-4015", &changed);
    assert_eq!(
        decision,
        ReGateDecision::ReGate {
            patch_changed: false,
            files_changed: vec!["crates/autospec-cli/src/commands/ready_queue.rs".to_string()],
        }
    );
    assert!(!decision.is_still_held());
}

#[test]
fn a_moved_unrelated_file_does_not_re_gate() {
    // The base moved, but only on files the hold does not depend on.
    let r = hold_4015();
    let changed = vec!["docs/notes.md".to_string()];
    let decision = re_gate(&r, "patch-4015", &changed);
    assert!(decision.is_still_held());
}

#[test]
fn a_changed_patch_re_gates_even_when_the_base_is_still() {
    // The redispatched agent produced fresh work: new patch key, same
    // base. Re-derive — this is the case `memo_key` (#4260) re-offers.
    let r = hold_4015();
    let decision = re_gate(&r, "patch-4015-v2", &[]);
    assert_eq!(
        decision,
        ReGateDecision::ReGate {
            patch_changed: true,
            files_changed: Vec::new(),
        }
    );
}

#[test]
fn a_changed_patch_and_a_moved_file_report_both() {
    let r = hold_4015();
    let changed = vec!["crates/autospec-cli/src/commands/queue_commands.rs".to_string()];
    let decision = re_gate(&r, "patch-4015-v2", &changed);
    assert_eq!(
        decision,
        ReGateDecision::ReGate {
            patch_changed: true,
            files_changed: vec!["crates/autospec-cli/src/commands/queue_commands.rs".to_string()],
        }
    );
    assert_eq!(
        decision.line(),
        "re-gate: patch changed; 1 dependent file(s) changed on base: \
         crates/autospec-cli/src/commands/queue_commands.rs"
    );
}

#[test]
fn a_hold_naming_no_files_depends_on_the_whole_base() {
    // A build-failure hold that names no files: over-re-gating is the
    // safe direction. An unchanged base and patch still stands; any base
    // movement re-gates.
    let r = HoldRecord::new(9, "patch-9", "base-9", vec![], "build error").unwrap();

    let unchanged = re_gate(&r, "patch-9", &[]);
    assert!(unchanged.is_still_held());

    let moved = re_gate(&r, "patch-9", &["anything.rs".to_string()]);
    assert_eq!(
        moved,
        ReGateDecision::ReGate {
            patch_changed: false,
            files_changed: vec!["anything.rs".to_string()],
        }
    );
}

// --- Invariant 3: report unchanged holds in one line --------------------

#[test]
fn still_held_reports_in_one_line() {
    let r = hold_4015();
    let decision = re_gate(&r, "patch-4015", &["README.md".to_string()]);
    // The issue's own phrasing: `still held (unchanged since <sha>)` —
    // one line, never silently skipped.
    assert_eq!(decision.line(), "still held (unchanged since abc123)");
    // The re-gated lines say why, on one line each.
    let re_gate_line = re_gate(
        &r,
        "patch-4015",
        &["crates/autospec-cli/src/commands/ready_queue.rs".to_string()],
    )
    .line();
    assert!(re_gate_line.starts_with("re-gate:"), "line: {re_gate_line}");
    assert!(
        re_gate_line.contains("ready_queue.rs"),
        "line: {re_gate_line}"
    );
}

// --- Invariant 4: count repeats and surface them ------------------------

#[test]
fn the_incident_ledger_counts_50_beyond_first_and_46_wasted() {
    let ledger = incident_ledger();
    assert_eq!(ledger.total(), 188);
    assert_eq!(ledger.subjects(), 138);
    // The issue's top-line number: holds beyond each issue's first.
    assert_eq!(ledger.beyond_first(), 50);
    // The work actually wasted: the same conclusion re-derived. The
    // legitimate re-derivations (#4068's three distinct reasons) are not
    // in this set, so 46 < 50.
    assert_eq!(ledger.distinct_outcomes(), 142);
    assert_eq!(ledger.repeated_outcomes(), 46);
    assert!(ledger.reconciles());
}

#[test]
fn the_offender_counts_match_the_table() {
    let ledger = incident_ledger();
    // issue | times held | distinct reasons
    assert_eq!((ledger.times(4068), ledger.distinct(4068)), (10, 3));
    assert_eq!((ledger.times(4015), ledger.distinct(4015)), (6, 1));
    assert_eq!((ledger.times(4198), ledger.distinct(4198)), (4, 2));
    assert_eq!((ledger.times(4018), ledger.distinct(4018)), (4, 2));
    assert_eq!((ledger.times(3992), ledger.distinct(3992)), (3, 1));
}

#[test]
fn the_4015_case_is_pure_repeat() {
    // Six holds, one reason: every hold beyond the first is wasted.
    let mut ledger = OutcomeLedger::new();
    for _ in 0..6 {
        ledger.record(4015, "conflict in queue_commands.rs and ready_queue.rs");
    }
    assert_eq!(ledger.total(), 6);
    assert_eq!(ledger.subjects(), 1);
    assert_eq!(ledger.beyond_first(), 5);
    assert_eq!(ledger.repeated_outcomes(), 5);
    assert_eq!(ledger.repeat_subjects(), vec![4015]);
    assert!(ledger.reconciles());
}

#[test]
fn a_changed_reason_is_a_legitimate_rederivation() {
    // #4068: ten holds, three distinct reasons. Nine holds are beyond the
    // first, but only seven are repeated outcomes — two were a genuinely
    // changed conclusion (the resolver was fixed twice), and those are the
    // case the design must preserve, not count as waste.
    let mut ledger = OutcomeLedger::new();
    for key in ["r1", "r1", "r1", "r2", "r2", "r2", "r2", "r3", "r3", "r3"] {
        ledger.record(4068, key);
    }
    assert_eq!(ledger.total(), 10);
    assert_eq!(ledger.subjects(), 1);
    assert_eq!(ledger.beyond_first(), 9);
    assert_eq!(ledger.distinct(4068), 3);
    assert_eq!(ledger.repeated_outcomes(), 7);
    assert!(ledger.reconciles());
}

#[test]
fn the_summary_line_names_the_offenders() {
    let ledger = incident_ledger();
    let line = ledger.line();
    assert!(line.contains("total=188"), "line: {line}");
    assert!(line.contains("beyond_first=50"), "line: {line}");
    assert!(line.contains("repeated_outcomes=46"), "line: {line}");
    assert!(line.contains("#4015 x6"), "line: {line}");
    assert!(line.contains("#4068 x10"), "line: {line}");
}

#[test]
fn an_empty_ledger_is_idle_not_broken() {
    let ledger = OutcomeLedger::new();
    assert_eq!(ledger.total(), 0);
    assert_eq!(ledger.beyond_first(), 0);
    assert_eq!(ledger.repeated_outcomes(), 0);
    assert!(ledger.repeat_subjects().is_empty());
    assert!(ledger.reconciles());
    assert_eq!(
        ledger.line(),
        "outcomes: total=0 subjects=0 beyond_first=0 repeated_outcomes=0"
    );
}

#[test]
fn the_reported_counts_are_mutually_consistent() {
    // The counts are not independent: total is both `subjects +
    // beyond_first` and `distinct_outcomes + repeated_outcomes`, and there
    // is at least one outcome per subject. A report whose numbers do not
    // line up is reporting a state that cannot exist.
    let ledger = incident_ledger();
    assert!(ledger.reconciles());
    assert_eq!(ledger.subjects() + ledger.beyond_first(), ledger.total());
    assert_eq!(
        ledger.distinct_outcomes() + ledger.repeated_outcomes(),
        ledger.total()
    );
    assert!(ledger.distinct_outcomes() >= ledger.subjects());

    // A single subject with all-repeated outcomes is the degenerate case:
    // beyond_first and repeated_outcomes coincide.
    let mut slim = OutcomeLedger::new();
    for _ in 0..5 {
        slim.record(4015, "same");
    }
    assert!(slim.reconciles());
    assert_eq!(slim.beyond_first(), 4);
    assert_eq!(slim.repeated_outcomes(), 4);
}

// --- The generalised measure: wasted time ------------------------------

#[test]
fn wasted_time_matches_the_incident() {
    // 50 redundant holds at ~8 minutes each is ~6h40m of gating.
    let secs = wasted_secs(50, 8 * 60);
    assert_eq!(secs, 24000);
    assert_eq!(format_duration(secs), "6h40m");
    // The true waste (46 repeated outcomes) is a little less.
    assert_eq!(format_duration(wasted_secs(46, 8 * 60)), "6h8m");
}

#[test]
fn format_duration_shapes() {
    assert_eq!(format_duration(0), "0s");
    assert_eq!(format_duration(45), "45s");
    assert_eq!(format_duration(8 * 60), "8m");
    assert_eq!(format_duration(8 * 60 + 30), "8m30s");
    assert_eq!(format_duration(6 * 3600), "6h");
    assert_eq!(format_duration(6 * 3600 + 40 * 60), "6h40m");
    assert_eq!(format_duration(6 * 3600 + 40 * 60 + 5), "6h40m5s");
    // saturated multiply never panics on overflow-sized inputs
    assert_eq!(wasted_secs(0, u64::MAX), 0);
}
