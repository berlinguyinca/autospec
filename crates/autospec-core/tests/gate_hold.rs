//! The conversion pass's hold reason (issue #3747).
//!
//! The regression tests run in the configuration the incident required: a
//! gate that greps the combined cargo output for `^error(\[|:)` and runs
//! the build check before the test check — so that a test failure is
//! labelled "build error", the 78% of the largest hold category, and the
//! hold record discards the failing test names the run already had in
//! hand.

use autospec_core::execution::patch_pipeline::{classify_gate, GateVerdict};
use autospec_core::gate_hold::{
    classify_output, compare_to_baseline, failing_test_names, failure_kind_from_exit_codes,
    first_compile_error, hold_reason_from_output, is_compile_error_line, is_test_failure_line,
    BaseSha, BaselineVerdict, HoldKind, HoldReason, TestBaseline, NAMED_FAILURE_LIMIT,
    NO_DISCRIMINATING_LINE,
};

/// The incident's classifier, verbatim: `grep -qE '^error(\[|:)'`. Any
/// line starting `error[` or `error:`, and the build check ran first, so
/// it won.
fn old_build_check(output: &str) -> bool {
    output
        .lines()
        .any(|l| l.starts_with("error[") || l.starts_with("error:"))
}

/// The exact line the incident's hold carried for autospec-2755: the
/// patch compiled fine, `validation_runner` failed, and the gate reported
/// a build error.
const INCIDENT_OUTPUT: &str =
    "error: test failed, to rerun pass `-p autospec-core --test validation_runner`\n";

/// A full `cargo test`-style output for one failing test, in cargo's own
/// shape: the per-test stdout section, then the `failures:` summary block
/// with the names indented four spaces, then the rerun hint.
const ONE_FAILING_TARGET: &str = "running 1 test
test managed_project::it_works ... FAILED

failures:

---- managed_project::it_works stdout ----
thread 'managed_project::it_works' panicked at crates/autospec-cli/tests/managed_project.rs:41:5:
assertion `left == right` failed

failures:
    managed_project::it_works

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

error: test failed, to rerun pass `-p autospec-cli --test managed_project`
";

// --- Invariant 1: the classification is the exit code ----------------------

#[test]
fn a_patch_that_compiles_but_fails_tests_is_held_as_a_test_failure() {
    // The incident: build green, tests red. The old classifier grepped
    // `^error(\[|:)` and ran the build check first, so it won.
    assert!(
        old_build_check(INCIDENT_OUTPUT),
        "the old gate did match this output"
    );
    assert_eq!(classify_output(INCIDENT_OUTPUT), Some(HoldKind::Tests));
    assert_eq!(
        failure_kind_from_exit_codes(0, 101),
        Some(HoldKind::Tests),
        "a red test status with a green build is a test failure, whatever the output says"
    );
}

#[test]
fn the_exit_code_classifies_not_the_output() {
    // A test's stdout may print anything, including lines that look like
    // compile errors; the exit codes decide.
    let out = "error[E9999]: fabricated by a test's stdout\nerror: test failed, to rerun pass `--test x`\n";
    let reason = HoldReason::from_gate(0, 101, out, "1 passed; 1 failed across 2 targets")
        .expect("the test status holds the patch");
    assert!(
        matches!(reason, HoldReason::TestsFailed { .. }),
        "the green build and red test status classify Tests: {reason:?}"
    );

    // A red build outranks a red test: with the build red, the test
    // status answers nothing.
    assert!(matches!(
        HoldReason::from_gate(101, 101, out, ""),
        Some(HoldReason::BuildError { .. })
    ));

    // Both green: there is nothing to hold on.
    assert!(HoldReason::from_gate(0, 0, out, "").is_none());
}

#[test]
fn the_exit_code_classification_is_the_pipeline_classifier() {
    // One classification for one fact, two labelings: the hold kind must
    // agree with the conversion pipeline's gate classifier on every
    // (build_rc, test_rc) pair.
    for build_rc in [-1i32, 0, 101] {
        for test_rc in [-1i32, 0, 101] {
            let expected = match classify_gate(build_rc, test_rc) {
                GateVerdict::Green => None,
                GateVerdict::CompileFailure => Some(HoldKind::Compile),
                GateVerdict::TestsFailed => Some(HoldKind::Tests),
            };
            assert_eq!(
                failure_kind_from_exit_codes(build_rc, test_rc),
                expected,
                "build_rc={build_rc} test_rc={test_rc}"
            );
        }
    }
}

// --- Invariant 2: text matches anchor on the discriminator -----------------

#[test]
fn the_compile_discriminator_matches_only_compile_failures() {
    assert!(is_compile_error_line("error[E0308]: mismatched types"));
    assert!(is_compile_error_line(
        "error: could not compile `autospec-cli` (test \"it\"): 1 error emitted"
    ));
    assert!(!is_compile_error_line(
        "error: test failed, to rerun pass `-p autospec-cli --test conductor`"
    ));
    assert!(!is_compile_error_line("error: 2 targets failed:"));
    assert!(!is_compile_error_line("error: build failed"));
    assert!(!is_compile_error_line(
        "error[e308]: a lowercase letter is not a code"
    ));
    assert!(!is_compile_error_line("error: [E308] is not a code"));
}

#[test]
fn the_test_discriminator_matches_only_test_failures() {
    assert!(is_test_failure_line(
        "error: test failed, to rerun pass `-p autospec-core --test validation_runner`"
    ));
    assert!(is_test_failure_line("error: 1 target failed:"));
    assert!(is_test_failure_line("error: 3 targets failed:"));
    assert!(!is_test_failure_line("error[E0308]: mismatched types"));
    assert!(!is_test_failure_line(
        "error: could not compile `autospec-cli`"
    ));
    assert!(!is_test_failure_line("error: build failed"));
    assert!(!is_test_failure_line(
        "error: aborting due to 1 previous error"
    ));
    assert!(!is_test_failure_line(
        "error: failed 12 targets, reversed word order"
    ));
}

#[test]
fn classify_output_matches_whatever_discriminates_and_never_guesses() {
    assert_eq!(
        classify_output(
            "error[E0308]: mismatched types\nerror: aborting due to 1 previous error\n"
        ),
        Some(HoldKind::Compile)
    );
    assert_eq!(
        classify_output("error: could not compile `autospec-core`\n"),
        Some(HoldKind::Compile)
    );
    assert_eq!(
        classify_output("error: test failed, to rerun pass `--lib`\n"),
        Some(HoldKind::Tests)
    );
    assert_eq!(
        classify_output("error: 2 targets failed:\n    `-p autospec-core --lib`\n"),
        Some(HoldKind::Tests)
    );
    // Compile evidence outranks test evidence in one capture: a target
    // that does not build is the stronger fact.
    assert_eq!(
        classify_output("error[E0308]: x\nerror: test failed, to rerun pass `--lib`\n"),
        Some(HoldKind::Compile)
    );
    // Neither discriminator: unclassified is recorded, not guessed.
    assert_eq!(classify_output("error: build failed\n"), None);
    assert_eq!(
        classify_output("error: this file contains an unclosed delimiter\n"),
        None
    );
    assert_eq!(classify_output("all green\n"), None);
}

#[test]
fn the_measured_corpus_is_relabelled_the_way_the_measurement_says() {
    // Measured across a full session of conversion: 38 mislabeled test
    // failures behind "HELD: build error", each carrying cargo's own
    // rerun hint or targets summary. The old gate matched every one of
    // them; the anchored discriminators classify them as tests.
    let mislabeled = [
        "error: test failed, to rerun pass `-p autospec-core --test validation_runner`",
        "error: test failed, to rerun pass `-p autospec-cli --test managed_project`",
        "error: test failed, to rerun pass `-p autospec-cli --test cli_commands`",
        "error: test failed, to rerun pass `-p autospec-cli --bin autospec`",
        "error: 2 targets failed:\n    `-p autospec-core --lib`\n    `-p autospec-cli --bin autospec`",
    ];
    for line in mislabeled {
        assert!(old_build_check(line), "the old gate matched: {line:?}");
        assert_eq!(
            classify_output(line),
            Some(HoldKind::Tests),
            "the anchored discriminators classify it as tests: {line:?}"
        );
    }

    // The 7 genuine compile failures keep their label — by exit code,
    // because some of them (an unclosed delimiter) match neither anchored
    // pattern and are classified by the build gate's status alone. A
    // text-only classifier either mislabels them back into the incident
    // or cannot label them at all; the exit code is the classifier.
    let genuine = [
        "error: this file contains an unclosed delimiter",
        "error[E0308]: mismatched types",
        "error: could not compile `autospec-core`",
    ];
    for line in genuine {
        assert!(old_build_check(line), "the old gate matched: {line:?}");
        let reason = HoldReason::from_gate(101, 0, line, "").expect("a red build holds the patch");
        assert!(
            matches!(reason, HoldReason::BuildError { .. }),
            "a red build classifies Compile: {line:?}"
        );
    }
    // The one that matches no anchored pattern keeps its label from the
    // exit code and says the evidence was not found rather than guessing.
    let reason = HoldReason::from_gate(101, 0, genuine[0], "").unwrap();
    assert!(
        matches!(&reason, HoldReason::BuildError { detail } if detail == NO_DISCRIMINATING_LINE)
    );
    // The ones that do match carry the discriminating line verbatim.
    let reason = HoldReason::from_gate(101, 0, genuine[1], "").unwrap();
    assert!(matches!(&reason, HoldReason::BuildError { detail } if detail == genuine[1]));
}

// --- Invariant 3: every route to a test hold records the same evidence ----

#[test]
fn the_incident_hold_line_is_relabelled_and_named() {
    let out = ONE_FAILING_TARGET;
    let reason = HoldReason::from_gate(0, 101, out, "2720 passed; 1 failed across 43 targets")
        .expect("the test status holds the patch");
    let line = reason.line("-p autospec-cli");
    assert!(
        line.starts_with("HELD: tests failed (-p autospec-cli) --"),
        "{line}"
    );
    assert!(
        !line.contains("build error"),
        "a test failure is never labelled a build error: {line}"
    );
    assert!(line.contains("managed_project::it_works"), "{line}");
    assert!(
        line.contains("2720 passed; 1 failed across 43 targets"),
        "{line}"
    );
    assert!(reason.names_failures(), "the hold names its failing test");
}

#[test]
fn failing_test_names_reads_cargos_summary_block() {
    let names = failing_test_names(ONE_FAILING_TARGET);
    assert_eq!(names, vec!["managed_project::it_works".to_string()]);

    // The per-test stdout section is skipped even when it indents
    // four-or-more spaces; the summary block is what names the tests.
    let out = "failures:\n\n---- a stdout ----\n    an indented stdout line\n\nfailures:\n    a\n    b\n\n";
    assert_eq!(
        failing_test_names(out),
        vec!["a".to_string(), "b".to_string()]
    );

    // Duplicates collapse; an output that names nothing yields no names.
    let out = "failures:\n    a\n    a\n";
    assert_eq!(failing_test_names(out), vec!["a".to_string()]);
    assert!(failing_test_names("test result: ok. 3 passed; 0 failed\n").is_empty());
}

#[test]
fn a_test_hold_with_no_names_to_parse_says_unnamed() {
    let reason = HoldReason::TestsFailed {
        named: Vec::new(),
        aggregate: "1 failed across 2 targets".to_string(),
    };
    let line = reason.line("-p autospec-cli");
    assert!(line.contains("-- unnamed |"), "{line}");
    assert!(!reason.names_failures());
}

#[test]
fn more_than_three_failures_name_three_and_count_the_rest() {
    let named = ["a", "b", "c", "d", "e"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(NAMED_FAILURE_LIMIT, 3);
    let reason = HoldReason::TestsFailed {
        named,
        aggregate: "5 failed across 9 targets".to_string(),
    };
    let line = reason.line("-p autospec-cli");
    assert!(line.contains("-- a, b, c +2 more |"), "{line}");
    assert!(!line.contains("d,"), "{line}");
    assert!(!line.contains(", e"), "{line}");
    assert!(reason.names_failures());
}

#[test]
fn a_build_error_hold_carries_the_discriminating_line_verbatim() {
    let out = "error[E0308]: mismatched types\n  --> crates/autospec-core/src/gate_hold.rs:1:5\n";
    let reason =
        HoldReason::from_gate(101, 101, out, "").expect("the build status holds the patch");
    let line = reason.line("-p autospec-core");
    assert_eq!(line, "HELD: build error -- error[E0308]: mismatched types");
    assert_eq!(
        first_compile_error(out),
        Some("error[E0308]: mismatched types")
    );
}

#[test]
fn the_scrape_only_gate_classifies_on_the_anchors_and_records_unclassified() {
    // The legacy gate that has no exit codes: the anchored discriminators
    // decide, and an output matching neither is recorded, not guessed.
    let reason = hold_reason_from_output(
        ONE_FAILING_TARGET,
        "2720 passed; 1 failed across 43 targets",
    );
    assert!(matches!(
        reason,
        Some(HoldReason::TestsFailed { named, .. }) if named == ["managed_project::it_works".to_string()]
    ));
    assert!(
        hold_reason_from_output("error: this file contains an unclosed delimiter\n", "").is_none(),
        "matching neither discriminator is unclassified, not a build error"
    );
    let reason =
        hold_reason_from_output("error[E0308]: mismatched types\n", "").expect("discriminates");
    assert!(matches!(reason, HoldReason::BuildError { .. }));
}

// --- Invariant 4: names against the baseline, not a count against zero -----

#[test]
fn a_preexisting_failure_on_main_holds_nothing() {
    // The count-against-zero gate holds every patch in the backlog on one
    // pre-existing failure, and the hold says only "1 failing" —
    // indistinguishable from a real regression. Compared by name, the
    // same run charges the patch with nothing.
    let baseline = TestBaseline::new(["flaky_pre_existing"]);
    let verdict = compare_to_baseline(["flaky_pre_existing"], &baseline);
    assert!(!verdict.is_hold());
    assert!(
        verdict
            .hold_line("-p autospec-cli", "2720 passed; 1 failed across 43 targets")
            .is_none(),
        "a pre-existing failure is reported as pre-existing, never charged"
    );
    assert_eq!(
        verdict,
        BaselineVerdict::PreExisting {
            preexisting: vec!["flaky_pre_existing".to_string()]
        }
    );
}

#[test]
fn a_new_failure_is_held_by_name_and_the_preexisting_are_reported_not_charged() {
    let baseline = TestBaseline::new(["flaky_pre_existing"]);
    let verdict = compare_to_baseline(
        [
            "flaky_pre_existing",
            "the_real_regression",
            "another_new_one",
        ],
        &baseline,
    );
    assert!(verdict.is_hold());
    assert_eq!(
        verdict.new_failures(),
        &[
            "another_new_one".to_string(),
            "the_real_regression".to_string()
        ],
        "sorted"
    );
    let line = verdict
        .hold_line("-p autospec-cli", "2720 passed; 3 failed across 43 targets")
        .expect("a holding verdict renders a hold line");
    assert!(
        line.contains("another_new_one, the_real_regression"),
        "{line}"
    );
    assert!(
        line.contains("2720 passed; 3 failed across 43 targets"),
        "{line}"
    );
    assert!(line.contains("(1 pre-existing on baseline)"), "{line}");
    assert!(!line.contains("flaky_pre_existing"), "{line}");
}

#[test]
fn an_empty_run_and_an_empty_baseline_agree_with_the_count_gate() {
    let baseline = TestBaseline::new([] as [String; 0]);
    let verdict = compare_to_baseline([] as [String; 0], &baseline);
    assert!(!verdict.is_hold());
    assert!(
        matches!(verdict, BaselineVerdict::PreExisting { preexisting } if preexisting.is_empty())
    );
    assert!(baseline.is_empty());
}

#[test]
fn the_baseline_collapses_duplicates_and_ignores_order() {
    let baseline = TestBaseline::new(["b", "a", "b"]);
    assert_eq!(baseline.len(), 2);
    assert_eq!(
        baseline.names(),
        &std::collections::BTreeSet::from(["a".to_string(), "b".to_string()])
    );
    assert!(baseline.contains("a"));
    assert!(!baseline.contains("c"));
    let verdict = compare_to_baseline(["b", "a"], &baseline);
    assert!(!verdict.is_hold());
}

// --- Invariant 5: the recorded base is the sha the branch was created from -

#[test]
fn the_incident_base_sha_is_stale_and_the_line_carries_the_real_base() {
    // mainsha captured before the per-patch fetch; the branch cut after
    // it; every line read `[base=bfc62571]` while origin/main had
    // advanced to 1d159336.
    let base = BaseSha::new("1d159336", Some("bfc62571".to_string())).unwrap();
    assert!(base.is_stale());
    let finding = base.stale_finding().expect("a stale sha is a finding");
    assert!(finding.contains("STALE_BASE_SHA"), "{finding}");
    assert!(finding.contains("bfc62571"), "{finding}");
    assert!(finding.contains("1d159336"), "{finding}");
    assert!(finding.contains("after the per-patch fetch"), "{finding}");
    assert_eq!(base.log_token(), "[base=1d159336]");
    assert!(!base.log_token().contains("bfc62571"));
}

#[test]
fn a_fresh_or_dropped_base_is_not_stale() {
    let fresh = BaseSha::new("1d159336", Some("1d159336".to_string())).unwrap();
    assert!(!fresh.is_stale());
    assert!(fresh.stale_finding().is_none());
    // Dropping the token is the other permitted fix: no recorded sha,
    // no finding.
    let dropped = BaseSha::new("1d159336", None).unwrap();
    assert!(!dropped.is_stale());
    assert!(dropped.stale_finding().is_none());
}

#[test]
fn an_empty_base_sha_is_refused() {
    let error = BaseSha::new("  ", None).unwrap_err();
    assert!(error.contains("nonempty"), "{error}");
}
