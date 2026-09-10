use super::*;

fn result_line(passed: u64, failed: u64) -> String {
    format!(
            "test result: ok. {passed} passed; {failed} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s"
        )
}

#[test]
fn zero_test_run_is_no_tests_ran_not_a_pass() {
    // The original defect: the last (empty) target read by `tail -1`.
    let output = "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
    let verdict = judge_test_run(output);

    assert_eq!(verdict.outcome, TestRunOutcome::NoTestsRan);
    assert!(!verdict.is_passed());
    assert_eq!(
        verdict.evidence(),
        "NO-TESTS-RAN — 0 passed; 0 failed across 1 target"
    );
}

#[test]
fn empty_output_is_no_tests_ran() {
    let verdict = judge_test_run("");

    assert_eq!(verdict.outcome, TestRunOutcome::NoTestsRan);
    assert!(!verdict.is_passed());
    assert_eq!(
        verdict.evidence(),
        "NO-TESTS-RAN — 0 passed; 0 failed across 0 targets"
    );
}

#[test]
fn sums_across_all_targets_and_reports_the_aggregate() {
    // 14 non-empty targets plus the empty one a `tail -1` would have read.
    let mut output = String::new();
    for _ in 0..14 {
        output.push_str(&result_line(8, 0));
        output.push('\n');
    }
    output.push_str(&result_line(6, 0));
    output.push('\n');

    let verdict = judge_test_run(&output);

    assert_eq!(verdict.outcome, TestRunOutcome::Passed);
    assert!(verdict.is_passed());
    assert_eq!(verdict.evidence(), "118 passed; 0 failed across 15 targets");
}

#[test]
fn a_failure_in_any_target_fails_the_run() {
    // Real fail-fast output: the failing target's result line, then the
    // abort. No `--no-fail-fast` summary, so the aggregate is a lower bound.
    let output = format!(
        "{}\n{}\n{}\n",
        result_line(100, 0),
        "test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s",
        "error: test failed, to rerun pass `--test failing`"
    );
    let verdict = judge_test_run(&output);

    assert_eq!(verdict.outcome, TestRunOutcome::Failed);
    assert!(!verdict.completed);
    assert!(!verdict.is_passed());
    assert!(
        verdict
            .evidence()
            .starts_with("103 passed; 2 failed across 2 targets"),
        "evidence must still carry the aggregate: {}",
        verdict.evidence()
    );
    assert!(
        verdict.evidence().contains("lower bound"),
        "truncated run must label its count: {}",
        verdict.evidence()
    );
}

#[test]
fn a_fail_fast_abort_is_labelled_a_lower_bound() {
    // The #4131 shape: main's run stopped at its first failing target and
    // the later failing targets never ran. The count must not read as a
    // measurement.
    let output = format!(
        "{}\n{}\n",
        result_line(989, 2),
        "error: test failed, to rerun pass `-p autospec-cli --test conductor`"
    );
    let verdict = judge_test_run(&output);

    assert_eq!(verdict.outcome, TestRunOutcome::Failed);
    assert!(!verdict.completed);
    assert_eq!(verdict.aggregate.failed, 2);
    assert!(
        verdict
            .evidence()
            .contains("2 failed across 1 target (lower bound"),
        "{}",
        verdict.evidence()
    );
    assert!(
        verdict.evidence().contains("without fail-fast"),
        "the label must say how to make the count complete: {}",
        verdict.evidence()
    );
}

#[test]
fn a_build_failure_truncates_the_run_too() {
    // A target that never built produces no `test result:` line at all; the
    // counts from the targets that did run are a lower bound.
    let output = format!(
        "{}\n{}\n",
        result_line(511, 0),
        "error: could not compile `crateb` (lib test) due to 1 previous error"
    );
    let verdict = judge_test_run(&output);

    assert_eq!(verdict.outcome, TestRunOutcome::Failed);
    assert!(!verdict.completed);
    assert!(verdict.evidence().contains("lower bound"));
}

#[test]
fn a_target_that_never_built_fails_the_run() {
    let output = format!(
        "{}\nerror: could not compile `autospec-cli` (test \"it\"): 1 error emitted\n",
        result_line(511, 0)
    );
    let verdict = judge_test_run(&output);

    assert_eq!(verdict.outcome, TestRunOutcome::Failed);
    assert!(!verdict.is_passed());
    assert!(
        !verdict.completed,
        "a build failure stops the run: the counts are a lower bound"
    );
}

#[test]
fn a_no_fail_fast_run_reports_every_target_and_is_complete() {
    // A failure in an early target does not hide a later one: with
    // `--no-fail-fast` every target runs and the numbered summary marks the
    // failure set as the full failure set. The per-target singular abort
    // lines are present but are not the completeness marker.
    let mut output = String::new();
    output.push_str("     Running unittests src/lib.rs (cratea)\n");
    output.push_str(&result_line(500, 2));
    output.push_str("error: test failed, to rerun pass `-p cratea --lib`\n");
    output.push_str("     Running unittests src/lib.rs (crateb)\n");
    output.push_str(&result_line(708, 2));
    output.push_str("error: test failed, to rerun pass `-p crateb --lib`\n");
    output.push_str("error: 2 targets failed:\n    `-p cratea --lib`\n    `-p crateb --lib`\n");

    let verdict = judge_test_run(&output);

    assert_eq!(verdict.outcome, TestRunOutcome::Failed);
    assert!(verdict.completed);
    assert_eq!(verdict.aggregate.passed, 1208);
    assert_eq!(verdict.aggregate.failed, 4);
    assert_eq!(verdict.aggregate.targets, 2);
    assert_eq!(verdict.evidence(), "1208 passed; 4 failed across 2 targets");
}

#[test]
fn a_single_target_no_fail_fast_summary_is_complete() {
    // Cargo prints the singular form for one failing target.
    let output = format!(
        "{}\n{}\n{}\n",
        result_line(100, 1),
        "error: test failed, to rerun pass `-p cratea --lib`",
        "error: 1 target failed:\n    `-p cratea --lib`"
    );
    let verdict = judge_test_run(&output);

    assert_eq!(verdict.outcome, TestRunOutcome::Failed);
    assert!(verdict.completed);
}

#[test]
fn comparing_a_truncated_run_to_a_complete_one_is_rejected() {
    // AC4: a lower bound must not be compared with a measurement.
    let truncated = judge_test_run(&format!(
        "{}\nerror: test failed, to rerun pass `-p a --lib`\n",
        result_line(989, 2)
    ));
    let complete = judge_test_run(&format!(
        "{}\nerror: 1 target failed:\n    `-p a --lib`\n",
        result_line(1208, 2)
    ));

    assert!(!truncated.completed);
    assert!(complete.completed);

    let err = failure_count_delta(&truncated, &complete).unwrap_err();
    assert_eq!(
        err,
        IncomparableRuns {
            baseline_truncated: true,
            current_truncated: false,
        }
    );

    let err = failure_count_delta(&complete, &truncated).unwrap_err();
    assert_eq!(
        err,
        IncomparableRuns {
            baseline_truncated: false,
            current_truncated: true,
        }
    );

    // Two lower bounds are equally incomparable.
    let err = failure_count_delta(&truncated, &truncated).unwrap_err();
    assert!(err.baseline_truncated && err.current_truncated);
    assert!(err.to_string().contains("lower bound"));
}

#[test]
fn a_delta_between_two_complete_runs_measures_the_change() {
    let baseline = judge_test_run(&format!(
        "{}\nerror: 1 target failed:\n    `-p a --lib`\n",
        result_line(989, 4)
    ));
    let current = judge_test_run(&format!(
        "{}\nerror: 1 target failed:\n    `-p a --lib`\n",
        result_line(1208, 2)
    ));

    assert!(baseline.completed && current.completed);
    assert_eq!(failure_count_delta(&baseline, &current), Ok(-2));
}

#[test]
fn a_delta_between_two_passing_runs_is_zero() {
    let baseline = judge_test_run(&result_line(989, 0));
    let current = judge_test_run(&result_line(1208, 0));

    assert!(baseline.completed && current.completed);
    assert_eq!(failure_count_delta(&baseline, &current), Ok(0));
}

#[test]
fn ignored_only_run_is_no_tests_ran() {
    let output = "test result: ok. 0 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
    let verdict = judge_test_run(output);

    assert_eq!(verdict.outcome, TestRunOutcome::NoTestsRan);
    assert_eq!(verdict.aggregate.ignored, 3);
    assert!(!verdict.is_passed());
}

#[test]
fn aggregate_parses_every_field() {
    let output = "test result: ok. 118 passed; 0 failed; 4 ignored; 2 measured; 7 filtered out; finished in 1.23s\n";
    let aggregate = parse_test_run(output);

    assert_eq!(
        aggregate,
        TestRunAggregate {
            passed: 118,
            failed: 0,
            ignored: 4,
            measured: 2,
            filtered: 7,
            targets: 1,
        }
    );
    assert_eq!(aggregate.evidence(), "118 passed; 0 failed across 1 target");
}

#[test]
fn non_result_lines_do_not_count_as_targets() {
    let output = "Running unittests src/lib.rs\nrunning 118 tests\n...\ntest result: ok. 118 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.5s\n";
    let aggregate = parse_test_run(output);

    assert_eq!(aggregate.targets, 1);
    assert_eq!(aggregate.passed, 118);
}

#[test]
fn completion_gate_requires_an_exit_status() {
    let verdict = judge_completion(&CompletionEvidence {
        exit_code: None,
        output: "Finished `dev` profile in 0.42s",
        completion_marker: Some("Finished"),
    });

    assert_eq!(verdict.outcome, CompletionOutcome::NoRunEvidence);
    assert!(!verdict.is_passed());
}

#[test]
fn completion_gate_requires_the_marker_when_declared() {
    // A wrapper that died before emitting output and reported success.
    let verdict = judge_completion(&CompletionEvidence {
        exit_code: Some(0),
        output: "",
        completion_marker: Some("Finished"),
    });

    assert_eq!(verdict.outcome, CompletionOutcome::NoRunEvidence);
    assert!(!verdict.is_passed());
}

#[test]
fn completion_gate_accepts_marker_evidence() {
    let verdict = judge_completion(&CompletionEvidence {
        exit_code: Some(0),
        output: "    Checking autospec-core v0.1.0\n    Finished `dev` profile in 2.10s\n",
        completion_marker: Some("Finished"),
    });

    assert_eq!(verdict.outcome, CompletionOutcome::RanClean);
    assert!(verdict.is_passed());
}

#[test]
fn completion_gate_accepts_silent_tool_by_exit_status() {
    // cargo fmt prints nothing on success; the exit status is the
    // positive evidence of a run.
    let verdict = judge_completion(&CompletionEvidence {
        exit_code: Some(0),
        output: "",
        completion_marker: None,
    });

    assert_eq!(verdict.outcome, CompletionOutcome::RanClean);
    assert!(verdict.is_passed());
}

#[test]
fn completion_gate_fails_on_nonzero_exit() {
    let verdict = judge_completion(&CompletionEvidence {
        exit_code: Some(101),
        output: "error: aborting due to 1 previous error",
        completion_marker: Some("Finished"),
    });

    assert_eq!(verdict.outcome, CompletionOutcome::Failed);
    assert!(!verdict.is_passed());
}
