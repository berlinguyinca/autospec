use autospec_core::autonomous::config::AutonomousConfig;

#[test]
fn absent_section_disables_board_ingestion() {
    let cfg = AutonomousConfig::parse("main_health:\n  branch: main\n").expect("parse");
    assert!(cfg.project_board.url.is_none());
    assert!(!cfg.project_board.write_back);
}

#[test]
fn configured_board_defaults_write_back_on() {
    let cfg = AutonomousConfig::parse(
        "project_board:\n  url: https://github.com/orgs/InferWeave/projects/2\n  repo_allowlist: [\"InferWeave/*\"]\n",
    )
    .expect("parse");
    assert_eq!(
        cfg.project_board.url.as_deref(),
        Some("https://github.com/orgs/InferWeave/projects/2")
    );
    assert!(cfg.project_board.write_back);
    assert_eq!(cfg.project_board.repo_allowlist, vec!["InferWeave/*"]);
}

#[test]
fn write_back_can_be_disabled_explicitly() {
    let cfg = AutonomousConfig::parse(
        "project_board:\n  url: https://github.com/orgs/InferWeave/projects/2\n  repo_allowlist: [\"InferWeave/*\"]\n  write_back: false\n",
    )
    .expect("parse");
    assert!(!cfg.project_board.write_back);
}

#[test]
fn configured_board_without_allowlist_is_rejected() {
    let err = AutonomousConfig::parse(
        "project_board:\n  url: https://github.com/orgs/InferWeave/projects/2\n",
    )
    .expect_err("empty allowlist must be rejected");
    assert!(err.contains("repo_allowlist"), "unexpected error: {err}");
}

#[test]
fn control_issue_is_optional_and_parsed_when_present() {
    let cfg = AutonomousConfig::parse(
        "project_board:\n  url: https://github.com/orgs/InferWeave/projects/2\n  repo_allowlist: [\"InferWeave/*\"]\n  control_issue: InferWeave/inferweave-workbench#1\n",
    )
    .expect("parse");
    assert_eq!(
        cfg.project_board.control_issue.as_deref(),
        Some("InferWeave/inferweave-workbench#1")
    );
}

#[test]
fn state_candidate_lists_default_when_absent() {
    let cfg = AutonomousConfig::parse("main_health:\n  branch: main\n").expect("parse");
    assert_eq!(
        cfg.project_board.state_field_candidates,
        vec!["AutoSpec state".to_string(), "Delivery status".to_string()]
    );
    assert_eq!(
        cfg.project_board.state_option_candidates.get("Implementation"),
        Some(&vec![
            "Implementation".to_string(),
            "In progress".to_string()
        ])
    );
    assert_eq!(
        cfg.project_board.state_option_candidates.get("Blocked"),
        Some(&vec!["Blocked".to_string()])
    );
}

#[test]
fn explicitly_empty_candidate_list_is_rejected() {
    let err = AutonomousConfig::parse(
        "project_board:\n  url: https://github.com/orgs/InferWeave/projects/2\n  repo_allowlist: [\"InferWeave/*\"]\n  state_field_candidates: []\n",
    )
    .expect_err("empty candidate list must be rejected");
    assert!(
        err.contains("state_field_candidates"),
        "unexpected error: {err}"
    );
}
