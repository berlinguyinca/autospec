use autospec_core::validation::{ValidationObservation, ValidationReport, ValidationStatus};

#[test]
fn validation_results_aggregate_required_and_optional_outcomes_without_execution() {
    let report = ValidationReport::new(vec![
        ValidationObservation::new("legacy-shell-fast", true, 0),
        ValidationObservation::new("advisory-style", false, 1),
    ]);

    let aggregate = report.aggregate().expect("observations aggregate");

    assert_eq!(aggregate.status, ValidationStatus::Passed);
    assert_eq!(aggregate.total, 2);
    assert_eq!(aggregate.passed, 1);
    assert_eq!(aggregate.failed, 1);
    assert_eq!(aggregate.required_failed, 0);
    assert_eq!(aggregate.optional_failed, 1);
    // Schema 2 adds `unknown` and `required_unknown`: a report that cannot distinguish an
    // unmeasured check from a measured zero cannot be trusted to mean what it says.
    assert_eq!(aggregate.unknown, 0);
    assert_eq!(aggregate.required_unknown, 0);
    assert_eq!(
        aggregate.to_json(),
        "{\"schema\":2,\"status\":\"passed\",\"total\":2,\"passed\":1,\"failed\":1,\"unknown\":0,\"required_failed\":0,\"required_unknown\":0,\"optional_failed\":1}"
    );
}

#[test]
fn validation_results_reject_duplicate_names_and_preserve_signed_exit_codes() {
    let report = ValidationReport::from_json(
        "{\"schema\":1,\"results\":[{\"name\":\"legacy-shell\",\"required\":true,\"exit_code\":-1}]}",
    )
    .expect("signed exit code parses");

    assert_eq!(report.aggregate().unwrap().status, ValidationStatus::Failed);
    assert!(ValidationReport::from_json(
        "{\"schema\":1,\"results\":[{\"name\":\"same\",\"required\":true,\"exit_code\":0},{\"name\":\"same\",\"required\":false,\"exit_code\":0}]}"
    )
    .is_err());
    assert!(ValidationReport::from_json("{\"schema\":1,\"results\":[]}").is_err());
}
