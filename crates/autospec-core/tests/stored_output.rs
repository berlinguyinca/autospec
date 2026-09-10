//! Stored-agent-output lifecycle, guard release paths, and blocker escalation
//! (issue #4170).
//!
//! The four acceptance scenarios, in the order the incident produced them:
//!
//! * a guard that prevents an action names the action that releases it;
//! * the patch behind the held issue no longer applies to `main`, so its
//!   output is superseded and archiving it releases the dispatch;
//! * a blocker that persists across runs escalates — it reports its age, and
//!   it is counted separately from ordinary idleness;
//! * `ready` and `dispatchable` are different counts, reported separately.

use std::time::Duration;

use autospec_core::stored_output::{
    classify, format_age, held_line, release, ApplyCheck, BlockedIssue, BlockerLedger,
    FrontierCounts, OutputEvidence, OutputState, Release, DEFAULT_ESCALATION_AFTER,
};

fn hour(n: u64) -> Duration {
    Duration::from_secs(n * 3_600)
}

// --- Invariant 2: stored output has a lifecycle ---------------------------

#[test]
fn an_empty_output_directory_is_no_state_the_guard_arms_for() {
    let evidence = OutputEvidence::default();
    assert_eq!(classify(&evidence), None);
    assert_eq!(release(None), Release::None);
    assert_eq!(held_line("229", &evidence, Some(hour(48))), None);
}

#[test]
fn a_live_unconverted_patch_holds_and_names_conversion_as_the_release() {
    let evidence = OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Applies),
    };
    assert_eq!(classify(&evidence), Some(OutputState::AwaitingConversion));
    assert_eq!(
        release(Some(OutputState::AwaitingConversion)),
        Release::ConvertPatch
    );
}

#[test]
fn a_patch_git_apply_rejects_is_superseded_and_names_archival() {
    // The #229 case: the output was written 2026-09-08, and by now every
    // hunk is already on main, so `git apply --check` rejects the patch.
    let evidence = OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Rejected),
    };
    assert_eq!(classify(&evidence), Some(OutputState::Superseded));
    assert_eq!(
        release(Some(OutputState::Superseded)),
        Release::ArchiveSuperseded
    );
}

#[test]
fn an_apply_check_that_cannot_run_is_fail_closed_not_superseded() {
    let unrunnable = OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Unrunnable {
            detail: "git apply: not a git repository".to_string(),
        }),
    };
    assert_eq!(classify(&unrunnable), Some(OutputState::AwaitingConversion));
    let never_run = OutputEvidence::new(true, false, false);
    assert_eq!(classify(&never_run), Some(OutputState::AwaitingConversion));
}

#[test]
fn a_converted_output_clears_the_guard_even_if_the_patch_file_remains() {
    let evidence = OutputEvidence {
        patch_present: true,
        converted: true,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Rejected),
    };
    assert_eq!(classify(&evidence), Some(OutputState::Converted));
    assert_eq!(release(Some(OutputState::Converted)), Release::None);
}

#[test]
fn a_failed_conversion_with_no_patch_left_is_a_review_not_an_archive() {
    let evidence = OutputEvidence::new(false, false, true);
    assert_eq!(classify(&evidence), Some(OutputState::Failed));
    assert_eq!(release(Some(OutputState::Failed)), Release::ReviewFailure);
}

#[test]
fn a_failed_conversion_with_a_live_patch_stays_live() {
    let evidence = OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: true,
        apply_check: Some(ApplyCheck::Applies),
    };
    assert_eq!(classify(&evidence), Some(OutputState::AwaitingConversion));
}

#[test]
fn the_lifecycle_states_round_trip_through_json() {
    let evidence = OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Rejected),
    };
    let raw = serde_json::to_string(&evidence).unwrap();
    let back: OutputEvidence = serde_json::from_str(&raw).unwrap();
    assert_eq!(evidence, back);
    let blocked = BlockedIssue {
        issue: "229".to_string(),
        reason: "superseded output".to_string(),
        release: Release::ArchiveSuperseded,
        age: Some(hour(48)),
    };
    let raw = serde_json::to_string(&blocked).unwrap();
    let back: BlockedIssue = serde_json::from_str(&raw).unwrap();
    assert_eq!(blocked, back);
}

// --- Invariant 1: a guard names its release action ------------------------

#[test]
fn the_hold_line_always_names_a_release_action() {
    let live = OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Applies),
    };
    let line = held_line("229", &live, None).unwrap();
    assert!(line.contains("convert the stored patch to a PR"), "{line}");

    let superseded = OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Rejected),
    };
    let line = held_line("229", &superseded, None).unwrap();
    assert!(line.contains("archive the superseded output"), "{line}");

    let failed = OutputEvidence::new(false, false, true);
    let line = held_line("229", &failed, None).unwrap();
    assert!(line.contains("review the failed conversion"), "{line}");
}

#[test]
fn a_hold_with_no_named_release_is_rendered_as_a_deadlock() {
    let blocked = BlockedIssue {
        issue: "229".to_string(),
        reason: "stored output present".to_string(),
        release: Release::None,
        age: None,
    };
    let line = blocked.line();
    assert!(
        line.contains("no release path (deadlock)"),
        "the line must name the defect, not pretend: {line}"
    );
}

// --- Invariant 3: a persistent blocker escalates --------------------------

#[test]
fn the_ledger_keeps_the_age_across_runs_and_restarts_it_on_resolve() {
    let mut ledger = BlockerLedger::new();
    // Run one: first observation stamps the age.
    ledger.observe("229:stored-output", Duration::ZERO);
    // Run two, three, four: later observations must not reset the age.
    ledger.observe("229:stored-output", hour(12));
    ledger.observe("229:stored-output", hour(36));
    assert_eq!(ledger.age("229:stored-output", hour(48)), Some(hour(48)));
    assert_eq!(
        ledger.escalation_phrase("229:stored-output", hour(48), DEFAULT_ESCALATION_AFTER),
        Some("blocked for 2d".to_string())
    );
    // The blocker clears (the output was archived): the age starts over.
    ledger.resolve("229:stored-output");
    assert!(ledger.is_empty());
    ledger.observe("229:stored-output", hour(100));
    assert_eq!(ledger.age("229:stored-output", hour(101)), Some(hour(1)));
}

#[test]
fn a_young_blocker_is_not_yet_an_escalation() {
    let mut ledger = BlockerLedger::new();
    ledger.observe("31:stored-output", hour(10));
    assert_eq!(
        ledger.escalation_phrase("31:stored-output", hour(11), DEFAULT_ESCALATION_AFTER),
        None
    );
    assert_eq!(
        ledger.escalation_phrase("31:stored-output", hour(100), DEFAULT_ESCALATION_AFTER),
        Some(format!("blocked for {}", format_age(hour(90))))
    );
}

#[test]
fn the_ledger_survives_a_run_boundary_through_json() {
    let mut ledger = BlockerLedger::new();
    ledger.observe("229:stored-output", Duration::ZERO);
    let raw = ledger.to_json();
    let back = BlockerLedger::from_json(&raw).expect("ledger round-trips");
    assert_eq!(back.age("229:stored-output", hour(48)), Some(hour(48)));
    assert!(
        BlockerLedger::from_json("not json").is_err(),
        "a corrupt ledger is an error, never an empty one"
    );
}

// --- Invariant 4: ready and dispatchable are separate counts --------------

#[test]
fn the_summary_reports_ready_dispatchable_and_blocked_separately() {
    let counts = FrontierCounts {
        open: 62,
        ready: 2,
        dispatchable: 0,
        blocked: vec![
            BlockedIssue {
                issue: "229".to_string(),
                reason: "superseded output (patch no longer applies to main)".to_string(),
                release: Release::ArchiveSuperseded,
                age: Some(hour(48)),
            },
            BlockedIssue {
                issue: "233".to_string(),
                reason: "superseded output (patch no longer applies to main)".to_string(),
                release: Release::ArchiveSuperseded,
                age: Some(hour(48)),
            },
        ],
        staged: 0,
        dispatched: 0,
    };
    let line = counts.line();
    // The old line "2 ready (0 already running)" collapsed these; the new
    // line keeps the counts apart and names the third number.
    assert!(line.contains("2 ready (0 dispatchable)"), "{line}");
    assert!(line.contains("2 blocked"), "{line}");
    assert!(line.contains("#229"), "{line}");
    assert!(line.contains("#233"), "{line}");
    assert!(counts.is_consistent(), "{line}");
}

#[test]
fn an_idle_frontier_and_a_blocked_one_render_differently() {
    let idle = FrontierCounts {
        open: 62,
        ready: 2,
        dispatchable: 2,
        blocked: vec![],
        staged: 0,
        dispatched: 0,
    };
    assert!(
        !idle.line().contains("blocked"),
        "an idle frontier must not report blockers: {}",
        idle.line()
    );
    let blocked = FrontierCounts {
        ready: 2,
        dispatchable: 1,
        blocked: vec![BlockedIssue {
            issue: "229".to_string(),
            reason: "superseded output (patch no longer applies to main)".to_string(),
            release: Release::ArchiveSuperseded,
            age: None,
        }],
        ..idle
    };
    let line = blocked.line();
    assert!(line.contains("2 ready (1 dispatchable)"), "{line}");
    assert!(line.contains("1 blocked"), "{line}");
    // First run that saw the blocker: no age is claimed.
    assert!(!line.contains("blocked for"), "{line}");
}

#[test]
fn the_counts_must_reconcile_or_the_frontier_is_inconsistent() {
    let counts = FrontierCounts {
        open: 10,
        ready: 5,
        dispatchable: 1,
        blocked: vec![
            BlockedIssue {
                issue: "1".to_string(),
                reason: "r".to_string(),
                release: Release::ConvertPatch,
                age: None,
            },
            BlockedIssue {
                issue: "2".to_string(),
                reason: "r".to_string(),
                release: Release::ConvertPatch,
                age: None,
            },
        ],
        staged: 0,
        dispatched: 0,
    };
    // 5 ready = 1 dispatchable + 4 blocked would reconcile; 3 does not.
    assert!(!counts.is_consistent());
}

// --- The whole story in one line ------------------------------------------

#[test]
fn the_two_day_blocker_escapes_rendering_as_idle() {
    let mut ledger = BlockerLedger::new();
    ledger.observe("229:stored-output", Duration::ZERO);
    let now = hour(48);
    let evidence = OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Rejected),
    };
    let line = held_line("229", &evidence, ledger.age("229:stored-output", now)).unwrap();
    assert!(line.contains("superseded"), "{line}");
    assert!(line.contains("archive the superseded output"), "{line}");
    assert!(line.contains("blocked for 2d"), "{line}");
}
