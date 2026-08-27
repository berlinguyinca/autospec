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

// ── New operator-facing fields (config-coherence fix) ─────────────────────
//
// Every field below previously existed only as an AUTOSPEC_PROJECT_BOARD_*
// (or AUTOSPEC_SPEND_SCOPE) shell env var with no YAML key at all, or (for
// state_field_candidates/state_option_candidates) as a YAML key with zero
// consumers. These tests pin: (a) the default equals the shell's former
// hardcoded literal, and (b) an operator-set value parses and is rejected
// when malformed.

#[test]
fn new_fields_default_to_the_shells_former_hardcoded_literals_when_absent() {
    let cfg = AutonomousConfig::parse("main_health:\n  branch: main\n").expect("parse");
    let board = cfg.project_board;
    assert_eq!(board.ttl_seconds, 300);
    assert_eq!(board.label_map, None);
    assert_eq!(board.spend_scope, None);
    assert_eq!(board.item_limit, 500);
    assert_eq!(board.max_parallel_repos, 2);
    assert_eq!(
        board.dep_field_candidates,
        vec!["Dependencies".to_string(), "Depends on".to_string()]
    );
    assert_eq!(
        board.dep_markers,
        vec!["Blocked by".to_string(), "Depends on".to_string()]
    );
}

#[test]
fn ttl_parses_from_yaml() {
    let cfg = AutonomousConfig::parse("project_board:\n  ttl: 900\n").expect("parse");
    assert_eq!(cfg.project_board.ttl_seconds, 900);
}

#[test]
fn ttl_rejects_non_integer() {
    let err = AutonomousConfig::parse("project_board:\n  ttl: soon\n")
        .expect_err("non-integer ttl must be rejected");
    assert!(err.contains("ttl"), "unexpected error: {err}");
}

#[test]
fn label_map_parses_from_yaml() {
    let cfg =
        AutonomousConfig::parse("project_board:\n  label_map: bug=defect,chore=maintenance\n")
            .expect("parse");
    assert_eq!(
        cfg.project_board.label_map.as_deref(),
        Some("bug=defect,chore=maintenance")
    );
}

#[test]
fn item_limit_parses_from_yaml() {
    let cfg = AutonomousConfig::parse("project_board:\n  item_limit: 1000\n").expect("parse");
    assert_eq!(cfg.project_board.item_limit, 1000);
}

#[test]
fn item_limit_rejects_negative_value() {
    let err = AutonomousConfig::parse("project_board:\n  item_limit: -5\n")
        .expect_err("negative item_limit must be rejected");
    assert!(err.contains("item_limit"), "unexpected error: {err}");
}

#[test]
fn max_parallel_repos_parses_from_yaml() {
    let cfg = AutonomousConfig::parse("project_board:\n  max_parallel_repos: 4\n").expect("parse");
    assert_eq!(cfg.project_board.max_parallel_repos, 4);
}

#[test]
fn dep_field_candidates_parses_from_yaml() {
    let cfg = AutonomousConfig::parse(
        "project_board:\n  dep_field_candidates: [\"Blocks\", \"Waiting on\"]\n",
    )
    .expect("parse");
    assert_eq!(
        cfg.project_board.dep_field_candidates,
        vec!["Blocks".to_string(), "Waiting on".to_string()]
    );
}

#[test]
fn dep_field_candidates_rejects_explicit_empty_list() {
    let err = AutonomousConfig::parse("project_board:\n  dep_field_candidates: []\n")
        .expect_err("empty dep_field_candidates must be rejected");
    assert!(
        err.contains("dep_field_candidates"),
        "unexpected error: {err}"
    );
}

#[test]
fn dep_markers_parses_from_yaml() {
    let cfg = AutonomousConfig::parse("project_board:\n  dep_markers: [\"Waiting on\"]\n")
        .expect("parse");
    assert_eq!(cfg.project_board.dep_markers, vec!["Waiting on".to_string()]);
}

#[test]
fn dep_markers_rejects_explicit_empty_list() {
    let err = AutonomousConfig::parse("project_board:\n  dep_markers: []\n")
        .expect_err("empty dep_markers must be rejected");
    assert!(err.contains("dep_markers"), "unexpected error: {err}");
}

#[test]
fn spend_scope_parses_from_yaml() {
    let cfg =
        AutonomousConfig::parse("project_board:\n  spend_scope: board-inferweave-2\n").expect("parse");
    assert_eq!(
        cfg.project_board.spend_scope.as_deref(),
        Some("board-inferweave-2")
    );
}

#[test]
fn spend_scope_rejects_path_traversal() {
    let err = AutonomousConfig::parse("project_board:\n  spend_scope: ../../etc\n")
        .expect_err("path traversal spend_scope must be rejected");
    assert!(err.contains("spend_scope"), "unexpected error: {err}");
}

#[test]
fn spend_scope_rejects_embedded_slash() {
    let err = AutonomousConfig::parse("project_board:\n  spend_scope: a/b\n")
        .expect_err("embedded slash spend_scope must be rejected");
    assert!(err.contains("spend_scope"), "unexpected error: {err}");
}

#[test]
fn spend_scope_rejects_dot_and_dotdot() {
    for value in ["project_board:\n  spend_scope: \".\"\n", "project_board:\n  spend_scope: \"..\"\n"] {
        let err = AutonomousConfig::parse(value)
            .expect_err("'.' and '..' spend_scope must be rejected");
        assert!(err.contains("spend_scope"), "unexpected error: {err}");
    }
}

#[test]
fn spend_scope_rejects_empty() {
    let err = AutonomousConfig::parse("project_board:\n  spend_scope: \"\"\n")
        .expect_err("empty spend_scope must be rejected");
    // scalar() rejects an empty value before validate_spend_scope ever
    // runs — either message is acceptable, but it must fail.
    let _ = err;
}

#[test]
fn spend_scope_rejects_leading_dash() {
    let err = AutonomousConfig::parse("project_board:\n  spend_scope: \"-oops\"\n")
        .expect_err("leading-dash spend_scope must be rejected");
    assert!(err.contains("spend_scope"), "unexpected error: {err}");
}

#[test]
fn spend_scope_rejects_overlong_value() {
    let long_value = "a".repeat(201);
    let source = format!("project_board:\n  spend_scope: {long_value}\n");
    let err = AutonomousConfig::parse(&source).expect_err("overlong spend_scope must be rejected");
    assert!(err.contains("spend_scope"), "unexpected error: {err}");
}

#[test]
fn spend_scope_accepts_max_length_value() {
    let max_value = "a".repeat(200);
    let source = format!("project_board:\n  spend_scope: {max_value}\n");
    let cfg = AutonomousConfig::parse(&source).expect("200-char spend_scope must be accepted");
    assert_eq!(cfg.project_board.spend_scope.as_deref(), Some(max_value.as_str()));
}
