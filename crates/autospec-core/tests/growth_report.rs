use autospec_core::growth::GrowthReport;

#[test]
fn growth_report_is_local_only_and_counts_readiness() {
    let report = GrowthReport::new(
        true,
        true,
        true,
        true,
        true,
        vec!["marketing/launch-post-github.md".to_string()],
    );

    let json = report.to_json();

    assert!(report.local_only);
    assert_eq!(report.ready_count(), 5);
    assert!(json.contains("\"command\":\"growth-report\""));
    assert!(json.contains("\"local_only\":true"));
}
