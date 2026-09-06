use autospec_core::validation::{
    require_tool, CheckResult, Measurement, ToolCommand, ValidationExecutionReport,
    ValidationObservation, ValidationReport, ValidationStatus,
};

/// #3535's first row: `gofmt -l … | wc -l` reported `0` because `gofmt` was absent.
#[test]
fn a_missing_tool_yields_unknown_rather_than_a_pass() {
    let command = ToolCommand::new("autospec-tool-that-does-not-exist", ["--check"])
        .expect("a direct argv builds");

    let result = command.execute_in("check_absent_tool", true, std::path::Path::new("."));

    assert!(result.is_unmeasured(), "{result:?}");
    assert!(
        !result.is_success(),
        "an absent tool must never read as a pass"
    );
    assert!(
        !result.is_failure(),
        "and must not be filed as a measured failure either"
    );
    let reason = result.unmeasured.as_deref().expect("a reason is recorded");
    assert!(
        reason.contains("autospec-tool-that-does-not-exist"),
        "the missing tool must be named: {reason}"
    );
}

#[test]
fn an_unmeasured_required_check_makes_the_run_unknown_not_passed() {
    let report = ValidationExecutionReport::new(vec![
        CheckResult::completed("check_that_ran", true, 0, 1, 1, 0, 0, "digest"),
        CheckResult::unmeasured("check_absent_tool", true, "gofmt is not on PATH"),
    ]);

    let aggregate = report.aggregate().expect("results aggregate");

    assert_eq!(aggregate.status, ValidationStatus::Unknown);
    assert!(!aggregate.status.is_passed());
    assert_eq!(aggregate.passed, 1);
    assert_eq!(
        aggregate.failed, 0,
        "unknown must not be counted as a failure"
    );
    assert_eq!(aggregate.unknown, 1);
    assert_eq!(aggregate.required_unknown, 1);
    assert_eq!(
        aggregate.total,
        aggregate.passed + aggregate.failed + aggregate.unknown,
        "every check lands in exactly one bucket"
    );
}

#[test]
fn a_measured_failure_outranks_an_unmeasured_check() {
    let report = ValidationExecutionReport::new(vec![
        CheckResult::completed("check_broken", true, 1, 1, 1, 0, 12, "digest"),
        CheckResult::unmeasured("check_absent_tool", true, "bats is not on PATH"),
    ]);

    let aggregate = report.aggregate().expect("results aggregate");

    assert_eq!(aggregate.status, ValidationStatus::Failed);
    assert_eq!(aggregate.required_failed, 1);
    assert_eq!(aggregate.required_unknown, 1);
}

/// A fabricated zero is indistinguishable from a measured one — unless the serialised
/// form keeps them apart.
#[test]
fn unknown_serialises_distinctly_from_a_measured_zero() {
    let measured = CheckResult::completed("check_clean", true, 0, 0, 1, 0, 0, "digest");
    let unknown = CheckResult::unmeasured("check_absent_tool", true, "gofmt is not on PATH");

    assert!(measured.to_json().contains("\"unmeasured\":null"));
    assert!(measured.to_json().contains("\"exit_code\":0"));
    assert!(unknown
        .to_json()
        .contains("\"unmeasured\":\"gofmt is not on PATH\""));
    assert!(
        unknown.to_json().contains("\"exit_code\":null"),
        "an unmeasured check has no exit status to report: {}",
        unknown.to_json()
    );
}

#[test]
fn a_captured_report_round_trips_its_unknowns_and_keeps_older_reports_readable() {
    let report = ValidationReport::new(vec![
        ValidationObservation::new("check_that_ran", true, 0),
        ValidationObservation::unmeasured("check_absent_tool", true, "python3 is not on PATH"),
    ]);
    let document = report.to_json().expect("report serialises");

    let parsed = ValidationReport::from_json(&document).expect("report round-trips");
    assert_eq!(parsed, report);
    assert_eq!(
        parsed.aggregate().expect("aggregate").status,
        ValidationStatus::Unknown
    );

    // A report captured before this field existed carries no `unmeasured` key, and must
    // keep parsing as measured rather than turning every old observation into an unknown.
    let legacy = "{\"schema\":1,\"results\":[{\"name\":\"check_that_ran\",\"required\":true,\"exit_code\":0}]}";
    let legacy = ValidationReport::from_json(legacy).expect("a pre-#3535 report still parses");
    assert_eq!(
        legacy.aggregate().expect("aggregate").status,
        ValidationStatus::Passed
    );
}

/// #3535's fourth row: `grep -c '^--- PASS'` over `go test` without `-v` matched nothing,
/// and the harness read that as a result rather than as the absence of one.
#[test]
fn a_command_with_no_parseable_output_measures_unknown_not_zero() {
    let measurement =
        Measurement::records_parsed("go test", Some(0), "ok  \tpkg\t0.10s\n", |line| {
            line.starts_with("--- PASS")
        });

    assert!(measurement.is_unknown());
    assert_eq!(measurement.count(), None);
    assert_eq!(measurement.to_json(), "null");
    assert_eq!(measurement.as_display(), "unknown");
}

#[test]
fn a_tool_that_ran_and_flagged_nothing_measures_a_genuine_zero() {
    let measurement = Measurement::problems_reported("gofmt", Some(0), "", |_| true);

    assert_eq!(measurement.count(), Some(0));
    assert_eq!(measurement.to_json(), "0");
    assert!(!measurement.is_unknown());
}

#[test]
fn a_tool_that_never_ran_measures_nothing_even_with_empty_output() {
    let measurement = Measurement::problems_reported("gofmt", None, "", |_| true);

    assert!(measurement.is_unknown());
    assert!(measurement
        .reason()
        .is_some_and(|reason| reason.contains("gofmt")));
}

#[test]
fn tool_presence_is_asserted_before_measuring_and_names_what_is_missing() {
    let error = require_tool("autospec-tool-that-does-not-exist")
        .expect_err("an absent tool must not resolve");

    assert!(
        error.contains("autospec-tool-that-does-not-exist"),
        "{error}"
    );
    require_tool("git").expect("git is required by the test harness itself");
}
