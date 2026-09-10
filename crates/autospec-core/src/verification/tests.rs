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
    let output = format!(
            "{}\n{}\n",
            result_line(100, 0),
            "test result: FAILED. 3 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s\n"
        );
    let verdict = judge_test_run(&output);

    assert_eq!(verdict.outcome, TestRunOutcome::Failed);
    assert!(!verdict.is_passed());
    assert_eq!(verdict.evidence(), "103 passed; 2 failed across 2 targets");
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
