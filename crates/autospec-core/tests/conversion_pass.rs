//! Tests for [`autospec_core::conversion_pass`] (issue #4388).
//!
//! The regression tests run in the configuration the incidents required:
//!
//! - **The "121 vs 11" selection.** The shell pass counted patch files on
//!   disk and reported 121 "patches awaiting conversion" when a branch, a
//!   PR, or a HELD entry already owned 110 of them and only 11 were
//!   actually fresh. [`select_fresh`] must select on the three
//!   disqualifiers, never on the count.
//! - **The unfed-vs-idle line.** The shell pass, run bare, printed
//!   `converted=0 held=0 skipped=0` — byte-identical to a healthy idle pass
//!   — while four patches waited. [`PassOutcome`] keeps the two lines apart.
//! - **HELD is a queue, not a log.** A HELD entry disqualifies only while its
//!   re-gate still holds: when the base moves under it (or the patch changes)
//!   the pass re-offers it, so a stale hold never parks an issue forever.

use autospec_core::conversion_pass::{
    disqualification, select_fresh, Candidate, Disqualification, PassOutcome, PatchCandidate,
};
use autospec_core::hold_memo::{re_gate, HoldRecord, ReGateDecision};
use autospec_core::unfed_pass::PassCounters;

/// Build a candidate whose three disqualifiers are all unset (fresh).
fn fresh(issue: u64) -> PatchCandidate {
    PatchCandidate {
        issue,
        patch_key: format!("patch-{issue}"),
        ..Default::default()
    }
}

/// The "11 not 121" incident: 121 patches on disk, 110 already owned by a
/// branch, a PR, or a HELD entry, 11 actually fresh.
fn incident_candidates() -> Vec<PatchCandidate> {
    (1..=121)
        .map(|n| {
            let mut c = fresh(n);
            match n {
                1..=70 => c.branch_exists = true,
                71..=95 => c.pull_request_exists = true,
                96..=110 => c.held_recorded = true,
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

    // 11 actually fresh, not 121 "awaiting conversion".
    assert_eq!(selection.fresh_count(), 11);
    assert_eq!(selection.disqualified.len(), 110);

    let counts = selection.disqualified_counts();
    assert_eq!(
        counts,
        [
            (Disqualification::Branch, 70),
            (Disqualification::PullRequest, 25),
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
    // A zero fresh count is only readable against the input size.
    assert!(line.contains("examined=121"), "{line}");
    assert!(line.contains("fresh=11"), "{line}");
    assert!(line.contains("70 branch"), "{line}");
    assert!(line.contains("25 pull-request"), "{line}");
    assert!(line.contains("15 held"), "{line}");
}

#[test]
fn an_idle_plan_is_distinct_from_a_broken_one() {
    // The pass enumerated 121 patches and found 11 fresh: a real pass,
    // reported as examined.
    let examined = PassOutcome::Examined(PassCounters {
        examined: 121,
        converted: 0,
        held: 0,
        skipped: 110,
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
    let candidate = PatchCandidate {
        held_recorded: still_held.is_still_held(),
        ..fresh(4015)
    };
    assert!(candidate.held_recorded);
    assert_eq!(
        disqualification(false, false, true),
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
    let reoffered = PatchCandidate {
        held_recorded: re_gated.is_still_held(),
        ..fresh(4015)
    };
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
        held_recorded: decision.is_still_held(),
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
        with(fresh(1), |c| c.branch_exists = true),
        fresh(2),
        with(fresh(4), |c| c.held_recorded = true),
        fresh(5),
    ];
    let selection = select_fresh(&candidates);
    let offered: Vec<u64> = selection.fresh.iter().map(|c| c.issue).collect();
    assert_eq!(offered, vec![3, 2, 5]);
}

fn with(mut candidate: PatchCandidate, change: impl FnOnce(&mut PatchCandidate)) -> PatchCandidate {
    change(&mut candidate);
    candidate
}
