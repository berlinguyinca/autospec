//! A verdict has a shelf life (issue #3673).
//!
//! The regression tests run in the configuration the incident required:
//! one conversion pass of 158 minutes over 89 patches while 49 commits
//! landed on `main`, reporting a single bare line —
//! `converted=81 held=8 skipped=0` — as though it described one state of
//! the world. Eight of the holds were decided against a trunk that has
//! since gained 49 commits, and the line said nothing of the kind.

use autospec_core::verdict_shelf::{
    due_rechecks, pass_span, recheck_line, BucketShelf, PassSpan, PassSummary, Verdict, VerdictKind,
};

/// The trunk tip at the start of the incident pass.
const OLD_BASE: &str = "a1b2c3d4";
/// The trunk tip at the end of the incident pass: 49 commits later.
const TIP: &str = "e5f6a7b8";
/// The incident pass's wall-clock length: 158 minutes.
const PASS_SECS: u64 = 158 * 60;
/// Commits to `main` during the incident pass.
const COMMITS: usize = 49;

/// Build the incident pass: 89 patches, 80 verdicts issued before the
/// trunk moved (8 of them `Held`, the rest converted), 9 after.
fn incident_pass() -> PassSummary {
    let mut verdicts = Vec::new();
    // The 80 verdicts decided against the old trunk: 72 converted, 8 held.
    // Three of the holds are cheap decisions — the "issue closed" check —
    // the rest are expensive build holds.
    for i in 0u64..72 {
        verdicts.push(Verdict::new(1000 + i, VerdictKind::Converted, OLD_BASE, false).unwrap());
    }
    for (j, issue) in [4001u64, 4002, 4003, 4004, 4005, 4006, 4007, 4008]
        .into_iter()
        .enumerate()
    {
        verdicts.push(Verdict::new(issue, VerdictKind::Held, OLD_BASE, j < 3).unwrap());
    }
    // The 9 verdicts decided after the trunk moved: converted.
    for i in 0u64..9 {
        verdicts.push(Verdict::new(2000 + i, VerdictKind::Converted, TIP, false).unwrap());
    }
    PassSummary {
        verdicts,
        start_sha: OLD_BASE.to_string(),
        end_sha: TIP.to_string(),
    }
}

// --- Invariant 1: every verdict records its base sha ----------------------

#[test]
fn a_verdict_without_a_base_sha_is_refused() {
    // A summary without an expiry is the defect: a verdict that cannot
    // say which trunk it was decided against is refused, never defaulted.
    assert_eq!(Verdict::new(1, VerdictKind::Held, "", false), None);
    assert_eq!(Verdict::new(1, VerdictKind::Held, "   ", false), None);
    assert_eq!(
        Verdict::new(1, VerdictKind::Held, TIP, false)
            .unwrap()
            .base_sha,
        TIP
    );
}

#[test]
fn incident_summary_line_carries_the_expiry() {
    let pass = incident_pass();
    assert_eq!(
        pass.line(TIP),
        "converted=81 (9 current, 72 against an older base) \
         held=8 (0 current, 8 against an older base, will be retested) \
         skipped=0"
    );
}

#[test]
fn the_bare_counter_line_is_not_what_the_pass_prints() {
    // The incident's line: one line as though it described one state of
    // the world. The fixed line names the eight holds that are
    // hypotheses, not decisions.
    let pass = incident_pass();
    let line = pass.line(TIP);
    assert_ne!(line, "converted=81 held=8 skipped=0");
    assert!(line.contains("8 against an older base, will be retested"));
}

#[test]
fn the_issue_example_held_bucket() {
    // The issue's own example: "held=15 (7 current, 8 against an older
    // base, will be retested)".
    let mut verdicts = Vec::new();
    for i in 0u64..7 {
        verdicts.push(Verdict::new(10 + i, VerdictKind::Held, TIP, false).unwrap());
    }
    for i in 0u64..8 {
        verdicts.push(Verdict::new(20 + i, VerdictKind::Held, OLD_BASE, false).unwrap());
    }
    let pass = PassSummary {
        verdicts,
        start_sha: OLD_BASE.to_string(),
        end_sha: TIP.to_string(),
    };
    assert_eq!(
        pass.line(TIP),
        "converted=0 held=15 (7 current, 8 against an older base, will be retested) skipped=0"
    );
}

#[test]
fn a_current_bucket_stays_bare() {
    // "All current" is the state the bare line honestly describes: no
    // annotation when no verdict is stale.
    let pass = PassSummary {
        verdicts: vec![
            Verdict::new(1, VerdictKind::Converted, TIP, false).unwrap(),
            Verdict::new(2, VerdictKind::Converted, TIP, false).unwrap(),
            Verdict::new(3, VerdictKind::Held, TIP, false).unwrap(),
        ],
        start_sha: TIP.to_string(),
        end_sha: TIP.to_string(),
    };
    assert_eq!(pass.line(TIP), "converted=2 held=1 skipped=0");
}

#[test]
fn the_retest_clause_is_on_held_only() {
    // A converted verdict is terminal and a skipped one is merely
    // re-evaluated: neither is retested the way a hold is, so the
    // annotation differs.
    let pass = PassSummary {
        verdicts: vec![
            Verdict::new(1, VerdictKind::Converted, OLD_BASE, false).unwrap(),
            Verdict::new(2, VerdictKind::Skipped, OLD_BASE, true).unwrap(),
        ],
        start_sha: OLD_BASE.to_string(),
        end_sha: TIP.to_string(),
    };
    assert_eq!(
        pass.line(TIP),
        "converted=1 (0 current, 1 against an older base) \
         held=0 skipped=1 (0 current, 1 against an older base)"
    );
}

#[test]
fn every_bucket_reconciles() {
    let pass = incident_pass();
    for kind in [
        VerdictKind::Converted,
        VerdictKind::Held,
        VerdictKind::Skipped,
    ] {
        let shelf: BucketShelf = pass.shelf(kind, TIP);
        assert!(
            shelf.reconciles(),
            "bucket {kind:?} does not reconcile: {shelf:?}"
        );
    }
    assert_eq!(
        pass.shelf(VerdictKind::Held, TIP),
        BucketShelf {
            total: 8,
            current: 0,
            stale: 8,
        }
    );
}

// --- Invariant 2: a pass that outlives the trunk says so ------------------

#[test]
fn the_incident_pass_span_is_trunk_moved() {
    let span = pass_span(OLD_BASE, TIP, COMMITS, PASS_SECS, None);
    assert_eq!(span, PassSpan::TrunkMoved { commits: COMMITS });
    assert_eq!(
        span.warn_line(PASS_SECS).as_deref(),
        Some(
            "WARN: pass ran 9480s across 49 trunk change(s); the summary spans more than one trunk"
        )
    );
}

#[test]
fn the_pass_struct_span_agrees() {
    let pass = incident_pass();
    assert_eq!(
        pass.span(COMMITS, PASS_SECS, None),
        PassSpan::TrunkMoved { commits: COMMITS }
    );
}

#[test]
fn an_unmeasured_tip_change_still_says_so() {
    // The tips differ but the advance count was not measured: the span
    // still names the trunk movement, without inventing a count.
    let span = pass_span(OLD_BASE, TIP, 0, PASS_SECS, None);
    assert_eq!(span, PassSpan::TrunkMoved { commits: 0 });
    assert_eq!(
        span.warn_line(PASS_SECS).as_deref(),
        Some(
            "WARN: the trunk moved during the pass (9480s); the summary spans more than one trunk"
        )
    );
}

#[test]
fn a_still_trunk_pass_that_outlived_the_interval_says_so() {
    // The trunk did not move, but the pass is at least as long as the
    // trunk's change interval: the moving target was waiting.
    let interval = 1200u64; // a merge every 20 minutes
    let span = pass_span(TIP, TIP, 0, PASS_SECS, Some(interval));
    assert_eq!(
        span,
        PassSpan::OutlivedInterval {
            duration_secs: PASS_SECS,
            interval_secs: interval
        }
    );
    assert_eq!(
        span.warn_line(PASS_SECS).as_deref(),
        Some("WARN: pass duration 9480s meets the trunk change interval 1200s; the summary is an average over a moving target")
    );
}

#[test]
fn the_trunk_moving_is_the_stronger_fact() {
    // A pass that ran across a tip change is TrunkMoved even when it is
    // also longer than the change interval.
    let span = pass_span(OLD_BASE, TIP, COMMITS, PASS_SECS, Some(1200));
    assert_eq!(span, PassSpan::TrunkMoved { commits: COMMITS });
}

#[test]
fn a_short_pass_below_the_interval_says_nothing() {
    // One state of the world: no qualification, no warning.
    let span = pass_span(TIP, TIP, 0, 600, Some(1200));
    assert_eq!(span, PassSpan::SingleTrunk);
    assert_eq!(span.warn_line(600), None);
}

#[test]
fn an_unmeasured_interval_never_trips_the_span() {
    // A zero or absent interval is not a measurement.
    assert_eq!(
        pass_span(TIP, TIP, 0, PASS_SECS, None),
        PassSpan::SingleTrunk
    );
    assert_eq!(
        pass_span(TIP, TIP, 0, PASS_SECS, Some(0)),
        PassSpan::SingleTrunk
    );
}

// --- Invariant 3: cheap decisions are re-checked at the end ---------------

#[test]
fn the_stale_cheap_decisions_are_due_for_recheck() {
    let pass = incident_pass();
    // The three "issue closed" holds decided against the old base are
    // seconds to recompute and the most likely to have changed.
    assert_eq!(due_rechecks(&pass.verdicts, TIP), vec![4001, 4002, 4003]);
}

#[test]
fn expensive_stale_verdicts_are_not_rechecked_mid_pass() {
    // The five expensive build holds are retested on the next pass via
    // the (patch hash, base sha) memo key, not re-run here: re-running
    // them is what made the pass 158 minutes.
    let pass = incident_pass();
    let due = due_rechecks(&pass.verdicts, TIP);
    assert!(!due.contains(&4004));
    assert!(!due.contains(&4008));
    assert!(!due.iter().any(|i| *i < 1000));
}

#[test]
fn a_current_cheap_verdict_is_not_due() {
    // Cheap but current: the decision stands, nothing to re-check.
    let verdicts = vec![
        Verdict::new(1, VerdictKind::Skipped, TIP, true).unwrap(),
        Verdict::new(2, VerdictKind::Skipped, OLD_BASE, true).unwrap(),
    ];
    assert_eq!(due_rechecks(&verdicts, TIP), vec![2]);
}

#[test]
fn the_recheck_line_names_what_it_rechecked() {
    assert_eq!(recheck_line(&[]), "recheck at pass end: none due");
    assert_eq!(
        recheck_line(&[4001, 4002, 4003]),
        "recheck at pass end: 3 cheap decision(s) against an older base (#4001 #4002 #4003)"
    );
}
