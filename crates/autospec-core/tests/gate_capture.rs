//! Gate results are captured, not narrated (#4007).
//!
//! The scenario these tests hold shut is the one from the issue: a suite of
//! 2907 tests whose report said "2901 passed, 6 failed" — copied truthfully
//! from a summary line — next to prose written from a tail of `ok 67 … ok
//! 69` that described the run as green, with no exit status recorded
//! anywhere. Every test here is a claim about a gate and the evidence that
//! claim is allowed to rest on.

use autospec_core::gate_capture::{
    capture, render_gate_section, review, review_all, CaptureRule, GateClaim, GateDefinition,
    SummaryFormat, TestSummary,
};

/// The #4007 shape in miniature: six failures in the first stretch of a bats
/// run, then a long green tail ending `ok 67`, `ok 68`, `ok 69`.
fn bats_tail_green() -> String {
    let mut out = String::from("1..69\n");
    for n in 1..=6 {
        out.push_str(&format!("not ok {n} suite A case {n}\n"));
    }
    for n in 7..=69 {
        out.push_str(&format!("ok {n} suite A case {n}\n"));
    }
    out
}

fn bats_gate() -> GateDefinition {
    GateDefinition::tap("bats-suite", "bats tests/")
}

fn cargo_gate() -> GateDefinition {
    GateDefinition::cargo_test("rust-tests", "cargo test --workspace")
}

// --- AC1: the result is recorded as data: exit status + parsed counts -----

#[test]
fn a_tap_run_records_its_exit_status_and_its_own_counts() {
    let record = capture(&bats_gate(), 1, &bats_tail_green());
    assert_eq!(record.exit_status, 1);
    assert_eq!(record.summary, Some(TestSummary::new(63, 6)));
    assert_eq!(record.failure_markers, 6);
    assert!(!record.is_green());
}

#[test]
fn cargo_counts_are_summed_across_every_test_binary() {
    // One `cargo test --workspace` invocation prints one `test result:` line
    // per binary. Reading only the last one reports one binary as the run.
    let output = concat!(
        "test a::one ... ok\n",
        "test result: ok. 1871 passed; 0 failed; 0 ignored; 0 measured; finished in 30.39s\n",
        "test b::two ... FAILED\n",
        "test result: FAILED. 297 passed; 3 failed; 0 ignored; 0 measured; finished in 1.02s\n",
        "error: test failed, to rerun pass `-p autospec-cli --lib`\n",
    );
    let record = capture(&cargo_gate(), 101, output);
    assert_eq!(record.summary, Some(TestSummary::new(2168, 3)));
    assert_eq!(record.exit_status, 101);
    assert!(!record.is_green());
}

#[test]
fn a_green_run_records_zero_failures_and_is_green() {
    let record = capture(&cargo_gate(), 0, "test result: ok. 2907 passed; 0 failed\n");
    assert!(record.is_green());
    assert_eq!(record.failed(), 0);
    assert_eq!(record.passed(), Some(2907));
}

#[test]
fn the_record_carries_the_raw_lines_the_counts_came_from() {
    // So a reader can tell a measurement from an estimate without re-running.
    let record = capture(&bats_gate(), 1, &bats_tail_green());
    assert_eq!(
        record.evidence.first().map(String::as_str),
        Some("not ok 1 suite A case 1")
    );
    assert!(record
        .evidence
        .iter()
        .any(|l| l.trim() == "ok 69 suite A case 69"));
    assert!(!record.evidence.iter().any(|l| l.trim() == "1..69"));
}

// --- AC2: a passing claim must agree with the exit status and the counts ---

#[test]
fn the_tail_of_a_failing_run_does_not_make_it_passing() {
    // The #4007 report: the tail says `ok 67 … ok 69`, the run says otherwise.
    let record = capture(&bats_gate(), 1, &bats_tail_green());
    let claim = GateClaim::asserted_pass("bats-suite").with_exit_status(1);
    let findings = review(&claim, Some(&record));
    let rules: Vec<CaptureRule> = findings.iter().map(|f| f.rule).collect();
    assert!(
        rules.contains(&CaptureRule::PassWithFailures),
        "6 failures recorded, pass claim accepted: {findings:?}"
    );
    let failure = findings
        .iter()
        .find(|f| f.rule == CaptureRule::PassWithFailures)
        .expect("a pass claim survived 6 recorded failures");
    assert!(failure.message.contains("6 failing"), "{}", failure.message);
}

#[test]
fn a_pass_claim_carrying_the_wrong_exit_status_is_rejected() {
    // The claim says exit 0; the captured run exited 101. The record wins.
    let record = capture(&cargo_gate(), 101, "test result: ok. 3 passed; 1 failed\n");
    let claim = GateClaim::asserted_pass("rust-tests")
        .with_exit_status(0)
        .with_counts(3, 0);
    let rules: Vec<CaptureRule> = review(&claim, Some(&record))
        .iter()
        .map(|f| f.rule)
        .collect();
    assert!(rules.contains(&CaptureRule::PassWithNonZeroExit));
    assert!(rules.contains(&CaptureRule::NumbersNotCaptured));
}

#[test]
fn a_pass_claim_with_a_nonzero_exit_and_no_record_is_rejected() {
    let claim = GateClaim::asserted_pass("rust-tests")
        .with_exit_status(101)
        .with_counts(2907, 0);
    let rules: Vec<CaptureRule> = review(&claim, None).iter().map(|f| f.rule).collect();
    assert_eq!(rules, vec![CaptureRule::PassWithNonZeroExit]);
}

#[test]
fn a_pass_claim_with_failures_above_zero_is_rejected_even_at_exit_zero() {
    // A runner that reports its own failures and still exits 0 (a gate run
    // without `-D warnings`, a `|| true` in the harness) is not green.
    let claim = GateClaim::asserted_pass("rust-tests")
        .with_exit_status(0)
        .with_counts(2901, 6);
    let rules: Vec<CaptureRule> = review(&claim, None).iter().map(|f| f.rule).collect();
    assert_eq!(rules, vec![CaptureRule::PassWithFailures]);
}

#[test]
fn a_pass_claim_whose_numbers_are_not_the_captured_numbers_is_rejected() {
    // 2901/0 written from a summary line, while the capture says 2907/6.
    let record = capture(&bats_gate(), 1, &bats_tail_green());
    let claim = GateClaim::asserted_pass("bats-suite")
        .with_exit_status(1)
        .with_counts(2901, 0);
    let findings = review(&claim, Some(&record));
    let mismatch = findings
        .iter()
        .find(|f| f.rule == CaptureRule::NumbersNotCaptured)
        .expect("a claim with invented numbers passes the numbers check");
    assert!(mismatch
        .message
        .contains("failed 0 (claim) vs 6 (captured)"));
}

#[test]
fn a_claim_generated_from_a_record_never_fails_review() {
    // The generated path (AC1) and the review path (AC2) agree by construct.
    let green = capture(&cargo_gate(), 0, "test result: ok. 2907 passed; 0 failed\n");
    assert!(review(&green.claim(), Some(&green)).is_empty());
    let red = capture(
        &cargo_gate(),
        101,
        "test result: FAILED. 2901 passed; 6 failed\n",
    );
    // A red record generates a fail claim, and fail claims are not rejected:
    // understating success holds work rather than releasing it.
    assert_eq!(
        red.claim().reported,
        autospec_core::gate_capture::Reported::Fail
    );
    assert!(review(&red.claim(), Some(&red)).is_empty());
}

#[test]
fn review_names_every_bad_gate_rather_than_the_first() {
    let records = vec![
        capture(&cargo_gate(), 0, "test result: ok. 10 passed; 0 failed\n"),
        capture(&bats_gate(), 1, "not ok 1 a\nok 2 b\n"),
    ];
    let claims = vec![
        GateClaim::asserted_pass("rust-tests"),
        GateClaim::asserted_pass("bats-suite"),
        GateClaim::asserted_pass("docs-gate"),
    ];
    let findings = review_all(&claims, &records);
    // rust-tests is green: no findings. bats-suite is red: exit + failures.
    // docs-gate was never captured: refused as unmeasured.
    let gated: Vec<(&str, CaptureRule)> =
        findings.iter().map(|f| (f.gate.as_str(), f.rule)).collect();
    assert!(gated.contains(&("bats-suite", CaptureRule::PassWithNonZeroExit)));
    assert!(gated.contains(&("bats-suite", CaptureRule::PassWithFailures)));
    assert!(gated.contains(&("docs-gate", CaptureRule::ExitStatusUnrecorded)));
    assert!(!gated.iter().any(|(gate, _)| *gate == "rust-tests"));
}

// --- AC3: the failure marker belongs to the gate definition ---------------

#[test]
fn the_failure_marker_comes_with_the_runner_not_the_call() {
    // `capture` takes no marker argument at all: a run cannot pick the
    // pattern that decides its own verdict.
    let def = bats_gate();
    assert_eq!(def.failure_marker, "not ok");
    assert_eq!(cargo_gate().failure_marker, "FAILED");
    assert_eq!(def.summary, SummaryFormat::Tap);
}

#[test]
fn a_tap_marker_is_anchored_so_a_passing_test_named_not_ok_is_not_a_failure() {
    let output = concat!(
        "1..3\n",
        "ok 1 parses the not ok directive\n",
        "ok 2 reports NOT ok status\n",
        "not okay 3 is prose, not a result line\n",
        "not ok 4 genuinely fails\n",
    );
    let record = capture(&bats_gate(), 1, output);
    assert_eq!(record.failure_markers, 1);
    assert_eq!(record.summary, Some(TestSummary::new(2, 1)));
}

#[test]
fn a_gate_with_no_summary_line_counts_its_declared_marker() {
    // e.g. a runner that prints `FAIL:` lines and no totals.
    let def = GateDefinition::markers_only("smoke", "./run-smoke.sh", "FAIL:").unwrap();
    let output = concat!(
        "PASS: bootstrap\n",
        "FAIL: ingest wrote nothing\n",
        "PASS: query\n",
        "FAIL: export empty\n",
        "done\n",
    );
    let record = capture(&def, 1, output);
    assert_eq!(record.summary, None);
    assert_eq!(record.failure_markers, 2);
    // With no summary line, the marker count *is* the failure count.
    assert_eq!(record.failed(), 2);
    assert!(!record.is_green());
}

#[test]
fn a_marker_only_gate_must_declare_its_marker() {
    // An empty marker would count every run as failing nothing.
    assert!(GateDefinition::markers_only("smoke", "./run-smoke.sh", "  ").is_err());
    assert!(GateDefinition::new("smoke", "./run-smoke.sh", SummaryFormat::MarkersOnly).is_err());
    assert!(GateDefinition::new("", "cargo test", SummaryFormat::CargoTest).is_err());
    assert!(GateDefinition::new("tests", "  ", SummaryFormat::CargoTest).is_err());
}

// --- AC4: the named test cases -------------------------------------------

#[test]
fn ac4_a_run_whose_tail_is_green_but_which_failed_is_reported_as_failing() {
    // The issue's own case: tail `ok 67 … ok 69`, six failures behind it.
    let output = bats_tail_green();
    let tail: Vec<&str> = output.lines().rev().take(3).collect();
    assert!(
        tail.iter().all(|l| l.starts_with("ok ")),
        "fixture must end green: {tail:?}"
    );

    let record = capture(&bats_gate(), 1, &output);
    assert!(!record.is_green(), "a green tail made a red run green");
    assert_eq!(record.failed(), 6);

    // And the generated report says so.
    let body = render_gate_section(std::slice::from_ref(&record));
    assert!(body.contains("FAILED — 1 of 1 gate(s) captured are red"));
    assert!(body.contains("exit 1"));
    assert!(body.contains("63 passed, 6 failed"));
    assert!(body.contains("not ok 1 suite A case 1"));
}

#[test]
fn ac4_failures_interleaved_between_later_passes_are_all_counted() {
    // not ok at 3 and 41, green everywhere else including the whole tail.
    let mut output = String::from("1..50\n");
    for n in 1..=50 {
        if n == 3 || n == 41 {
            output.push_str(&format!("not ok {n} case {n}\n"));
        } else {
            output.push_str(&format!("ok {n} case {n}\n"));
        }
    }
    let record = capture(&bats_gate(), 1, &output);
    assert_eq!(record.failure_markers, 2);
    assert_eq!(record.summary, Some(TestSummary::new(48, 2)));
    assert!(!record.is_green());

    // The tail of this output is 9 green lines; a claim of zero failures
    // against it is rejected with the count that the tail hides.
    let claim = GateClaim::asserted_pass("bats-suite")
        .with_exit_status(0)
        .with_counts(48, 0);
    let findings = review(&claim, Some(&record));
    assert!(findings
        .iter()
        .any(|f| f.rule == CaptureRule::PassWithFailures && f.message.contains("2 failing")));
}

#[test]
fn ac4_a_gate_claim_with_no_exit_status_recorded_is_rejected() {
    let findings = review(&GateClaim::asserted_pass("rust-tests"), None);
    assert_eq!(
        findings.len(),
        2,
        "expected exit + count findings: {findings:?}"
    );
    assert_eq!(findings[0].rule, CaptureRule::ExitStatusUnrecorded);
    assert!(findings[0].message.contains("no exit status recorded"));
    assert_eq!(findings[1].rule, CaptureRule::FailureCountUnrecorded);
}

#[test]
fn ac4_a_pass_claim_with_no_count_recorded_is_rejected() {
    // An exit status alone is not a verdict: the run may have been truncated
    // before its summary, and nothing was counted.
    let claim = GateClaim::asserted_pass("rust-tests").with_exit_status(0);
    let findings = review(&claim, None);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, CaptureRule::FailureCountUnrecorded);
    assert!(findings[0].message.contains("read the end"));
}

// --- rendering: the PR body is generated from the records -----------------

#[test]
fn the_rendered_section_states_every_gate_and_counts_the_unquoted_evidence() {
    let records = vec![
        capture(
            &cargo_gate(),
            0,
            "test result: ok. 2907 passed; 0 failed; 0 ignored\n",
        ),
        capture(&bats_gate(), 1, &bats_tail_green()),
    ];
    let body = render_gate_section(&records);
    assert!(body.starts_with("## Gate evidence (captured)"));
    assert!(body.contains("FAILED — 1 of 2 gate(s) captured are red"));
    assert!(body.contains("`cargo test --workspace`: GREEN — exit 0"));
    assert!(body.contains("`bats tests/`: RED — exit 1"));
    // Only MAX_EVIDENCE_LINES are quoted; the rest are counted, not dropped.
    assert!(body.contains("more counted line(s), all read, none quoted"));
    assert!(!body.contains("ok 69 suite A case 69"));
}

#[test]
fn a_report_with_no_gates_says_it_proved_nothing() {
    let body = render_gate_section(&[]);
    assert!(body.contains("NO GATES RECORDED"));
}

#[test]
fn rule_ids_are_stable() {
    // Reports and dashboards quote these; renumbering is a breaking change.
    assert_eq!(CaptureRule::ExitStatusUnrecorded.id(), "GC-001");
    assert_eq!(CaptureRule::PassWithNonZeroExit.id(), "GC-002");
    assert_eq!(CaptureRule::PassWithFailures.id(), "GC-003");
    assert_eq!(CaptureRule::FailureCountUnrecorded.id(), "GC-004");
    assert_eq!(CaptureRule::NumbersNotCaptured.id(), "GC-005");
}
