//! The runner's verdict is a gate, or it is not one (issue #4027).
//!
//! The regression tests run in the configuration the incident required: a
//! runner that graded its own output, recorded the verdict in
//! `status.txt` (with `new-failures.txt` naming the exact test), and then
//! wrote the patch to `changes.patch` anyway — three patches (#3726,
//! #3820, #3831) carried `status=NEW-TEST-FAILURES`, the conversion pass
//! re-ran the suite locally on each to reach the same conclusion, and each
//! broken patch blocked its own issue from re-dispatch because a patch on
//! disk marks the issue as produced.

use autospec_core::run_status::Status;
use autospec_core::runner_verdict::{
    companion_files, consume, eligibility, held_line, is_failing, landing_verification,
    publish_audit, rejected_path, route, ConsumeAction, Eligibility, LandingVerification,
    PublishAudit, PublishTarget, RunnerVerdict, CONVERSION_CANDIDATE, NEW_FAILURES_FILE,
    STATUS_FILE,
};

// --- Invariant 1: a failing verdict does not publish a candidate ----------

#[test]
fn failing_statuses_are_the_gate_negatives() {
    // The gate ran and produced a negative verdict about the patch — the
    // statuses the issue names (NEW-TEST-FAILURES, FMT-DIRTY, TEST-TIMEOUT)
    // plus BUILD-FAIL, the fourth status in which the gate rejected the
    // patch itself.
    for status in [
        Status::NewTestFailures,
        Status::TestTimeout,
        Status::FmtDirty,
        Status::BuildFail,
    ] {
        assert!(is_failing(status), "{status:?} is a gate negative");
    }
    assert!(!is_failing(Status::Verified));
}

#[test]
fn incident_patches_produce_no_conversion_candidate_and_stay_eligible() {
    // AC6, part 1: a run graded NEW-TEST-FAILURES produces no conversion
    // candidate and leaves its issue eligible. The three held patches
    // (#3726, #3820, #3831) all carried this status with
    // new-failures.txt naming the exact test.
    for status in [
        Status::NewTestFailures,
        Status::FmtDirty,
        Status::TestTimeout,
        Status::BuildFail,
    ] {
        let target = route(status);
        assert!(
            !target.is_conversion_candidate(),
            "{status:?} must not publish a conversion candidate"
        );
        assert_ne!(
            target.path(),
            CONVERSION_CANDIDATE,
            "{status:?} must not be written to the path the converter consumes"
        );
        assert!(
            target.path().starts_with("changes."),
            "the rejected path must sit beside the candidate in the issue's output directory: {}",
            target.path()
        );
        match target {
            PublishTarget::Rejected {
                status: recorded, ..
            } => {
                assert_eq!(recorded, status.as_str());
            }
            other => panic!("expected a rejected target, got {other:?}"),
        }

        // AC4: the broken patch must not mark the issue produced — a patch
        // on disk used to block re-dispatch (#3994's mechanism), and a
        // patch known to be broken is not work in flight.
        match eligibility(Some(status)) {
            Eligibility::Redispatchable { status: recorded } => {
                assert_eq!(recorded, status);
            }
            other => panic!("expected redispatchable, got {other:?}"),
        }
    }
}

#[test]
fn verified_run_produces_one_candidate() {
    // AC6, part 2: a run graded VERIFIED produces a conversion candidate
    // and its issue is produced — the conversion pass owns it.
    let target = route(Status::Verified);
    assert!(target.is_conversion_candidate());
    assert_eq!(target.path(), CONVERSION_CANDIDATE);
    match target {
        PublishTarget::Candidate { path } => assert_eq!(path, CONVERSION_CANDIDATE),
        other => panic!("expected a candidate target, got {other:?}"),
    }
    match eligibility(Some(Status::Verified)) {
        Eligibility::Produced { status } => assert_eq!(status, Status::Verified),
        other => panic!("expected produced, got {other:?}"),
    }
}

#[test]
fn unmeasured_runs_are_not_failures() {
    // The run stopped before the gate (TIMEOUT, NO-OUTPUT,
    // TIMEOUT-NO-OUTPUT) or the gate could not measure
    // (UNKNOWN-NO-BASELINE, NO-TEST-DB): no verdict about the patch exists,
    // so the patch is not rejected on a status that says nothing about it.
    for status in [
        Status::Timeout,
        Status::TimeoutNoOutput,
        Status::NoOutput,
        Status::UnknownNoBaseline,
        Status::NoTestDb,
    ] {
        assert!(
            !is_failing(status),
            "{status:?} did not measure the patch; it is not a failure"
        );
        assert!(route(status).is_conversion_candidate());
    }
}

#[test]
fn rejected_path_is_the_sibling_the_verdict_names() {
    assert_eq!(
        rejected_path(Status::NewTestFailures),
        "changes.rejected-NEW-TEST-FAILURES.patch"
    );
    assert_eq!(
        rejected_path(Status::FmtDirty),
        "changes.rejected-FMT-DIRTY.patch"
    );
    // The rejected path is not the candidate path, and it is exact — the
    // selector counts `changes.patch` files, and none of these is one.
    assert_ne!(rejected_path(Status::NewTestFailures), CONVERSION_CANDIDATE);
}

// --- Invariant 2: the verdict travels with the artifact --------------------

#[test]
fn consumer_reads_the_status_before_the_patch() {
    // Any consumer reading a patch also reads the status recorded for it;
    // a failing status also names the new-failures sidecar.
    assert_eq!(
        companion_files(Some(Status::NewTestFailures)),
        vec![STATUS_FILE, NEW_FAILURES_FILE]
    );
    assert_eq!(
        companion_files(Some(Status::FmtDirty)),
        vec![STATUS_FILE, NEW_FAILURES_FILE]
    );
    assert_eq!(companion_files(Some(Status::Verified)), vec![STATUS_FILE]);
    assert_eq!(companion_files(None), vec![STATUS_FILE]);
}

#[test]
fn converter_holds_on_failing_status_without_invoking_a_gate() {
    // AC6, part 3: a converter handed a failing status holds without
    // invoking a gate, and the hold line is the runner's own.
    let verdict = RunnerVerdict::from_recorded(
        Some("NEW-TEST-FAILURES"),
        vec![
            "tests::a::first".to_string(),
            "tests::b::second".to_string(),
        ],
    )
    .expect("NEW-TEST-FAILURES is in the vocabulary");
    let action = consume(&verdict);
    match &action {
        ConsumeAction::Hold {
            status,
            new_failures,
        } => {
            assert_eq!(*status, Status::NewTestFailures);
            assert_eq!(
                new_failures.as_slice(),
                [
                    "tests::a::first".to_string(),
                    "tests::b::second".to_string()
                ]
            );
        }
        other => panic!("expected a hold, got {other:?}"),
    }
    assert!(
        !action.gate_invoked(),
        "a recorded failing verdict is the reason; the local gate must not re-derive it"
    );
    assert_eq!(
        held_line(Status::NewTestFailures, &verdict.new_failures),
        "held: runner already reported NEW-TEST-FAILURES, new failures: tests::a::first, tests::b::second"
    );
}

#[test]
fn hold_line_without_test_names_carries_the_status_alone() {
    // A FMT-DIRTY or BUILD-FAIL run records no test names: the line names
    // the runner's status and nothing the pass re-derived.
    let verdict = RunnerVerdict::from_recorded(Some("FMT-DIRTY"), vec![])
        .expect("FMT-DIRTY is in the vocabulary");
    match consume(&verdict) {
        ConsumeAction::Hold { status, .. } => {
            assert_eq!(status, Status::FmtDirty);
        }
        other => panic!("expected a hold, got {other:?}"),
    }
    assert_eq!(
        held_line(Status::FmtDirty, &[]),
        "held: runner already reported FMT-DIRTY"
    );
}

#[test]
fn legacy_spelling_holds_through_the_vocabulary() {
    // A consumer must not be fooled by a spelling: the legacy name
    // BUILD-FAILED resolves to the runner's BUILD-FAIL and holds.
    let verdict = RunnerVerdict::from_recorded(Some("BUILD-FAILED"), vec![])
        .expect("BUILD-FAILED is a vocabulary alias");
    assert_eq!(verdict.status, Some(Status::BuildFail));
    match consume(&verdict) {
        ConsumeAction::Hold { status, .. } => assert_eq!(status, Status::BuildFail),
        other => panic!("expected a hold, got {other:?}"),
    }
}

#[test]
fn unknown_status_refuses_naming_the_vocabulary() {
    // A name the vocabulary does not declare is refused, not guessed: the
    // refusal names the file that would authorise the name.
    let error = RunnerVerdict::from_recorded(Some("SOME-NEW-STATUS"), vec![])
        .expect_err("an undeclared status must refuse");
    assert!(
        error.contains("run-status-vocabulary.tsv"),
        "the refusal must name what would tell the consumer: {error}"
    );
    assert!(error.contains("SOME-NEW-STATUS"));
}

#[test]
fn unrecorded_verdict_proceeds_and_gates() {
    // No status next to the patch: the consumer cannot distinguish "never
    // checked" from "checked and failed", so it proceeds and its own gate
    // runs — the one case where the local gate is the first check.
    let verdict = RunnerVerdict::from_recorded(None, vec![]).expect("no status is readable");
    match consume(&verdict) {
        ConsumeAction::Proceed { status } => assert_eq!(status, None),
        other => panic!("expected proceed, got {other:?}"),
    }
    assert!(action_gates(&verdict));
}

#[test]
fn verified_verdict_proceeds_to_confirmation() {
    // A green verdict does not hold: the patch proceeds, and the local
    // gate runs to confirm against the current main — the verdict was
    // earned against the dispatch base, and that base may have moved.
    let verdict = RunnerVerdict::from_recorded(Some("VERIFIED"), vec![])
        .expect("VERIFIED is in the vocabulary");
    match consume(&verdict) {
        ConsumeAction::Proceed { status } => assert_eq!(status, Some(Status::Verified)),
        other => panic!("expected proceed, got {other:?}"),
    }
    assert!(action_gates(&verdict));
}

fn action_gates(verdict: &RunnerVerdict) -> bool {
    consume(verdict).gate_invoked()
}

#[test]
fn unrecorded_issue_is_held_not_redispatched() {
    // Fail-closed: a patch with no recorded verdict is held for the
    // conversion pass — never re-dispatched, never archived on a guess.
    match eligibility(None) {
        Eligibility::Unrecorded => {}
        other => panic!("expected unrecorded, got {other:?}"),
    }
    assert!(!Eligibility::Unrecorded.redispatchable());
    assert!(!eligibility(Some(Status::Verified)).redispatchable());
    assert!(eligibility(Some(Status::NewTestFailures)).redispatchable());
}

// --- Invariant 5: verification runs against the landing tree ---------------

#[test]
fn drifted_base_reroutes_verification_to_the_landing_tree() {
    // AC5: #3818, #3819 and #3832 were genuinely VERIFIED against their
    // own base and still failed on current main — they added a bats suite
    // without registering it, and the registration invariant is a property
    // of the whole tree, not of any one patch. When the dispatch base has
    // drifted from origin/main, verification runs against origin/main.
    let decision = landing_verification(Some("1111111"), Some("2222222"));
    assert!(decision.reverify_needed());
    match decision {
        LandingVerification::ReverifyAgainstLandingTree {
            dispatch_base,
            origin_main,
        } => {
            assert_eq!(dispatch_base, "1111111");
            assert_eq!(origin_main, "2222222");
        }
        other => panic!("expected a reverify, got {other:?}"),
    }
}

#[test]
fn current_base_needs_no_reroute() {
    let decision = landing_verification(Some("abc1234"), Some("abc1234"));
    assert!(!decision.reverify_needed());
    match decision {
        LandingVerification::BaseIsCurrent { base } => assert_eq!(base, "abc1234"),
        other => panic!("expected base-is-current, got {other:?}"),
    }
}

#[test]
fn unreadable_landing_tree_refuses() {
    // Fail-closed: a side that cannot be read is refused, naming what
    // would tell — never a guess that a drift exists or does not.
    for (base, main) in [
        (None, Some("2222222")),
        (Some("1111111"), None),
        (None, None),
        (Some("   "), Some("2222222")),
    ] {
        let decision = landing_verification(base, main);
        match &decision {
            LandingVerification::Unverifiable { detail } => {
                assert!(!detail.is_empty());
                assert!(!decision.reverify_needed());
            }
            other => panic!("expected unverifiable for {base:?}/{main:?}, got {other:?}"),
        }
    }
}

// --- The incident, end to end ----------------------------------------------

#[test]
fn incident_patches_held_without_duplicated_compute() {
    // The incident: three patches carried status=NEW-TEST-FAILURES with
    // new-failures.txt naming the exact test; the pass spent ten minutes
    // per patch re-running the suite to reach the same conclusion. After
    // the fix the pass holds on the runner's own verdict, and each issue
    // is back in the eligible pool.
    let held = [
        ("3726", "tests::unit::test_dispatch::incident_one"),
        ("3820", "tests::unit::test_gate::incident_two"),
        ("3831", "tests::integration::test_pipeline::incident_three"),
    ];
    for (issue, failure) in held {
        let verdict =
            RunnerVerdict::from_recorded(Some("NEW-TEST-FAILURES"), vec![failure.to_string()])
                .expect("recorded by the runner");
        let action = consume(&verdict);
        assert!(
            !action.gate_invoked(),
            "issue #{issue}: the gate must not re-run"
        );
        let line = held_line(Status::NewTestFailures, &verdict.new_failures);
        assert!(
            line.starts_with("held: runner already reported NEW-TEST-FAILURES"),
            "issue #{issue}: {line}"
        );
        assert!(line.contains(failure), "issue #{issue}: {line}");
        assert!(
            eligibility(verdict.status).redispatchable(),
            "issue #{issue} must be eligible for re-dispatch"
        );

        // And the publish that caused it is the finding the audit names:
        // the runner graded the patch failing and wrote it to
        // changes.patch anyway.
        match publish_audit(Some(Status::NewTestFailures), CONVERSION_CANDIDATE) {
            PublishAudit::EmittedDespiteFailure { status, path } => {
                assert_eq!(status, Status::NewTestFailures);
                assert_eq!(path, CONVERSION_CANDIDATE);
                assert!(
                    publish_audit(Some(Status::NewTestFailures), CONVERSION_CANDIDATE)
                        .line()
                        .contains(&rejected_path(Status::NewTestFailures))
                );
            }
            other => panic!("expected the emit-despite-failure finding, got {other:?}"),
        }
        // The fixed publish is clean: the failing artifact went to its
        // sibling path, nothing was discarded, nothing picked up.
        assert!(matches!(
            publish_audit(
                Some(Status::NewTestFailures),
                &rejected_path(Status::NewTestFailures)
            ),
            PublishAudit::Clean
        ));
        assert!(matches!(
            publish_audit(Some(Status::Verified), CONVERSION_CANDIDATE),
            PublishAudit::Clean
        ));
        assert!(matches!(
            publish_audit(None, CONVERSION_CANDIDATE),
            PublishAudit::Clean
        ));
    }
}
