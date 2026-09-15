//! Tests for [`autospec_core::conversion_pass`] (issues #4388, #4499).
//!
//! The regression tests run in the configuration the incidents required:
//!
//! - **The "121 vs 11" selection.** The shell pass counted patch files on
//!   disk and reported 121 "patches awaiting conversion" when a live attempt
//!   or a HELD entry already owned 110 of them and only 11 were actually
//!   fresh. [`select_fresh`] must select on the attempt state and the HELD
//!   re-gate, never on the count.
//! - **The push-before-PR window (#4499).** The pass pushes a branch and
//!   only then opens the PR; an interruption between them leaves a branch
//!   with no PR. A bare branch is not evidence of an attempt: the issue is
//!   re-offered (redoing overwrites the orphan branch) and reported as a
//!   distinct `interrupted` category, never silently retired.
//! - **The unfed-vs-idle line.** The shell pass, run bare, printed
//!   `converted=0 held=0 skipped=0` — byte-identical to a healthy idle pass
//!   — while four patches waited. [`PassOutcome`] keeps the two lines apart.
//! - **HELD is a queue, not a log.** A HELD entry disqualifies only while its
//!   re-gate still holds: when the base moves under it (or the patch changes)
//!   the pass re-offers it, so a stale hold never parks an issue forever.

use autospec_core::conversion_pass::{
    disqualification, select_fresh, Attempt, Candidate, Disqualification, PassOutcome,
    PatchCandidate,
};
use autospec_core::hold_memo::{re_gate, HoldRecord, ReGateDecision};
use autospec_core::unfed_pass::PassCounters;

/// Build a candidate with a fresh attempt and no held record.
fn fresh(issue: u64) -> PatchCandidate {
    PatchCandidate {
        issue,
        patch_key: format!("patch-{issue}"),
        ..Default::default()
    }
}

fn with(mut candidate: PatchCandidate, change: impl FnOnce(&mut PatchCandidate)) -> PatchCandidate {
    change(&mut candidate);
    candidate
}

/// The "11 not 121" incident, restated on the attempt axis: 121 patches on
/// disk, 70 owned by a live attempt, 15 by a HELD entry, 26 interrupted
/// (a branch, no PR — the push-before-PR window), 10 actually fresh.
fn incident_candidates() -> Vec<PatchCandidate> {
    (1..=121)
        .map(|n| {
            let mut c = fresh(n);
            match n {
                1..=70 => c.attempt = Attempt::Live,
                71..=85 => c.held_recorded = true,
                86..=111 => c.attempt = Attempt::Interrupted,
                _ => {}
            }
            c
        })
        .collect()
}

#[test]
fn the_pass_selects_the_fresh_not_the_count() {
    let candidates = incident_candidates();
    let selection = select_fresh(&candidates);

    // 36 offered (10 fresh + 26 interrupted), 85 disqualified — not 121
    // "awaiting conversion", and not 10 pretending the 26 orphans do not
    // exist.
    assert_eq!(selection.fresh_count(), 36);
    assert_eq!(selection.disqualified.len(), 85);

    let counts = selection.disqualified_counts();
    assert_eq!(
        counts,
        [
            (Disqualification::Attempted, 70),
            (Disqualification::Held, 15),
        ]
    );

    // The selection reconciles against the input: every patch is either
    // offered or disqualified, never both, never neither.
    assert_eq!(
        selection.fresh_count() + selection.disqualified.len(),
        candidates.len()
    );
}

#[test]
fn the_plan_line_reports_examined_fresh_and_each_reason() {
    let selection = select_fresh(&incident_candidates());
    let line = selection.line(121);
    // A zero fresh count is only readable against the input size, and the
    // interrupted count is named on its own: the orphans are visible.
    assert!(line.contains("examined=121"), "{line}");
    assert!(line.contains("fresh=36"), "{line}");
    assert!(line.contains("interrupted=26"), "{line}");
    assert!(line.contains("delivered=0"), "{line}");
    assert!(line.contains("70 attempted"), "{line}");
    assert!(line.contains("15 held"), "{line}");
}

#[test]
fn a_delivered_patch_is_reported_not_offered() {
    // #4501: the changes are in the base — the patch is residue. It must
    // not be offered (the gate would spend two hours on nothing), and it
    // must be visible on its own: the backlog number has to mean what a
    // reader assumes it means.
    let candidates = vec![fresh(1), with(fresh(2), |c| c.delivered = true)];
    let selection = select_fresh(&candidates);
    assert_eq!(
        selection.fresh.len(),
        1,
        "only the undelivered patch is offered"
    );
    assert_eq!(
        selection.fresh.first().map(|c| c.patch_key.as_str()),
        Some("patch-1")
    );
    assert_eq!(
        selection.delivered.first().map(|c| c.patch_key.as_str()),
        Some("patch-2"),
        "the residue is named as delivered, not hidden"
    );
    assert_eq!(selection.delivered_count(), 1);
    let line = selection.line(2);
    assert!(line.contains("delivered=1"), "{line}");
    assert!(line.contains("fresh=1"), "{line}");
}

#[test]
fn a_delivered_patch_beats_a_live_attempt_and_a_hold() {
    // The changes are in the base: the attempt state and any recorded hold
    // are both stale facts about a patch that has nothing left to convert.
    // Delivered must win so the pass reports it, not re-offers or skips it.
    let live = with(fresh(1), |c| c.attempt = Attempt::Live);
    let held = with(fresh(2), |c| c.held_recorded = true);
    let candidates = vec![
        with(live, |c| c.delivered = true),
        with(held, |c| c.delivered = true),
    ];
    let selection = select_fresh(&candidates);
    assert_eq!(selection.fresh.len(), 0, "delivered is never offered");
    assert_eq!(
        selection.disqualified.len(),
        0,
        "delivered is not a disqualification; it is its own category"
    );
    assert_eq!(
        selection.delivered.len(),
        2,
        "both are reported as delivered"
    );
}

#[test]
fn the_line_counts_delivered_alongside_fresh_and_interrupted() {
    let selection = select_fresh(&[
        with(fresh(1), |c| c.attempt = Attempt::Interrupted),
        with(fresh(2), |c| c.delivered = true),
        with(fresh(3), |c| c.delivered = true),
        with(fresh(4), |c| c.attempt = Attempt::Live),
        fresh(5),
    ]);
    let line = selection.line(5);
    // fresh counts the offered: the new one and the re-offered
    // interrupted one (issue #4499); delivered and live are not offered.
    assert!(line.contains("fresh=2"), "{line}");
    assert!(line.contains("interrupted=1"), "{line}");
    assert!(line.contains("delivered=2"), "{line}");
    assert!(line.contains("1 attempted"), "{line}");
}

#[test]
fn an_idle_plan_is_distinct_from_a_broken_one() {
    // The pass enumerated 121 patches and found 37 to attempt: a real pass,
    // reported as examined.
    let examined = PassOutcome::Examined(PassCounters {
        examined: 121,
        converted: 0,
        held: 0,
        skipped: 85,
        deferred: 0,
        delivered: 0,

        invalidated: 0,
        closed: 0,
    });
    // The pass was handed no candidates: it did no work, and its line must
    // not look like the idle one.
    let unfed = PassOutcome::Unfed;
    let (tool, script, selector) = ("convert", "autospec convert", "enumerate $LLM");

    assert_ne!(
        unfed.line(tool, script, selector),
        examined.line(tool, script, selector),
        "the incident is the two lines being identical"
    );
    assert!(PassOutcome::unfed_and_idle_differ(tool, script, selector));
    // The idle line leads with the input size; the unfed line names the
    // empty-input branch and the selector that would feed it.
    assert!(examined
        .line(tool, script, selector)
        .contains("examined=121"));
    let unfed_line = unfed.line(tool, script, selector);
    assert!(unfed_line.contains("no issues given"), "{unfed_line}");
    assert!(unfed_line.contains(selector), "{unfed_line}");
}

#[test]
fn a_held_entry_disqualifies_only_while_its_regate_still_holds() {
    // A HELD entry recorded against base abc123, depending on two files.
    let record = HoldRecord::new(
        4015,
        "patch-4015",
        "abc123",
        vec![
            "crates/autospec-cli/src/commands/queue_commands.rs".to_string(),
            "crates/autospec-cli/src/commands/ready_queue.rs".to_string(),
        ],
        "conflict in queue_commands.rs and ready_queue.rs",
    )
    .unwrap();

    // The patch is unchanged and none of its dependent files moved on the
    // base: the hold stands, so it disqualifies.
    let still_held = re_gate(&record, "patch-4015", &[]);
    assert!(
        matches!(still_held, ReGateDecision::StillHeld { .. }),
        "{still_held:?}"
    );
    let candidate = with(fresh(4015), |c| c.held_recorded = true);
    assert!(candidate.held_recorded);
    assert_eq!(
        disqualification(Attempt::Fresh, true),
        Some(Disqualification::Held)
    );
    assert!(select_fresh(&[candidate]).fresh.is_empty());

    // Main moved under the hold: a dependent file changed on the base since
    // abc123. The re-gate fires, the pass re-offers the patch, and it is no
    // longer disqualified by the hold — the queue, not the log.
    let changed = vec![
        "crates/autospec-cli/src/commands/queue_commands.rs".to_string(),
        "some-unrelated-file.rs".to_string(),
    ];
    let re_gated = re_gate(&record, "patch-4015", &changed);
    assert!(
        matches!(
            re_gated,
            ReGateDecision::ReGate {
                patch_changed: false,
                ..
            }
        ),
        "{re_gated:?}"
    );
    let reoffered = fresh(4015);
    assert!(!reoffered.held_recorded, "a re-gated hold is re-offered");
    let selection = select_fresh(&[reoffered]);
    assert_eq!(
        selection.fresh,
        vec![Candidate {
            issue: 4015,
            patch_key: "patch-4015".to_string(),
        }]
    );
}

#[test]
fn a_changed_patch_is_reoffered_even_if_the_base_is_unchanged() {
    let record = HoldRecord::new(
        4068,
        "old-key",
        "abc123",
        vec!["a.rs".to_string()],
        "conflict in a.rs",
    )
    .unwrap();
    // The redispatched agent produced fresh work: the patch key changed, so
    // the re-gate fires on the patch alone (no base change needed).
    let decision = re_gate(&record, "new-key", &[]);
    assert!(
        matches!(
            decision,
            ReGateDecision::ReGate {
                patch_changed: true,
                ..
            }
        ),
        "{decision:?}"
    );
    let candidate = PatchCandidate {
        patch_key: "new-key".to_string(),
        ..fresh(4068)
    };
    assert!(!candidate.held_recorded);
    assert_eq!(select_fresh(&[candidate]).fresh_count(), 1);
}

#[test]
fn the_selection_is_pure_and_order_preserving() {
    // The offered set keeps the input order: the pass attempts patches in
    // the order enumerated, not in a re-sorted order.
    let candidates = vec![
        fresh(3),
        with(fresh(1), |c| c.attempt = Attempt::Live),
        fresh(2),
        with(fresh(4), |c| c.held_recorded = true),
        with(fresh(5), |c| c.attempt = Attempt::Interrupted),
    ];
    let selection = select_fresh(&candidates);
    let offered: Vec<u64> = selection.fresh.iter().map(|c| c.issue).collect();
    assert_eq!(offered, vec![3, 2, 5]);
}

// --- the push-before-PR window (#4499) ------------------------------------

#[test]
fn a_bare_branch_never_disqualifies_the_candidate() {
    // The incident: a branch pushed, no PR opened (the run died in the
    // window). The old axis read the bare branch as "attempted" and retired
    // the issue forever. On the attempt axis the same fact is
    // `Interrupted`, and the patch is offered again.
    let interrupted = with(fresh(2995), |c| c.attempt = Attempt::Interrupted);
    let selection = select_fresh(&[interrupted]);
    assert_eq!(
        selection.fresh,
        vec![Candidate {
            issue: 2995,
            patch_key: "patch-2995".to_string(),
        }],
        "the interrupted attempt is re-offered, not retired"
    );
    assert!(selection.disqualified.is_empty());
}

#[test]
fn an_interrupted_attempt_is_reported_as_a_distinct_category() {
    // The re-offered orphans are named, so an interrupted run leaves a
    // record of what it left behind instead of disappearing into `fresh`.
    let candidates = vec![
        fresh(1),
        with(fresh(2995), |c| c.attempt = Attempt::Interrupted),
        with(fresh(3189), |c| c.attempt = Attempt::Interrupted),
        with(fresh(2692), |c| c.held_recorded = true),
    ];
    let selection = select_fresh(&candidates);
    let interrupted: Vec<u64> = selection.interrupted.iter().map(|c| c.issue).collect();
    assert_eq!(interrupted, vec![2995, 3189]);
    // They are offered AND named: both buckets carry them.
    assert_eq!(selection.fresh_count(), 3);
    // And the line says so.
    let line = selection.line(4);
    assert!(line.contains("interrupted=2"), "{line}");
}

#[test]
fn a_live_attempt_disqualifies_and_an_unknown_one_fails_closed() {
    // The disqualifying fact is a live attempt: a branch with an open or
    // merged PR (or a checked-out worktree).
    assert_eq!(
        disqualification(Attempt::Live, false),
        Some(Disqualification::Attempted)
    );
    // Unknown is the lookup failing, not a verdict: offering the patch
    // risks a duplicate PR, so it folds fail-closed like live.
    assert_eq!(
        disqualification(Attempt::Unknown, false),
        Some(Disqualification::Attempted)
    );
    // Fresh and interrupted do not disqualify; a held record does, on its
    // own, for either.
    assert_eq!(disqualification(Attempt::Fresh, false), None);
    assert_eq!(disqualification(Attempt::Interrupted, false), None);
    assert_eq!(
        disqualification(Attempt::Fresh, true),
        Some(Disqualification::Held)
    );
    assert_eq!(
        disqualification(Attempt::Interrupted, true),
        Some(Disqualification::Held)
    );
}

#[test]
fn the_attempt_and_disqualifier_round_trip_their_wire_forms() {
    for (value, wire) in [
        (Attempt::Fresh, "fresh"),
        (Attempt::Live, "live"),
        (Attempt::Interrupted, "interrupted"),
        (Attempt::Unknown, "unknown"),
    ] {
        assert_eq!(value.as_str(), wire);
        let parsed: Attempt = serde_json::from_str(&format!("\"{wire}\"")).unwrap();
        assert_eq!(parsed, value);
    }
    for (value, wire) in [
        (Disqualification::Attempted, "attempted"),
        (Disqualification::Held, "held"),
    ] {
        assert_eq!(value.as_str(), wire);
        let parsed: Disqualification = serde_json::from_str(&format!("\"{wire}\"")).unwrap();
        assert_eq!(parsed, value);
    }
}

#[test]
fn an_unfed_pass_and_an_idle_pass_print_different_lines() {
    let tool = "convert";
    let script = "autospec convert";
    let selector = "enumerate $LLM";
    assert!(PassOutcome::unfed_and_idle_differ(tool, script, selector));

    let unfed = PassOutcome::Unfed.line(tool, script, selector);
    let idle = PassOutcome::Examined(PassCounters {
        examined: 0,
        converted: 0,
        held: 0,
        skipped: 0,
        delivered: 0,

        invalidated: 0,
        closed: 0,
        deferred: 0,
    })
    .line(tool, script, selector);
    assert_ne!(unfed, idle, "the incident is the two lines being equal");
    assert!(idle.contains("examined=0"), "{idle}");
    // The line accounts for every counter a pass can carry, including
    // the candidates it declined to start (#4607): a pass that did 1 of
    // 12 and deferred 11 prints a different line than one that did 12
    // of 12, and the shape of the line says so.
    let partial = PassOutcome::Examined(PassCounters {
        examined: 12,
        converted: 1,
        held: 0,
        skipped: 0,
        deferred: 11,
        delivered: 0,

        invalidated: 0,
        closed: 0,
    })
    .line(tool, script, selector);
    assert!(partial.contains("deferred=11"), "{partial}");
    assert_ne!(partial, idle, "a partial pass must not print the idle line");
    assert!(unfed.contains("no issues given"), "{unfed}");
    assert!(unfed.contains(selector), "{unfed}");
}

#[test]
fn an_unfed_outcome_carries_no_counters() {
    assert!(PassOutcome::Unfed.counters().is_none());
    let examined = PassOutcome::Examined(PassCounters {
        examined: 3,
        converted: 1,
        held: 1,
        skipped: 1,
        deferred: 0,
        delivered: 0,

        invalidated: 0,
        closed: 0,
    });
    assert_eq!(examined.counters().unwrap().examined, 3);
}

#[test]
fn the_outcome_reconciles_its_counters() {
    assert!(PassOutcome::Unfed.reconciles());
    let ok = PassOutcome::Examined(PassCounters {
        examined: 3,
        converted: 1,
        held: 1,
        skipped: 1,
        deferred: 0,
        delivered: 0,

        invalidated: 0,
        closed: 0,
    });
    assert!(ok.reconciles());
    let impossible = PassOutcome::Examined(PassCounters {
        examined: 3,
        converted: 3,
        held: 1,
        skipped: 0,
        deferred: 0,
        delivered: 0,

        invalidated: 0,
        closed: 0,
    });
    assert!(
        !impossible.reconciles(),
        "acting on more than examined is impossible"
    );
    // Deferral is part of the accounting: a pass that deferred more than
    // it did not examine does not reconcile either.
    let deferred_impossible = PassOutcome::Examined(PassCounters {
        examined: 2,
        converted: 0,
        held: 0,
        skipped: 0,
        deferred: 3,
        delivered: 0,

        invalidated: 0,
        closed: 0,
    });
    assert!(!deferred_impossible.reconciles());
}

// --- closed issues (#4626) ------------------------------------------------

/// The incident: a hold on a closed issue was re-gated forever because
/// nothing ever asked the tracker. The fact of closure routes the candidate
/// to the closed bucket — not the hold's, not the fresh's.
#[test]
fn a_held_candidate_on_a_closed_issue_is_closed() {
    let selection = select_fresh(&[with(fresh(305), |c| {
        c.held_recorded = true;
        c.closed = true;
    })]);
    assert_eq!(selection.closed_count(), 1);
    assert_eq!(
        selection.closed.first().map(|c| c.issue),
        Some(305),
        "the closed bucket names the issue"
    );
    assert!(
        selection.disqualified.is_empty(),
        "closure is not a disqualification: {selection:?}"
    );
    assert!(selection.fresh.is_empty());
}

/// Closure wins over a live attempt: a merged PR on a closed issue is
/// residue to archive, not work to keep alive.
#[test]
fn closure_wins_over_a_live_attempt() {
    let selection = select_fresh(&[with(fresh(305), |c| {
        c.attempt = Attempt::Live;
        c.held_recorded = true;
        c.closed = true;
    })]);
    assert_eq!(selection.closed_count(), 1);
    assert!(
        selection.fresh.is_empty() && selection.interrupted.is_empty(),
        "a closed issue is never offered: {selection:?}"
    );
}

/// Delivered still wins over closure: the work is in the base, and that is
/// the stronger fact.
#[test]
fn delivered_wins_over_closure() {
    let selection = select_fresh(&[with(fresh(305), |c| {
        c.delivered = true;
        c.held_recorded = true;
        c.closed = true;
    })]);
    assert_eq!(selection.delivered.len(), 1);
    assert_eq!(selection.closed_count(), 0);
}

/// The line counts closures alongside everything else.
#[test]
fn the_line_counts_closed_alongside_fresh_and_held() {
    let selection = select_fresh(&[
        fresh(1),
        with(fresh(2), |c| c.held_recorded = true),
        with(fresh(3), |c| {
            c.held_recorded = true;
            c.closed = true;
        }),
    ]);
    let line = selection.line(3);
    assert!(line.contains("fresh=1"), "{line}");
    assert!(line.contains("closed=1"), "{line}");
    // The hold sits in the attempt clause, as it always has.
    assert!(line.contains("1 held"), "{line}");
}
