//! Dispatch eligibility of stored output: a produced patch has a lifecycle,
//! not a terminal state (issue #3994).
//!
//! The incident: the dispatcher skipped any issue whose
//! `out/issue-N/changes.patch` existed, and nothing ever removed a patch —
//! "produced" was a terminal state with no exit. `main` moved (78 PRs merged
//! in a day), the patches stopped applying, the conversion pass held them,
//! and each held patch removed one issue from the queue forever. 205 of 212
//! queued issues became undispatchable and the fleet idled at 8/22.
//!
//! The exit that matters is the cheap one the guard already computes: does
//! the patch still apply to the current base? The eligibility check
//! separates "patch exists and applies" (skip, awaiting conversion) from
//! "patch exists and does not apply" (retire and re-dispatch), with the
//! 3-way-merge boundary the conversion pass actually uses.
//!
//! Acceptance scenarios, in order:
//!
//! * a patch that no longer applies is retired to an archive path (archival,
//!   never deletion) and its issue becomes dispatch-eligible again (AC1);
//! * the eligibility check distinguishes applies (skip) from does-not-apply
//!   (retire and re-dispatch), including the boundary where the patch
//!   applies only under 3-way merge (AC2);
//! * an idle pass over a fully-blocked queue names its dominant skip reason
//!   and count (AC3);
//! * an issue whose patch has been retired appears in the eligible set on
//!   the next pass, and an issue whose patch still applies does not (AC4).

use autospec_core::stored_output::{
    classify, dominant_skip_line, eligibility, skip_reason_counts, superseded_archive_path,
    ApplyCheck, Eligibility, OutputEvidence, OutputState, SkipReasonCount,
};

fn evidence(apply_check: Option<ApplyCheck>) -> OutputEvidence {
    OutputEvidence {
        patch_present: true,
        converted: false,
        conversion_failed: false,
        apply_check,
    }
}

// --- AC2: the two branches of the eligibility check -----------------------

#[test]
fn a_patch_that_applies_is_a_skip_not_a_redispatch() {
    let live = evidence(Some(ApplyCheck::Applies));
    assert_eq!(eligibility(&live), Eligibility::AwaitingConversion);
    assert!(!eligibility(&live).is_dispatchable());
    assert_eq!(classify(&live), Some(OutputState::AwaitingConversion));
}

#[test]
fn a_patch_that_does_not_apply_is_retired_and_redispatched() {
    let superseded = evidence(Some(ApplyCheck::Rejected));
    assert_eq!(eligibility(&superseded), Eligibility::RetireSuperseded);
    // Retirement returns the issue to the eligible pool on the next pass.
    assert!(eligibility(&superseded).is_dispatchable());
    assert_eq!(classify(&superseded), Some(OutputState::Superseded));
}

/// AC2 boundary: the conversion pass applies with `--3way`, not strict
/// `--check`. A patch that strict `--check` rejects but `--3way` applies is
/// still convertible — it must not be retired.
#[test]
fn a_patch_that_applies_only_under_3way_is_still_live() {
    let boundary = evidence(Some(ApplyCheck::AppliesUnder3way));
    assert_eq!(eligibility(&boundary), Eligibility::AwaitingConversion);
    assert!(!eligibility(&boundary).is_dispatchable());
    assert_eq!(classify(&boundary), Some(OutputState::AwaitingConversion));

    // The three-way boundary is what separates the two branches: the same
    // strict rejection is *not* superseded when 3-way still applies.
    let dead = evidence(Some(ApplyCheck::Rejected));
    assert_eq!(eligibility(&dead), Eligibility::RetireSuperseded);
    assert_ne!(eligibility(&boundary), eligibility(&dead));
}

#[test]
fn an_apply_check_that_cannot_answer_is_fail_closed_never_retired() {
    let unrunnable = evidence(Some(ApplyCheck::Unrunnable {
        detail: "git apply: not a git repository".to_string(),
    }));
    assert_eq!(eligibility(&unrunnable), Eligibility::AwaitingConversion);
    let never_run = evidence(None);
    assert_eq!(eligibility(&never_run), Eligibility::AwaitingConversion);
}

// --- AC1: retirement is archival, and the patch is not deleted ------------

#[test]
fn a_superseded_patch_is_retired_to_the_issue_archive_path() {
    let issue_dir = std::path::Path::new("out/issue-205");
    let path = superseded_archive_path(issue_dir, "changes.patch", 1_770_000_000);
    assert_eq!(
        path,
        std::path::Path::new("out/issue-205/superseded/changes-1770000000.patch")
    );
    // The retired patch keeps its stem and gains the retirement timestamp;
    // nothing is deleted, so the content survives under the issue's
    // superseded/ directory.
    let parent = path.parent().unwrap();
    assert_eq!(parent, std::path::Path::new("out/issue-205/superseded"));
}

#[test]
fn the_archive_path_handles_a_patch_name_without_an_extension() {
    let path = superseded_archive_path(std::path::Path::new("out/issue-7"), "patch", 1_770_000_001);
    assert_eq!(
        path,
        std::path::Path::new("out/issue-7/superseded/patch-1770000001.patch")
    );
}

// --- AC4: retirement returns the issue to the eligible set -----------------

#[test]
fn a_retired_patch_reappears_in_the_eligible_set_and_a_live_one_does_not() {
    // The next pass after retirement: the patch was moved to superseded/, so
    // no unconverted patch is on record in the issue dir.
    let retired = OutputEvidence::default();
    assert_eq!(eligibility(&retired), Eligibility::Dispatch);
    assert!(eligibility(&retired).is_dispatchable());

    // The issue whose patch still applies stays out of the eligible set.
    let still_applies = evidence(Some(ApplyCheck::Applies));
    assert_eq!(eligibility(&still_applies), Eligibility::AwaitingConversion);
    assert!(!eligibility(&still_applies).is_dispatchable());

    // A converted patch also does not block a re-dispatch at the patch level.
    let converted = OutputEvidence {
        patch_present: true,
        converted: true,
        conversion_failed: false,
        apply_check: Some(ApplyCheck::Rejected),
    };
    assert_eq!(eligibility(&converted), Eligibility::Dispatch);
}

// --- AC3: an idle pass over a fully-blocked queue names its reason ---------

#[test]
fn an_idle_pass_names_the_dominant_skip_reason_and_its_count() {
    let counts = skip_reason_counts([
        "patch exists",
        "in-flight",
        "patch exists",
        "patch exists",
        "queue-hold",
    ]);
    assert_eq!(
        counts,
        vec![
            SkipReasonCount {
                reason: "patch exists".to_string(),
                count: 3,
            },
            SkipReasonCount {
                reason: "in-flight".to_string(),
                count: 1,
            },
            SkipReasonCount {
                reason: "queue-hold".to_string(),
                count: 1,
            },
        ]
    );

    let line = dominant_skip_line(&counts).expect("an idle pass has a reason");
    assert!(line.contains("0 eligible"), "{line}");
    assert!(line.contains("blocked by patch exists (3/5)"), "{line}");
}

#[test]
fn a_pass_that_skipped_nothing_has_no_dominant_reason() {
    assert_eq!(dominant_skip_line(&[]), None);
    // Blank reasons are dropped and never counted as a dominant reason.
    assert_eq!(dominant_skip_line(&skip_reason_counts(["  ", ""])), None);
}

// --- The whole story in one line ------------------------------------------

#[test]
fn the_205_issue_hold_resolves_through_the_threeway_boundary() {
    // The incident's shape: 138 of 207 patches no longer apply even under
    // --3way (superseded → retire), the rest still apply (skip). The strict
    // --check rejection on its own is not enough to retire a patch.
    let strict_only = evidence(Some(ApplyCheck::AppliesUnder3way));
    let dead = evidence(Some(ApplyCheck::Rejected));
    assert_eq!(eligibility(&strict_only), Eligibility::AwaitingConversion);
    assert_eq!(eligibility(&dead), Eligibility::RetireSuperseded);
}
