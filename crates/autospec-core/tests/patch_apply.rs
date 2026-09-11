//! Real-world coverage for apply-step outcome classification (#3980) and
//! the stage guard (#4294).
//!
//! The case that produced the empty hold record on 2026-09-09 (issue
//! #3892): `git apply --3way` failed strictly — the worktree lacked the
//! patch's base blobs — so no conflict markers were left, the
//! `--diff-filter=U` list was empty, and the pass had discarded the
//! command's output, leaving `HELD: does not apply -- ` with nothing
//! after the dash. The classification must quote the captured error, and
//! must refuse to classify a failed apply whose output was not captured
//! at all.
//!
//! The stage-guard case (#4294, InferWeave #288): a generated binary
//! artefact was in the agent's patch, `git apply` refused the binary
//! hunk, and the whole apply aborted — staging zero files and leaving no
//! unmerged paths. The pass then asked what the patch staged, got an
//! empty list, and concluded "already in main": a skip, the one bucket
//! nobody revisits, for work that was intact and convertible. The guard
//! here is what must not exist in that shape: a failed apply can only
//! terminate in a recorded failure, an empty index is a symptom rather
//! than a conclusion, a skip needs positive evidence, and a rejected
//! binary is named with its regeneration.

use autospec_core::execution::{
    captured_error, classify_apply, rejected_binaries, stage_guard, ApplyOutcome,
    ConflictObservation, StageVerdict,
};

/// The exact error `git apply --3way` reports when the repository lacks
/// the blob the patch's index line points at.
const LACKS_OBJECT: &str = "error: repository lacks the necessary object to perform 3-way merge.\n";

/// The shape of the error `git apply` reports when a binary hunk lacks a
/// full index line — the #4294 incident's apply failure.
const BINARY_REJECTED: &str = "error: cannot apply binary patch to 'dist/schema.bin' without full index line\nerror: dist/schema.bin: patch does not apply\n";

#[test]
fn strict_failure_records_the_git_error_verbatim() {
    let outcome = classify_apply(1, LACKS_OBJECT, &[]).expect("strict failure");

    match &outcome {
        ApplyOutcome::StrictFailure {
            error,
            rejected_binaries,
        } => {
            assert_eq!(
                error,
                "error: repository lacks the necessary object to perform 3-way merge."
            );
            assert!(
                rejected_binaries.is_empty(),
                "a missing-blob failure names no binary: {rejected_binaries:?}"
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

// --- Stage guard (#4294) -------------------------------------------------

fn applied() -> ApplyOutcome {
    classify_apply(0, "", &[]).expect("clean apply")
}

/// Invariant 4: a failed apply never falls through to a benign verdict.
/// The #4294 incident's exact shape — the binary rejection aborted the
/// apply, the index was left empty, there were no unmerged paths, and
/// the old guard read the empty index as "already in main". The guard
/// must hold, and the hold must be the failure's recorded line.
#[test]
fn the_incidents_failed_apply_holds_with_the_failure_recorded() {
    let outcome = classify_apply(1, BINARY_REJECTED, &[]).expect("strict failure");
    let staged: Vec<String> = Vec::new(); // the incident's empty index

    let verdict = stage_guard(&outcome, &staged, None).expect("a failed apply decides");
    match &verdict {
        StageVerdict::Held { reason } => {
            assert!(reason.starts_with("HELD: does not apply"), "{reason}");
            assert!(
                reason.contains("strict failure"),
                "the hold is the failure's own record: {reason}"
            );
            let line = verdict.line().expect("a hold renders a line");
            assert_eq!(
                line.as_str(),
                reason.as_str(),
                "the rendered line is the recorded failure"
            );
        }
        StageVerdict::AlreadyInMain { .. } => panic!("a failed apply must never skip"),
        StageVerdict::Proceed { .. } => panic!("a failed apply must never proceed"),
    }
}

/// Invariant 4, the other arm: evidence that the content is in main
/// cannot turn a failed apply into a skip. The pass classified the
/// no-net-change before the apply step; the stage guard is not where a
/// failure quietly becomes a skip.
#[test]
fn evidence_does_not_rescue_a_failed_apply_into_a_skip() {
    let outcome = classify_apply(1, BINARY_REJECTED, &[]).expect("strict failure");
    let staged: Vec<String> = Vec::new();

    let verdict = stage_guard(
        &outcome,
        &staged,
        Some("patch applies in reverse against origin/main"),
    )
    .expect("a failed apply decides");
    match verdict {
        StageVerdict::Held { reason } => {
            assert!(reason.contains("strict failure"), "{reason}");
        }
        other => panic!("a failed apply must hold, got {other:?}"),
    }
}

/// Invariant 5: the same symptom (an empty index) yields different
/// verdicts from different outcomes — the verdict is derived from the
/// operation's own result, not from the index state.
#[test]
fn the_empty_index_is_a_symptom_not_a_conclusion() {
    let staged: Vec<String> = Vec::new();
    let evidence = Some("patch applies in reverse against origin/main");

    // Success, nothing staged, content confirmed in main: a skip.
    let skip = stage_guard(&applied(), &staged, evidence).expect("evidence decides");
    assert!(
        matches!(skip, StageVerdict::AlreadyInMain { .. }),
        "{skip:?}"
    );

    // Failure, nothing staged: a hold. Same index, different outcome,
    // different verdict — the index alone cannot carry the conclusion.
    let failed = classify_apply(1, BINARY_REJECTED, &[]).expect("strict failure");
    let hold = stage_guard(&failed, &staged, evidence).expect("a failed apply decides");
    assert!(matches!(hold, StageVerdict::Held { .. }), "{hold:?}");

    assert_ne!(
        skip.line().as_deref(),
        hold.line().as_deref(),
        "the same symptom must not render the same line"
    );
}

/// Invariant 6: a successful apply that staged nothing cannot skip
/// without positive evidence — the guard refuses to decide instead of
/// guessing.
#[test]
fn a_no_change_apply_refuses_to_skip_without_evidence() {
    let staged: Vec<String> = Vec::new();

    let err = stage_guard(&applied(), &staged, None).expect_err("no evidence, no skip");
    assert!(err.contains("no positive evidence"), "{err}");
    assert!(
        err.contains("an empty index is a symptom"),
        "the refusal names why: {err}"
    );

    // Blank evidence is no evidence, not weaker evidence.
    let err = stage_guard(&applied(), &staged, Some("   \n")).expect_err("blank evidence");
    assert!(err.contains("no positive evidence"), "{err}");
}

/// Invariant 6, the reachable skip: evidence is required, and it is
/// quoted in the skip line so the skip's evidence survives the session
/// that made it.
#[test]
fn the_skip_quotes_the_evidence_that_earned_it() {
    let staged: Vec<String> = Vec::new();
    let evidence = "patch applies in reverse against origin/main (git apply --reverse --check: 0)";

    let verdict = stage_guard(&applied(), &staged, Some(evidence)).expect("evidence decides");
    let line = match &verdict {
        StageVerdict::AlreadyInMain { evidence: e } => {
            assert_eq!(e.as_str(), evidence.trim());
            verdict.line().expect("a skip renders a line")
        }
        other => panic!("expected AlreadyInMain, got {other:?}"),
    };
    assert!(
        line.starts_with("SKIP: stages nothing (already in main)"),
        "{line}"
    );
    assert!(line.contains(evidence.trim()), "{line}");

    // And the skip is silent when there is nothing to say.
    let files = vec!["a.rs".to_string()];
    let proceed = stage_guard(&applied(), &files, None).expect("files staged");
    assert!(
        matches!(proceed, StageVerdict::Proceed { .. }),
        "{proceed:?}"
    );
    assert_eq!(proceed.line(), None, "a proceed renders nothing");
}

/// Invariant 7: the rejected binary is named in the strict-failure hold
/// record, with the regeneration that fixes it — and the instruction is
/// the repo's generator, not a tool name hard-coded here.
#[test]
fn the_binary_rejection_is_named_and_pointed_at_regeneration() {
    let outcome = classify_apply(1, BINARY_REJECTED, &[]).expect("strict failure");

    match &outcome {
        ApplyOutcome::StrictFailure {
            rejected_binaries, ..
        } => {
            assert_eq!(
                rejected_binaries,
                &["dist/schema.bin"],
                "{rejected_binaries:?}"
            );
        }
        other => panic!("expected StrictFailure, got {other:?}"),
    }

    let line = outcome.hold_line().expect("a strict failure holds");
    assert!(
        line.contains("`dist/schema.bin`"),
        "the artefact is named: {line}"
    );
    assert!(line.contains("rebuilt, not patched"), "{line}");
    assert!(
        line.contains("regenerate them with the repo's generator"),
        "the fix is the regeneration, stated generically: {line}"
    );

    // And the hold the stage guard records is the same annotated line.
    let staged: Vec<String> = Vec::new();
    let verdict = stage_guard(&outcome, &staged, None).expect("a failed apply decides");
    let held = match verdict {
        StageVerdict::Held { reason } => reason,
        other => panic!("expected Held, got {other:?}"),
    };
    assert_eq!(held, line, "the guard holds the failure's own record");
}

/// The binary parser: git's stable naming, de-duplicated in order, and
/// the plain `patch does not apply` form is not binary-specific.
#[test]
fn the_binary_parser_matches_gits_naming_only() {
    let multi = "error: cannot apply binary patch to 'b.bin' without full index line\n\
                 error: cannot apply binary patch to 'a.bin' without full index line\n\
                 error: cannot apply binary patch to 'b.bin' without full index line\n\
                 error: c.rs: patch does not apply\n";
    assert_eq!(
        rejected_binaries(multi),
        vec!["b.bin", "a.bin"],
        "deduplicated, order observed, plain rejections excluded"
    );

    // Without the `error:` prefix (some callers feed already-trimmed
    // lines) and with an empty path (malformed) the parser still holds.
    assert_eq!(
        rejected_binaries("cannot apply binary patch to 'x.bin' without full index line"),
        vec!["x.bin"]
    );
    assert_eq!(
        rejected_binaries("cannot apply binary patch to '' without full index line"),
        Vec::<String>::new()
    );
    assert_eq!(rejected_binaries(LACKS_OBJECT), Vec::<String>::new());
}
