use autospec_core::incident::{parse_agent_failure_event, IncidentIssueProjection};

fn event() -> String {
    r#"{
      "type":"agent_failure",
      "schema_version":1,
      "incident_id":"inc_1",
      "run_id":"run_1",
      "repo":"owner/repo",
      "revision":"abc123",
      "worktree":"/tmp/worktree",
      "agent_role":"executor",
      "failure_class":"test_failure",
      "exit_code":1,
      "summary":"test failed",
      "artifact_refs":["artifact://failure/log"],
      "reproduction":"npm test",
      "command":"npm test",
      "root_cause":"AssertionError at src/lib.ts:4",
      "classification":"product_defect",
      "risk":"medium",
      "created_at":"2026-10-24T00:00:00Z",
      "fingerprint":"deadbeefdeadbeef"
    }"#
    .to_string()
}

#[test]
fn parses_valid_agent_failure_event_and_projects_dry_run_issue() {
    let parsed = parse_agent_failure_event(&event()).expect("valid event");
    let projection = IncidentIssueProjection::from_event(&parsed);

    assert_eq!(projection.title, "[Pi incident] owner/repo: test_failure");
    assert!(projection
        .body
        .contains("<!-- pi-incident:fingerprint=deadbeefdeadbeef -->"));
    assert!(projection.body.contains("artifact://failure/log"));
    assert_eq!(
        projection.labels,
        vec!["auto-implement", "origin:pi-incident"]
    );
    assert_eq!(projection.debugger_dispatch.role, "debugger");
    assert_eq!(projection.debugger_dispatch.dry_run, true);
}

#[test]
fn rejects_missing_required_fields_and_unknown_event_type() {
    let missing = event().replace("\"summary\":\"test failed\",", "");
    assert!(parse_agent_failure_event(&missing).is_err());

    let unknown = event().replace("agent_failure", "agent_success");
    assert!(parse_agent_failure_event(&unknown).is_err());
}

#[test]
fn rejects_unbounded_summary_and_artifact_references() {
    let oversized = event().replace("test failed", &"x".repeat(1025));
    assert!(parse_agent_failure_event(&oversized).is_err());

    let oversized_ref = event().replace(
        "artifact://failure/log",
        &format!("artifact://{}", "x".repeat(2048)),
    );
    assert!(parse_agent_failure_event(&oversized_ref).is_err());
}

#[test]
fn projection_has_stable_marker_and_debugger_prompt() {
    let parsed = parse_agent_failure_event(&event()).expect("valid event");
    let projection = IncidentIssueProjection::from_event(&parsed);

    assert_eq!(
        projection.marker(),
        "<!-- pi-incident:fingerprint=deadbeefdeadbeef -->"
    );
    assert!(projection.debugger_dispatch.prompt.contains("inc_1"));
    assert!(projection
        .debugger_dispatch
        .prompt
        .contains("artifact://failure/log"));
}
