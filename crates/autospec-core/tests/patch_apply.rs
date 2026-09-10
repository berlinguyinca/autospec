//! Real-world coverage for apply-step outcome classification (#3980).
//!
//! The case that produced the empty hold record on 2026-09-09 (issue
//! #3892): `git apply --3way` failed strictly — the worktree lacked the
//! patch's base blobs — so no conflict markers were left, the
//! `--diff-filter=U` list was empty, and the pass had discarded the
//! command's output, leaving `HELD: does not apply -- ` with nothing
//! after the dash. The classification must quote the captured error, and
//! must refuse to classify a failed apply whose output was not captured
//! at all.

use autospec_core::execution::{captured_error, classify_apply, ApplyOutcome, ConflictObservation};

/// The exact error `git apply --3way` reports when the repository lacks
/// the blob the patch's index line points at.
const LACKS_OBJECT: &str = "error: repository lacks the necessary object to perform 3-way merge.\n";

#[test]
fn strict_failure_records_the_git_error_verbatim() {
    let outcome = classify_apply(1, LACKS_OBJECT, &[]).expect("strict failure");

    match &outcome {
        ApplyOutcome::StrictFailure { error } => {
            assert_eq!(
                error,
                "error: repository lacks the necessary object to perform 3-way merge."
            );
        }
        other => panic!("expected StrictFailure, got {other:?}"),
    }

    let line = outcome.hold_line().expect("a strict failure holds");
    assert_eq!(
        line,
        "HELD: does not apply — strict failure: error: repository lacks the necessary object to perform 3-way merge."
    );
}

#[test]
fn the_historical_empty_reason_is_unrepresentable() {
    // The #3980 bug: the apply failed, the capture was discarded
    // (>/dev/null 2>&1), and nothing was left behind. The old code
    // rendered `HELD: does not apply -- ` from the empty unmerged-file
    // list. The classification must reject this instead — the record
    // would have had no reason.
    let err = classify_apply(1, "", &[]).expect_err("discarded capture");
    assert!(err.contains("must quote the failure it describes"), "{err}");
    assert!(err.contains("instead of discarding it"), "{err}");
}

#[test]
fn the_capture_helper_is_what_the_pass_must_feed_the_classifier() {
    // stderr carries the git error; composing the capture from the real
    // streams is the only way the classifier sees it.
    let capture = captured_error(LACKS_OBJECT, "");
    let outcome = classify_apply(1, &capture, &[]).expect("strict failure");
    assert!(
        outcome
            .hold_line()
            .unwrap()
            .contains("repository lacks the necessary object"),
        "{}",
        outcome.hold_line().unwrap()
    );
}

#[test]
fn conflicted_and_strict_are_distinct_outcomes() {
    // Same exit status, same error stream, different worktree evidence:
    // markers on disk make it a conflict (file list carries the record),
    // an empty worktree makes it a strict failure (error quote carries
    // the record). The two records must not borrow each other's shape.
    let conflicted = ConflictObservation {
        path: "crates/autospec-core/src/insights/mod.rs".to_string(),
        content: "<<<<<<< HEAD\npub mod config;\n=======\npub mod config;\npub mod correlate;\n>>>>>>> feat/insights-correlate\n"
            .to_string(),
    };

    let with_markers = classify_apply(1, LACKS_OBJECT, &[conflicted.clone()]).expect("conflicted");
    let without = classify_apply(1, LACKS_OBJECT, &[]).expect("strict");

    let conflict_line = with_markers.hold_line().expect("conflict holds");
    assert!(conflict_line.contains("conflicted:"), "{conflict_line}");
    assert!(
        conflict_line.contains("module declarations only"),
        "the registry-only flag is in the record: {conflict_line}"
    );
    assert!(
        conflict_line.contains("1 hunk(s)"),
        "the hunk count is in the record: {conflict_line}"
    );

    let strict_line = without.hold_line().expect("strict holds");
    assert!(strict_line.contains("strict failure:"), "{strict_line}");
    assert!(!strict_line.contains("conflicted:"), "{strict_line}");

    assert_ne!(
        conflict_line, strict_line,
        "the two failure shapes must produce different records"
    );
}
