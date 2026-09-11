//! Tests for [`autospec_core::memo_key`] (issue #4260).
//!
//! The regression case reconstructs the incident: an "already attempted"
//! filter keyed on the issue identifier excludes a redispatched agent's
//! brand-new patch for the same issue, and the denominator
//! (`attempted=236 -> candidates=0`) makes it look healthy.

use autospec_core::memo_key::{
    input_keyed_excluded, is_fresh, reconcile, select_candidates, subject_keyed_excluded,
    AttemptRecord, CompletedWork, Decision, FinishedPatch, HeldLine, PrOutcome, DEFAULT_FRESH_MIN,
    DEFAULT_RECONCILE_WINDOW,
};

fn patch(issue: u64, key: &str, mtime: u64) -> FinishedPatch {
    FinishedPatch {
        issue,
        input_key: key.into(),
        mtime,
        has_pr: false,
        issue_closed: false,
    }
}

fn work(issue: u64, key: &str, completed_at: u64) -> CompletedWork {
    CompletedWork {
        issue,
        input_key: key.into(),
        completed_at,
    }
}

/// The incident: 236 finished patches, all for issues that were "already
/// attempted" against *older* artifacts. The subject-keyed filter yields
/// candidates=0; the input-keyed filter admits every one as `stale`.
#[test]
fn incident_reconstruction_subject_keyed_filter_loses_fresh_work() {
    let now = 1_000_000u64;
    let patches: Vec<FinishedPatch> = (1..=236)
        .map(|i| patch(i, &format!("sha-{i}-new"), now - 3_600))
        .collect();
    // Attempts recorded against the previous artifacts — different keys.
    let records: Vec<AttemptRecord> = (1..=236)
        .map(|i| AttemptRecord::new(i, format!("sha-{i}-old"), now - 7_200))
        .collect();

    // The failure mode: every subject has a record, so every patch is
    // excluded.
    assert!(patches
        .iter()
        .all(|p| subject_keyed_excluded(&records, p.issue)));

    // The fix: no record matches the current input, nothing is excluded,
    // and the stale dimension makes the blast radius visible.
    let sel = select_candidates(&patches, &records, now, DEFAULT_FRESH_MIN);
    assert_eq!(sel.report.attempted, 0);
    assert_eq!(sel.report.stale, 236);
    assert_eq!(sel.report.fresh, 0);
    assert_eq!(sel.candidates.len(), 236);
    assert!(sel.report.reconciles());
    assert!(sel.report.line().contains("stale=236"));
    assert!(sel.report.line().contains("fresh=0"));
}

/// A record whose key matches the current input and whose artifact is not
/// fresh still excludes: the memo decision holds for the same artifact.
#[test]
fn matching_key_excludes_same_artifact() {
    let now = 1_000_000u64;
    let p = patch(42, "sha-42", now - 3_600);
    let records = vec![AttemptRecord::new(42, "sha-42", now - 3_600)];
    assert!(input_keyed_excluded(&records, 42, "sha-42"));

    let sel = select_candidates(std::slice::from_ref(&p), &records, now, DEFAULT_FRESH_MIN);
    assert_eq!(sel.decisions, vec![Decision::Attempted]);
    assert!(sel.candidates.is_empty());
    assert_eq!(sel.report.attempted, 1);
    assert_eq!(sel.report.stale, 0);
    assert!(sel.report.reconciles());
}

/// A record with no captured input key is legacy and never excludes:
/// failing open costs a re-attempt, failing closed costs the patch.
#[test]
fn record_without_key_never_excludes() {
    let now = 1_000_000u64;
    let p = patch(42, "sha-42", now - 3_600);
    let records = vec![AttemptRecord::legacy(42, now - 3_600)];
    assert!(!input_keyed_excluded(&records, 42, "sha-42"));

    let sel = select_candidates(std::slice::from_ref(&p), &records, now, DEFAULT_FRESH_MIN);
    assert_eq!(sel.decisions, vec![Decision::Stale]);
    assert_eq!(sel.candidates, vec![42]);
    assert!(sel.report.reconciles());
}

/// Recency overrides history: a matching attempt, but the artifact is
/// fresh, is a candidate.
#[test]
fn fresh_override_admits_attempted_fresh_patch() {
    let now = 1_000_000u64;
    let p = patch(42, "sha-42", now - 60);
    let records = vec![AttemptRecord::new(42, "sha-42", now - 3_600)];
    assert!(input_keyed_excluded(&records, 42, "sha-42"));

    let sel = select_candidates(std::slice::from_ref(&p), &records, now, DEFAULT_FRESH_MIN);
    assert_eq!(sel.decisions, vec![Decision::FreshOverride]);
    assert_eq!(sel.candidates, vec![42]);
    assert_eq!(sel.report.fresh, 1);
    assert_eq!(sel.report.attempted, 0);
    assert!(sel.report.reconciles());
}

/// Beyond the fresh window, a matching attempt excludes.
#[test]
fn stale_patch_beyond_window_not_overridden() {
    let now = 1_000_000u64;
    // One second past the 10-minute window.
    let p = patch(42, "sha-42", now - DEFAULT_FRESH_MIN - 1);
    let records = vec![AttemptRecord::new(42, "sha-42", now - 7_200)];

    let sel = select_candidates(std::slice::from_ref(&p), &records, now, DEFAULT_FRESH_MIN);
    assert_eq!(sel.decisions, vec![Decision::Attempted]);
    assert!(sel.candidates.is_empty());
}

/// `is_fresh` at the window boundary and under clock skew: an mtime in
/// the future is fresh, not an error; an mtime far before `now` is not.
#[test]
fn is_fresh_boundary_and_future() {
    let now = 1_000_000u64;
    assert!(is_fresh(now, now, DEFAULT_FRESH_MIN));
    assert!(is_fresh(now - DEFAULT_FRESH_MIN, now, DEFAULT_FRESH_MIN));
    assert!(!is_fresh(
        now - DEFAULT_FRESH_MIN - 1,
        now,
        DEFAULT_FRESH_MIN
    ));
    assert!(is_fresh(now + 3_600, now, DEFAULT_FRESH_MIN));
    // now below the window: saturates at 0, everything is fresh.
    assert!(is_fresh(1, 10, DEFAULT_FRESH_MIN));
}

/// The selector skips terminal buckets before the attempted filter: a PR
/// wins over closed, and neither is "considered".
#[test]
fn select_candidates_skips_pr_and_closed() {
    let now = 1_000_000u64;
    let mut p_pr = patch(1, "sha-1", now - 3_600);
    p_pr.has_pr = true;
    let mut p_closed = patch(2, "sha-2", now - 3_600);
    p_closed.issue_closed = true;
    // A patch that is both PR-backed and closed: the PR wins.
    let mut p_both = patch(4, "sha-4", now - 3_600);
    p_both.has_pr = true;
    p_both.issue_closed = true;
    let p_open = patch(5, "sha-5", now - 3_600);

    let sel = select_candidates(
        &[p_pr, p_closed, p_both, p_open],
        &[],
        now,
        DEFAULT_FRESH_MIN,
    );
    assert_eq!(
        sel.decisions,
        vec![
            Decision::HavePr,
            Decision::ClosedIssue,
            Decision::HavePr,
            Decision::Unattempted
        ]
    );
    assert_eq!(sel.candidates, vec![5]);
    assert_eq!(sel.report.finished_patches, 4);
    assert_eq!(sel.report.considered, 1);
    assert_eq!(sel.report.have_pr, 2);
    assert_eq!(sel.report.closed_issue, 1);
    assert_eq!(sel.report.unattempted, 1);
    assert_eq!(sel.report.candidates, 1);
    assert!(sel.report.reconciles());
}

/// The report line carries the dimensions the old line did not have:
/// `stale=` and `fresh=` are present and the line reconciles.
#[test]
fn report_line_carries_fresh_and_stale_dimensions() {
    let now = 1_000_000u64;
    let patches = vec![
        patch(1, "sha-1-new", now - 3_600), // stale: record with old key
        patch(2, "sha-2", now - 60),        // fresh: matching key, recent
        patch(3, "sha-3", now - 3_600),     // unattempted
    ];
    let records = vec![
        AttemptRecord::new(1, "sha-1-old", now - 7_200),
        AttemptRecord::new(2, "sha-2", now - 7_200),
    ];

    let sel = select_candidates(&patches, &records, now, DEFAULT_FRESH_MIN);
    assert_eq!(sel.report.stale, 1);
    assert_eq!(sel.report.fresh, 1);
    assert_eq!(sel.report.unattempted, 1);
    assert_eq!(sel.candidates, vec![1, 2, 3]);
    assert!(sel.report.reconciles());
    let line = sel.report.line();
    assert!(line.contains("stale=1"), "line: {line}");
    assert!(line.contains("fresh=1"), "line: {line}");
    assert!(line.contains("candidates=3"), "line: {line}");
}

/// A report whose buckets do not reconcile is a state that cannot exist —
/// the predicate catches a hand-built lie.
#[test]
fn non_reconciling_report_is_detectable() {
    let ok = select_candidates(&[patch(1, "sha-1", 0)], &[], 100, DEFAULT_FRESH_MIN);
    assert!(ok.report.reconciles());

    let lying = ok.report.clone();
    // candidates claims more than stale+fresh+unattempted can cover.
    let lying = autospec_core::memo_key::SelectionReport {
        candidates: lying.candidates + 1,
        ..lying
    };
    assert!(!lying.reconciles());
}

/// End-to-end invariant: work completed within the window that reached
/// neither a PR nor a held line is named, with age.
#[test]
fn reconcile_names_missing_work() {
    let now = 1_000_000u64;
    let completed = vec![
        work(42, "sha-42", now - 1_200), // nothing
        work(456, "sha-456", now - 55),  // nothing
    ];
    let report = reconcile(&completed, &[], &[], now, DEFAULT_RECONCILE_WINDOW);
    assert_eq!(report.completed, 2);
    assert_eq!(report.as_pr, 0);
    assert_eq!(report.as_held, 0);
    assert_eq!(report.missing.len(), 2);
    assert!(report.reconciles());
    assert_eq!(
        report.missing[0],
        autospec_core::memo_key::MissingWork {
            issue: 42,
            input_key: "sha-42".into(),
            completed_at: now - 1_200,
            age: 1_200
        }
    );
    let line = report.line();
    assert!(line.contains("#42 age=1200s"), "line: {line}");
    assert!(line.contains("#456 age=55s"), "line: {line}");
}

/// A held line satisfies the invariant: the work is not a PR, but its
/// absence from candidacy is on record.
#[test]
fn reconcile_held_line_satisfies_invariant() {
    let now = 1_000_000u64;
    let completed = vec![work(7, "sha-7", now - 3_600)];
    let held = vec![HeldLine {
        issue: 7,
        reason: "branch conflict, holding".into(),
    }];
    let report = reconcile(&completed, &[], &held, now, DEFAULT_RECONCILE_WINDOW);
    assert_eq!(report.as_held, 1);
    assert!(report.missing.is_empty());
    assert!(report.reconciles());
    assert_eq!(
        report.line(),
        "reconciled window=3600s completed=1 pr=0 held=1 missing=0"
    );
}

/// A PR takes precedence over a held line when both exist.
#[test]
fn reconcile_pr_takes_precedence() {
    let now = 1_000_000u64;
    let completed = vec![work(9, "sha-9", now - 60)];
    let prs = vec![PrOutcome { issue: 9, pr: 1234 }];
    let held = vec![HeldLine {
        issue: 9,
        reason: "stale hold".into(),
    }];
    let report = reconcile(&completed, &prs, &held, now, DEFAULT_RECONCILE_WINDOW);
    assert_eq!(report.as_pr, 1);
    assert_eq!(report.as_held, 0);
    assert!(report.missing.is_empty());
    assert!(report.reconciles());
}

/// Window bounds: work older than the window is outside, work stamped in
/// the future is inside with age 0.
#[test]
fn reconcile_window_bounds() {
    let now = 1_000_000u64;
    let completed = vec![
        work(1, "sha-1", now - DEFAULT_RECONCILE_WINDOW - 1), // outside
        work(2, "sha-2", now - DEFAULT_RECONCILE_WINDOW),     // exactly on the horizon
        work(3, "sha-3", now + 120),                          // future: inside, age 0
    ];
    let report = reconcile(&completed, &[], &[], now, DEFAULT_RECONCILE_WINDOW);
    assert_eq!(report.outside_window, 1);
    assert_eq!(report.completed, 2);
    // Both inside works are missing (no PR, no held line).
    assert_eq!(report.missing.len(), 2);
    let future = report.missing.iter().find(|m| m.issue == 3).unwrap();
    assert_eq!(future.age, 0);
    assert!(report.reconciles());
    // `reconciles` covers in-window work only; outside is tracked but not
    // part of the invariant.
    assert_eq!(
        report.line(),
        "reconciled window=3600s completed=2 pr=0 held=0 missing=2 \
         [#2 age=3600s #3 age=0s]"
    );
}

/// A custom window narrows what must reconcile.
#[test]
fn reconcile_custom_window() {
    let now = 1_000_000u64;
    let completed = vec![work(5, "sha-5", now - 3_000)];
    let report = reconcile(&completed, &[], &[], now, 600);
    assert_eq!(report.window, 600);
    assert_eq!(report.outside_window, 1);
    assert_eq!(report.completed, 0);
    assert!(report.reconciles());
}
