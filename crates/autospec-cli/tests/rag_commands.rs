//! `autospec rag` command surface.

use std::process::{Command, Output};

fn autospec(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
        .args(args)
        .output()
        .expect("the autospec binary runs")
}

fn stdout_of(args: &[&str]) -> String {
    let output = autospec(args);
    assert!(
        output.status.success(),
        "autospec {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

#[test]
fn rag_appears_in_the_top_level_help() {
    let help = stdout_of(&["--help"]);

    assert!(help.contains("rag"), "{help}");
}

#[test]
fn rag_help_lists_every_subcommand() {
    let help = stdout_of(&["rag", "--help"]);

    for subcommand in ["config", "policy", "sources", "route"] {
        assert!(help.contains(subcommand), "missing {subcommand} in {help}");
    }
}

#[test]
fn rag_config_renders_the_specification_defaults() {
    let yaml = stdout_of(&["rag", "config"]);

    assert!(yaml.starts_with("agentic_rag:"), "{yaml}");
    assert!(yaml.contains("max_iterations: 8"), "{yaml}");
    assert!(yaml.contains("max_context_tokens: 40000"), "{yaml}");
    assert!(yaml.contains("web: policy"), "{yaml}");
    assert!(yaml.contains("revision_aware: true"), "{yaml}");
}

#[test]
fn rag_config_json_is_parseable_and_names_every_role() {
    let json = stdout_of(&["rag", "config", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    assert_eq!(value["enabled"], true);
    assert_eq!(value["default"]["max_iterations"], 8);
    assert_eq!(value["roles"].as_array().expect("roles array").len(), 8);
}

#[test]
fn rag_config_applies_an_override_before_reporting() {
    let yaml = stdout_of(&["rag", "config", "--set", "default.max_iterations=3"]);

    assert!(yaml.contains("max_iterations: 3"), "{yaml}");
}

#[test]
fn rag_config_rejects_a_revision_blind_cache() {
    let output = autospec(&["rag", "config", "--set", "cache.revision_aware=false"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("revision_aware"), "{stderr}");
}

#[test]
fn rag_config_rejects_an_unknown_override_key() {
    let output = autospec(&["rag", "config", "--set", "default.max_iteratons=3"]);

    assert!(!output.status.success(), "a typo must not be ignored");
}

#[test]
fn rag_policy_reports_one_role_when_asked() {
    let report = stdout_of(&["rag", "policy", "--role", "implementation"]);

    assert!(report.contains("worktree_local"), "{report}");
    assert!(report.contains("max_context_tokens:  30000"), "{report}");
    assert!(!report.contains("architecture_first"), "{report}");
}

#[test]
fn rag_policy_reports_every_role_by_default() {
    let report = stdout_of(&["rag", "policy"]);

    for role in ["spec", "planner", "implementation", "reviewer", "test"] {
        assert!(report.contains(role), "missing {role} in {report}");
    }
}

#[test]
fn rag_policy_rejects_an_unknown_role() {
    let output = autospec(&["rag", "policy", "--role", "architect"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown agent role"), "{stderr}");
}

#[test]
fn rag_sources_shows_the_web_gate_opening_only_for_an_external_task() {
    let closed = stdout_of(&["rag", "sources", "--role", "spec", "--json"]);
    let open = stdout_of(&["rag", "sources", "--role", "spec", "--external", "--json"]);

    let closed: serde_json::Value = serde_json::from_str(&closed).expect("valid JSON");
    let open: serde_json::Value = serde_json::from_str(&open).expect("valid JSON");
    let web_allowed = |value: &serde_json::Value| {
        value["sources"]
            .as_array()
            .expect("sources array")
            .iter()
            .find(|entry| entry["source"] == "web")
            .expect("web is listed")["allowed"]
            .as_bool()
            .expect("allowed is a boolean")
    };

    assert!(!web_allowed(&closed));
    assert!(web_allowed(&open));
}

#[test]
fn rag_route_prefers_capacity_over_speed() {
    // Specification section 24's worked example.
    let report = stdout_of(&[
        "rag",
        "route",
        "--task",
        "architecture_synthesis",
        "--context",
        "60000",
        "--node",
        "A:strong:20000:100:2",
        "--node",
        "B:strong:100000:10:2",
    ]);

    assert!(report.contains("selected:                B"), "{report}");
    assert!(report.contains("rejected A"), "{report}");
}

#[test]
fn rag_route_json_names_the_required_context_and_the_selection() {
    let json = stdout_of(&[
        "rag",
        "route",
        "--task",
        "query_rewriting",
        "--context",
        "4000",
        "--node",
        "fast:small:8000:100:1",
        "--json",
    ]);
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    assert_eq!(value["task"], "query_rewriting");
    assert_eq!(value["required_context_tokens"], 4400);
    assert_eq!(value["selected"], "fast");
}

#[test]
fn rag_route_reports_when_no_node_is_eligible() {
    let report = stdout_of(&[
        "rag",
        "route",
        "--task",
        "implementation_plan",
        "--context",
        "60000",
        "--node",
        "small:small:100000:100:4",
    ]);

    assert!(report.contains("none eligible"), "{report}");
    assert!(report.contains("reasoning class"), "{report}");
}

#[test]
fn rag_route_rejects_a_malformed_node_specification() {
    let output = autospec(&["rag", "route", "--node", "A:strong:1000"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--node expects"), "{stderr}");
}

#[test]
fn an_unknown_rag_subcommand_is_an_error() {
    let output = autospec(&["rag", "frobnicate"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown autospec rag subcommand"), "{stderr}");
}
