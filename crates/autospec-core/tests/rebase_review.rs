//! Regression tests for issue #3688 — rebase before review.
//!
//! Two pull requests, reviewed the same way on the same day, both filed
//! against an older `main`:
//!
//! | PR | diff against main, as filed | after rebasing onto current main |
//! |---|---|---|
//! | a clippy fix | `7 files changed, 1,522 deletions` | **1 line** |
//! | a capacity feature | `60 files changed, 9,349 deletions` | conflicts in four core files |
//!
//! Neither number described the change. Both described how far the base had
//! moved — 2 commits in the first case, 40 in the second.
//!
//! These tests run in the configuration the bug required: a review
//! conducted against the diff *as filed* (base behind), which is the only
//! configuration in which the defect is visible. On a base that is the tip
//! every review stands and every action agrees, so an at-tip-only test
//! cannot see the bug.

use autospec_core::rebase_review::{
    decide, drift_line, record_names_base, review_standing, verdict_reports_drift,
    ConversionVerdict, ReviewRecord, ReviewStanding, StalenessAction, TestResult,
};
use autospec_core::rebaseline::BaseDrift;

/// The base both PRs were filed against.
const BASE: &str = "aaaa1111";
/// The trunk tip at review time.
const TIP: &str = "bbbb2222";

// ---------------------------------------------------------------------------
// The two PRs from the issue, end to end
// ---------------------------------------------------------------------------

/// The clippy fix: 2 commits behind, rebases cleanly. It is late, not
/// conflicted: rebase, re-run the gates, merge.
#[test]
fn a_clean_rebase_on_a_moved_trunk_is_late_not_conflicted() {
    let touched = vec!["crates/autospec-core/src/linted.rs".to_string()];
    let verdict = decide(BASE, TIP, 2, &[], &touched).unwrap();
    assert_eq!(
        verdict.action,
        StalenessAction::RebaseVerifyMerge {
            onto: TIP.to_string()
        }
    );
    assert!(verdict.action.mergeable());
    assert!(!verdict.action.allows_hand_resolution());
    let line = verdict.line();
    assert!(line.contains("2 commit(s) behind main"), "{line}");
    assert!(line.contains("[base=aaaa1111]"), "{line}");
    assert!(line.contains("re-run the full gate set"), "{line}");
}

/// The capacity feature: 40 commits behind, conflicts in the four core
/// files it exists to change. It is obsolete, not late: hold with the
/// conflicting paths recorded, re-dispatch against current `main`, and
/// never hand-resolve.
#[test]
fn a_conflict_in_the_change_own_subject_matter_holds_and_re_dispatches() {
    let conflicting = vec![
        "crates/autospec-core/src/scheduler.rs".to_string(),
        "crates/autospec-core/src/queue.rs".to_string(),
        "crates/autospec-core/src/pool.rs".to_string(),
        "crates/autospec-core/src/gates.rs".to_string(),
    ];
    let touched: Vec<String> = conflicting
        .iter()
        .cloned()
        .chain(["docs/capacity.md".to_string()].into_iter())
        .collect();
    let verdict = decide(BASE, TIP, 40, &conflicting, &touched).unwrap();
    match verdict.action {
        StalenessAction::HoldAndRedispatch { ref files } => {
            assert_eq!(files, &conflicting);
        }
        other => panic!(
            "a subject-matter conflict must hold, got {:?}",
            other.line()
        ),
    }
    assert!(!verdict.action.mergeable());
    assert!(!verdict.action.allows_hand_resolution());
    let line = verdict.action.line();
    // The hold records the conflicting paths and names the release.
    for file in &conflicting {
        assert!(line.contains(file), "hold line omits {file}: {line}");
    }
    assert!(line.contains("re-dispatch"), "{line}");
    assert!(line.contains("never hand-resolve"), "{line}");
    let full = verdict.line();
    assert!(full.contains("40 commit(s) behind main"), "{full}");
}

/// A branch whose only conflicts sit outside the files it changes is stale,
/// not conflicted: the rebase is mechanical and the issue is not re-dispatched.
#[test]
fn a_conflict_outside_the_change_surface_is_staleness() {
    let conflicting = vec!["src/generated/bindings.rs".to_string()];
    let touched = vec!["src/capacity.rs".to_string()];
    let verdict = decide(BASE, TIP, 40, &conflicting, &touched).unwrap();
    assert_eq!(
        verdict.action,
        StalenessAction::RebaseVerifyMerge {
            onto: TIP.to_string()
        }
    );
}

/// A base that is the tip leaves the review standing as filed: verify and
/// merge, no rebase.
#[test]
fn a_current_base_verifies_and_merges_as_filed() {
    let verdict = decide(TIP, TIP, 0, &[], &["src/a.rs".to_string()]).unwrap();
    assert_eq!(verdict.action, StalenessAction::VerifyAndMerge);
    assert!(verdict.action.mergeable());
    assert!(drift_line(&verdict.drift).contains("[base=bbbb2222]"));
}

// ---------------------------------------------------------------------------
// Invariant 1 — no review against a diff whose base is behind
// ---------------------------------------------------------------------------

/// Both PRs were reviewed against a base that had moved. Neither review
/// stands as recorded: the diff described the base's movement, not the
/// change.
#[test]
fn a_review_against_a_moved_base_is_void() {
    assert_eq!(
        review_standing(BASE, TIP, 2),
        Ok(ReviewStanding::StaleBase { commits_behind: 2 })
    );
    assert_eq!(
        review_standing(BASE, TIP, 40),
        Ok(ReviewStanding::StaleBase { commits_behind: 40 })
    );
    let void = review_standing(BASE, TIP, 2).unwrap();
    assert!(!void.valid());
    assert!(void.line().contains("rebase before reading the diff"));
}

/// A review against the tip stands.
#[test]
fn a_review_against_the_tip_stands() {
    assert_eq!(review_standing(TIP, TIP, 0), Ok(ReviewStanding::Valid));
    assert!(review_standing(TIP, TIP, 0).unwrap().valid());
}

/// A review whose drift was assumed rather than measured is refused, not
/// graded: identical ids with a non-zero count, different ids with a zero.
#[test]
fn a_review_refuses_an_assumed_drift() {
    assert!(review_standing(TIP, TIP, 3).is_err());
    assert!(review_standing(BASE, TIP, 0).is_err());
    assert!(review_standing("", TIP, 3).is_err());
}

/// A review record names the base it was performed against, the same way
/// the conversion pass records `[base=<sha>]`.
#[test]
fn a_review_record_names_its_base() {
    let record = ReviewRecord::new(4306, BASE, TIP, 2).unwrap();
    let line = record.line();
    assert!(line.contains("[base=aaaa1111]"), "{line}");
    assert!(record_names_base(&line), "{line}");
    assert!(!record.standing().valid());
}

/// A review line without a base is not a review anyone can grade.
#[test]
fn a_review_line_without_a_base_is_not_gradable() {
    assert!(!record_names_base("review of PR 4306: LGTM"));
    assert!(!record_names_base(
        "review PR 4306 [base=] — 2 commit(s) behind"
    ));
    assert!(record_names_base("review PR 4306 [base=aaaa1111]"));
}

// ---------------------------------------------------------------------------
// Invariant 3 — every conversion verdict reports its base drift
// ---------------------------------------------------------------------------

/// The incident's merge record said `9065 passed, 0 failing` — a true
/// statement that carried no signal about which base it was measured on.
/// The verdict line must carry the drift on the same line as the result.
#[test]
fn a_conversion_verdict_reports_its_drift() {
    let verdict = ConversionVerdict::new(
        TestResult::new(9065, 0),
        BaseDrift::measure(BASE, TIP, 40).unwrap(),
    );
    let line = verdict.line();
    assert!(line.contains("9065 passed, 0 failing"), "{line}");
    assert!(line.contains("40 commit(s) behind main"), "{line}");
    assert!(line.contains("[base=aaaa1111]"), "{line}");
    assert!(verdict_reports_drift(&line), "{line}");
}

/// A verdict at the tip still names its base and its count — "0 behind" is
/// a measurement like any other.
#[test]
fn a_current_verdict_names_its_base_and_count() {
    let verdict = ConversionVerdict::new(
        TestResult::new(120, 0),
        BaseDrift::measure(TIP, TIP, 0).unwrap(),
    );
    let line = verdict.line();
    assert!(line.contains("[base=bbbb2222]"), "{line}");
    assert!(verdict_reports_drift(&line), "{line}");
}

/// The failure this invariant exists for: a verdict that carries the test
/// result but not the base drift is green and unfalsifiable — it could
/// belong to any base.
#[test]
fn a_verdict_silent_about_its_drift_is_not_legible() {
    assert!(!verdict_reports_drift("9065 passed, 0 failing"));
    assert!(!verdict_reports_drift(
        "120 passed, 0 failing — 40 commit(s) behind main"
    ));
}
