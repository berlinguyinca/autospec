//! Regression tests for the failure-attribution invariants (issue #4331).
//!
//! Incident configuration: the conversion pass extracted failing test ids from
//! the libtest *progress* stream (`test NAME ... FAILED`) and compared that set
//! against the known-failing baseline. Under `cargo test -q` the progress stream
//! is not printed at all — the failures appear only in the `failures:` block and
//! in the `test result:` summary. The extractor returned an empty id set, an
//! empty set matches no baseline entry, and the run passed as "not attributed":
//! a regression could be merged with the summary declaring failures nobody named.
//!
//! The quiet run below is that configuration. A verbose run whose progress stream
//! and name list agree cannot see the defect — every extractor, right or wrong,
//! returns the same set there — so the quiet and truncated runs are the ones that
//! must fail if any of the four invariants regresses.

use autospec_core::failure_attribution::{
    attribute, authoritative_names, progress_stream_names, TargetVerdict,
};

/// The incident: `cargo test -q`. No `test NAME ... FAILED` lines anywhere; the
/// only name evidence is the `failures:` block.
fn quiet_run_log() -> String {
    "running 25 tests
.........................
failures:

---- conversion::test_patch_applies stdout ----
thread 'conversion::test_patch_applies' panicked at src/conversion.rs:88:9:
patch does not apply

---- sizing::test_layer_within_cap stdout ----
assertion failed: added + deleted <= cap

failures:
    conversion::test_patch_applies
    sizing::test_layer_within_cap

test result: FAILED. 23 passed; 2 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.04s
"
    .to_string()
}

/// A verbose run: progress lines, a detail block and a name list, all agreeing.
/// The configuration the pre-fix extractor handled correctly — the control.
fn verbose_run_log() -> String {
    "     Running unittests src/lib.rs (target/debug/deps/autospec_core-1a2b3c4d5e6f)

running 3 tests
test sizing::test_layer_within_cap ... ok
test conversion::test_patch_applies ... FAILED
test prose::test_no_live_directive ... FAILED

failures:

---- conversion::test_patch_applies stdout ----
thread 'conversion::test_patch_applies' panicked at src/conversion.rs:88:9:
patch does not apply

---- prose::test_no_live_directive stdout ----
assertion failed: body.is_empty()

failures:
    conversion::test_patch_applies
    prose::test_no_live_directive

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"
    .to_string()
}

/// The log truncated after the progress stream: the `failures:` block never
/// reached the file (a `head -N` on the log, the #4167 family).
fn truncated_progress_only_log() -> String {
    "     Running unittests src/lib.rs (target/debug/deps/autospec_core-1a2b3c4d5e6f)

running 2 tests
test conversion::test_patch_applies ... FAILED
test prose::test_no_live_directive ... FAILED

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"
    .to_string()
}

/// The summary declares three failures, the name list accounts for one.
fn shortfall_log() -> String {
    "     Running unittests src/lib.rs (target/debug/deps/autospec_core-1a2b3c4d5e6f)

running 4 tests
test sizing::test_layer_within_cap ... ok

failures:
    conversion::test_patch_applies

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"
    .to_string()
}

/// Two targets, failures in the second one only.
fn multi_target_log() -> String {
    "     Running unittests src/lib.rs (target/debug/deps/autospec_core-1a2b3c4d5e6f)

running 1 test
test sizing::test_layer_within_cap ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/conversion.rs (target/debug/deps/conversion-9f8e7d6c)

running 2 tests

failures:
    test_patch_applies
    test_held_patch_reports

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

   Doc-tests autospec_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
"
    .to_string()
}

fn all_green_log() -> String {
    "     Running unittests src/lib.rs (target/debug/deps/autospec_core-1a2b3c4d5e6f)

running 2 tests
test sizing::test_layer_within_cap ... ok
test prose::test_no_live_directive ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
"
    .to_string()
}

// --- Invariant 1: names come from the block, never the stream ---

#[test]
fn quiet_run_names_failures_the_progress_stream_cannot() {
    let log = quiet_run_log();
    assert!(
        progress_stream_names(&log).is_empty(),
        "a quiet run prints no `test NAME ... FAILED` lines; the progress channel stays empty"
    );
    assert_eq!(
        authoritative_names(&log),
        vec![
            "conversion::test_patch_applies".to_string(),
            "sizing::test_layer_within_cap".to_string(),
        ]
    );
}

#[test]
fn quiet_run_is_attributed_completely_from_the_name_list() {
    let report = attribute(&quiet_run_log());
    assert!(report.harness_ran);
    assert_eq!(report.declared, 2);
    assert_eq!(report.named, 2);
    assert_eq!(report.shortfall(), 0);
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    assert!(report.complete());
    assert!(report.line().contains("from failures-name-list"));
}

#[test]
fn verbose_run_agrees_across_channels_and_is_complete() {
    let report = attribute(&verbose_run_log());
    assert_eq!(report.declared, 2);
    assert_eq!(report.named, 2);
    assert!(report.complete(), "{:?}", report.findings());
    let target = &report.targets[0];
    assert_eq!(target.target, "autospec_core");
    assert_eq!(target.name_list, 2);
    assert_eq!(target.progress, 2);
    assert_eq!(target.stdout_block, 2);
    assert_eq!(
        authoritative_names(&verbose_run_log()).len(),
        2,
        "the three channels must not be concatenated"
    );
}

#[test]
fn stdout_detail_markers_alone_are_authoritative() {
    // Detail markers are still the harness naming a test it captured output for.
    let log = "running 2 tests
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

failures:

---- conversion::test_patch_applies stdout ----
patch does not apply

---- sizing::test_layer_within_cap stderr ----
warn: leak
";
    let names = authoritative_names(log);
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"sizing::test_layer_within_cap".to_string()));
}

// --- Invariant 2: declared and named must agree, or it is a finding ---

#[test]
fn progress_only_attribution_is_a_shortfall_not_a_match() {
    let report = attribute(&truncated_progress_only_log());
    assert_eq!(report.declared, 2);
    assert_eq!(report.named, 0, "progress names are not evidence");
    assert_eq!(report.shortfall(), 2);
    assert!(matches!(
        report.targets[0].verdict(),
        TargetVerdict::Shortfall {
            declared: 2,
            named: 0
        }
    ));
    let findings = report.findings();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].starts_with("FAILURE_ATTRIBUTION:"));
    assert!(findings[0].contains("must not be read as tolerated"));
    assert!(
        !report.complete(),
        "the pre-fix extractor called this run clean"
    );
}

#[test]
fn shortfall_names_the_gap_between_declared_and_named() {
    let report = attribute(&shortfall_log());
    assert_eq!(report.declared, 3);
    assert_eq!(report.named, 1);
    assert_eq!(report.shortfall(), 2);
    let findings = report.findings();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0].contains("declares 3 failed test(s) but its `failures:` block names 1"),
        "{}",
        findings[0]
    );
    assert!(findings[0].contains("2 unnamed"), "{}", findings[0]);
    assert!(!report.complete());
}

#[test]
fn a_duplicate_name_is_not_counted_twice() {
    // The summary declares 2, the block names one test twice: deduplication
    // keeps the shortfall visible instead of inflating the named side.
    let log = "running 2 tests

failures:
    conversion::test_patch_applies
    conversion::test_patch_applies

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";
    let report = attribute(log);
    assert_eq!(report.named, 1, "distinct names only");
    assert_eq!(report.shortfall(), 1);
    assert!(!report.complete());
}

// --- Invariant 3: a surplus is a finding too ---

#[test]
fn surplus_names_is_a_finding() {
    let log = "running 3 tests

failures:
    a
    b
    c

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";
    let report = attribute(log);
    assert_eq!(report.declared, 1);
    assert_eq!(report.named, 3);
    assert!(matches!(
        report.targets[0].verdict(),
        TargetVerdict::Surplus {
            declared: 1,
            named: 3
        }
    ));
    assert!(report.findings()[0].starts_with("FAILURE_ATTRIBUTION:"));
    assert!(!report.complete());
    assert_eq!(
        report.shortfall(),
        0,
        "a surplus is not a negative shortfall"
    );
}

// --- Invariant 4: progress-only evidence is called out by name ---

#[test]
fn source_warning_explains_why_progress_names_do_not_count() {
    let report = attribute(&truncated_progress_only_log());
    let warnings = report.source_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].starts_with("WARN:"));
    assert!(warnings[0].contains("progress stream only"));
    assert!(warnings[0].contains("not verified"));
    // The verbose run has progress lines too, but the block was read.
    assert!(attribute(&verbose_run_log()).source_warnings().is_empty());
}

// --- Target boundaries ---

#[test]
fn each_target_is_checked_against_its_own_summary() {
    let report = attribute(&multi_target_log());
    assert_eq!(report.targets.len(), 3, "{:?}", report.targets);
    let labels: Vec<&str> = report.targets.iter().map(|t| t.target.as_str()).collect();
    assert_eq!(
        labels,
        vec!["autospec_core", "conversion", "Doc-tests autospec_core"]
    );
    assert_eq!(report.declared, 2);
    assert_eq!(report.named, 2);
    assert!(report.complete(), "{:?}", report.findings());
    assert_eq!(report.targets[1].authoritative().len(), 2);
    assert!(report.targets[0].authoritative().is_empty());
}

#[test]
fn a_target_without_a_summary_line_is_never_verified() {
    let log = "     Running unittests src/lib.rs (target/debug/deps/autospec_core-1a2b3c4d5e6f)

running 2 tests

failures:
    conversion::test_patch_applies
";
    let report = attribute(log);
    assert!(
        !report.harness_ran,
        "no summary line means the harness verdict was never reported"
    );
    assert!(matches!(
        report.targets[0].verdict(),
        TargetVerdict::NoResultLine { named: 1 }
    ));
    assert!(!report.complete());
    assert!(report.findings()[0].contains("no `test result:` line"));
}

#[test]
fn an_empty_log_is_not_an_attributed_run() {
    let report = attribute("");
    assert!(!report.harness_ran);
    assert_eq!(report.declared, 0);
    assert_eq!(report.named, 0);
    assert!(!report.complete());
    assert!(report.line().contains("nothing is attributed"));
}

#[test]
fn all_green_run_has_nothing_to_attribute_and_is_complete() {
    let report = attribute(&all_green_log());
    assert!(report.harness_ran);
    assert_eq!(report.declared, 0);
    assert_eq!(report.named, 0);
    assert!(report.complete());
    assert!(report.findings().is_empty());
    assert!(authoritative_names(&all_green_log()).is_empty());
}
