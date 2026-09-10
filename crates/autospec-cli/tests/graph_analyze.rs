//! `autospec graph analyze` CLI contract.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! — §19 (DAG analyzer), §22 (AS-DAG-004 fan-in threshold), §24 (execution
//! waves), §25 (planner output summary), §38 (planning telemetry).
//!
//! Real temp-file inputs, no mocks: every test writes a real JSON file and
//! invokes the built `autospec` binary, asserting exit codes and output.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_INPUT: AtomicUsize = AtomicUsize::new(0);

fn analyze(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
        .arg("graph")
        .arg("analyze")
        .args(args)
        .output()
        .expect("spawn autospec binary")
}

/// Run `autospec graph analyze` with the given string arguments.
fn analyze_with(args: Vec<String>) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
        .arg("graph")
        .arg("analyze")
        .args(&args)
        .output()
        .expect("spawn autospec binary")
}

/// `--input <path>` plus any extra `--flag value` pairs.
fn input_args_with(path: &Path, extra: &[&str]) -> Vec<String> {
    let mut args = input_args(path);
    args.extend(extra.iter().map(|arg| arg.to_string()));
    args
}

/// Write `source` to a fresh real temp file and return its path.
fn write_input(name: &str, source: &str) -> PathBuf {
    let index = NEXT_INPUT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "autospec-graph-analyze-{}-{index}-{name}.json",
        std::process::id()
    ));
    fs::write(&path, source).expect("write input file");
    path
}

fn input_args(path: &Path) -> Vec<String> {
    vec!["--input".to_string(), path.to_string_lossy().into_owned()]
}

fn issue_json(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": format!("Issue {id}"),
        "ownership": {
            "exclusive": [],
            "shared_read": [],
            "shared_write": []
        },
        "concurrency": {
            "parallel_safe": true,
            "conflict_domains": []
        }
    })
}

fn issue_json_with_shared_write(id: &str, path: &str) -> serde_json::Value {
    let mut issue = issue_json(id);
    issue["ownership"]["shared_write"] = serde_json::json!([{ "path": path, "symbols": [] }]);
    issue
}

fn edge_json(predecessor: &str, successor: &str) -> serde_json::Value {
    serde_json::json!({
        "predecessor": predecessor,
        "successor": successor,
        "reason": "consumes-new-interface",
        "artifact": null
    })
}

fn graph_json(issues: &[&str], edges: &[(&str, &str)]) -> serde_json::Value {
    serde_json::json!({
        "issues": issues.iter().map(|id| issue_json(id)).collect::<Vec<_>>(),
        "hard_edges": edges
            .iter()
            .map(|(pre, suc)| edge_json(pre, suc))
            .collect::<Vec<_>>()
    })
}

/// 5-node diamond: A, B roots; C waits on A; D on B; E on both.
fn diamond() -> serde_json::Value {
    graph_json(
        &["a", "b", "c", "d", "e"],
        &[("a", "c"), ("b", "d"), ("c", "e"), ("d", "e")],
    )
}

#[test]
fn help_exits_zero() {
    let output = analyze(&["--help"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.contains("--input"),
        "help must mention --input: {help}"
    );
    assert!(
        help.contains("--capacity"),
        "help must mention --capacity: {help}"
    );
    assert!(
        help.contains("--format"),
        "help must mention --format: {help}"
    );
}

#[test]
fn missing_subcommand_is_diagnostic() {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .arg("graph")
        .output()
        .expect("spawn autospec binary");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unknown_graph_subcommand_is_diagnostic() {
    let output = Command::new(env!("CARGO_BIN_EXE_autospec"))
        .arg("graph")
        .arg("frobnicate")
        .output()
        .expect("spawn autospec binary");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown"), "stderr: {stderr}");
}

#[test]
fn analyze_positional_arg_is_diagnostic() {
    let output = analyze(&["frobnicate"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown option"), "stderr: {stderr}");
}

#[test]
fn missing_input_flag_is_diagnostic() {
    let output = analyze(&[]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--input"), "stderr: {stderr}");
}

#[test]
fn malformed_json_is_diagnostic_without_panic() {
    let path = write_input("malformed", "{ this is not json");
    let output = analyze_with(input_args(&path));
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "untrusted input must not panic: {stderr}"
    );
}

#[test]
fn scalar_input_is_rejected() {
    let path = write_input("scalar", "42");
    let output = analyze_with(input_args(&path));
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "untrusted input must not panic: {stderr}"
    );
}

#[test]
fn unknown_format_is_rejected() {
    let path = write_input("diamond", &diamond().to_string());
    let output = analyze_with(input_args_with(&path, &["--format", "yaml"]));
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--format"), "stderr: {stderr}");
}

#[test]
fn non_numeric_capacity_is_rejected() {
    let path = write_input("diamond", &diamond().to_string());
    let output = analyze_with(input_args_with(&path, &["--capacity", "lots"]));
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--capacity"), "stderr: {stderr}");
}

#[test]
fn json_carries_section19_keys() {
    let path = write_input("diamond", &diamond().to_string());
    let output = analyze_with(input_args(&path));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    for key in [
        "issue_count",
        "hard_edge_count",
        "root_count",
        "leaf_count",
        "critical_path_length",
        "maximum_width",
        "initial_width",
        "average_wave_width",
        "serialization_ratio",
        "shared_write_hotspots",
        "high_fan_in_nodes",
        "high_fan_out_nodes",
        "estimated_fleet_saturation",
    ] {
        assert!(report.get(key).is_some(), "missing §19 key {key}");
    }
    let saturation = &report["estimated_fleet_saturation"];
    for key in ["capacity", "initial", "peak"] {
        assert!(
            saturation.get(key).is_some(),
            "missing saturation key {key}"
        );
    }
    assert_eq!(report["issue_count"], 5);
    assert_eq!(report["hard_edge_count"], 4);
    assert_eq!(report["root_count"], 2);
    assert_eq!(report["critical_path_length"], 3);
    assert_eq!(report["initial_width"], 2);
    assert_eq!(report["maximum_width"], 2);
}

#[test]
fn json_keys_appear_in_stable_section19_order() {
    let path = write_input("diamond", &diamond().to_string());
    let output = analyze_with(input_args(&path));
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_order = [
        "issue_count",
        "hard_edge_count",
        "root_count",
        "leaf_count",
        "critical_path_length",
        "maximum_width",
        "initial_width",
        "average_wave_width",
        "serialization_ratio",
        "shared_write_hotspots",
        "high_fan_in_nodes",
        "high_fan_out_nodes",
        "estimated_fleet_saturation",
    ];
    let mut last = 0usize;
    for key in expected_order {
        let needle = format!("\"{key}\"");
        let offset = stdout[last..]
            .find(&needle)
            .unwrap_or_else(|| panic!("key {key} not found after position {last}"));
        last = last + offset + needle.len();
    }
}

#[test]
fn telemetry_carries_the_twelve_section38_counters() {
    let path = write_input("diamond", &diamond().to_string());
    let output = analyze_with(input_args(&path));
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    let telemetry = &report["telemetry"];
    let expected = [
        "issue_count",
        "hard_edge_count",
        "initial_width",
        "maximum_width",
        "critical_path",
        "parallelization_score",
        "fleet_capacity",
        "initial_saturation",
        "shared_write_hotspots",
        "edges_removed_by_optimizer",
        "issues_split_by_optimizer",
        "issues_merged_by_optimizer",
    ];
    assert_eq!(
        telemetry.as_object().map(|map| map.len()),
        Some(expected.len()),
        "telemetry must carry exactly the 12 §38 counters"
    );
    for key in expected {
        assert!(telemetry.get(key).is_some(), "missing telemetry key {key}");
    }
    assert_eq!(telemetry["issue_count"], 5);
    assert_eq!(telemetry["critical_path"], 3);
    assert_eq!(telemetry["fleet_capacity"], 32, "default capacity is 32");
    assert_eq!(telemetry["edges_removed_by_optimizer"], 0);
    assert_eq!(telemetry["issues_split_by_optimizer"], 0);
    assert_eq!(telemetry["issues_merged_by_optimizer"], 0);
}

#[test]
fn capacity_flag_drives_saturation() {
    let path = write_input("diamond", &diamond().to_string());
    let output = analyze_with(input_args_with(&path, &["--capacity", "2"]));
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    let saturation = &report["estimated_fleet_saturation"];
    assert_eq!(saturation["capacity"], 2);
    // initial_width = 2, maximum_width = 2, capacity = 2 -> both saturations 1.0
    assert_eq!(saturation["initial"], 1.0);
    assert_eq!(saturation["peak"], 1.0);
}

#[test]
fn draft_array_input_is_accepted_with_no_edges() {
    let drafts = serde_json::json!([issue_json("a"), issue_json("b"), issue_json("c")]);
    let path = write_input("drafts", &drafts.to_string());
    let output = analyze_with(input_args(&path));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    assert_eq!(report["issue_count"], 3);
    assert_eq!(report["hard_edge_count"], 0);
    assert_eq!(report["initial_width"], 3);
    assert_eq!(report["critical_path_length"], 1);
}

#[test]
fn cyclic_input_exits_nonzero_and_prints_the_cycle_on_one_line() {
    let cyclic = graph_json(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("c", "a")]);
    let path = write_input("cyclic", &cyclic.to_string());
    let output = analyze_with(input_args(&path));
    assert_ne!(
        output.status.code(),
        None,
        "process must exit, not be killed"
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "cyclic input must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stderr_lines: Vec<&str> = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(
        stderr_lines.len(),
        1,
        "cycle must be printed in 1 line: {stderr_lines:?}"
    );
    assert!(
        stderr_lines[0].contains("a -> b -> c -> a"),
        "cycle path missing: {stderr_lines:?}"
    );
}

#[test]
fn self_loop_is_reported_as_a_cycle() {
    let self_loop = graph_json(&["a"], &[("a", "a")]);
    let path = write_input("self-loop", &self_loop.to_string());
    let output = analyze_with(input_args(&path));
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("a -> a"),
        "self loop must be reported: {stderr}"
    );
}

#[test]
fn shared_write_hotspot_is_counted() {
    let graph = serde_json::json!({
        "issues": [
            issue_json_with_shared_write("a", "crates/core/src/lib.rs"),
            issue_json_with_shared_write("b", "crates/core/src/lib.rs"),
            issue_json("c")
        ],
        "hard_edges": []
    });
    let path = write_input("hotspot", &graph.to_string());
    let output = analyze_with(input_args(&path));
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    assert_eq!(report["shared_write_hotspots"], 1);
}

#[test]
fn high_fan_in_nodes_use_the_section22_threshold() {
    // Six predecessors on "hub" (> 5, AS-DAG-004 default) and exactly five on
    // "edge" (not above the threshold) must separate the two.
    let mut issues: Vec<serde_json::Value> = vec![issue_json("hub"), issue_json("edge")];
    let mut edges: Vec<serde_json::Value> = Vec::new();
    for i in 0..6 {
        let id = format!("p{i}");
        issues.push(issue_json(&id));
        edges.push(edge_json(&id, "hub"));
    }
    for i in 0..5 {
        let id = format!("q{i}");
        issues.push(issue_json(&id));
        edges.push(edge_json(&id, "edge"));
    }
    let graph = serde_json::json!({ "issues": issues, "hard_edges": edges });
    let path = write_input("fan-in", &graph.to_string());
    let output = analyze_with(input_args(&path));
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    assert_eq!(report["high_fan_in_nodes"], serde_json::json!(["hub"]));
    assert_eq!(report["high_fan_out_nodes"], serde_json::json!([]));
}

#[test]
fn high_fan_out_nodes_are_reported() {
    let graph = graph_json(
        &["hub", "s1", "s2", "s3", "s4", "s5", "s6"],
        &[
            ("hub", "s1"),
            ("hub", "s2"),
            ("hub", "s3"),
            ("hub", "s4"),
            ("hub", "s5"),
            ("hub", "s6"),
        ],
    );
    let path = write_input("fan-out", &graph.to_string());
    let output = analyze_with(input_args(&path));
    assert_eq!(output.status.code(), Some(0));
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    assert_eq!(report["high_fan_out_nodes"], serde_json::json!(["hub"]));
}

#[test]
fn text_format_prints_wave_lines_and_parallelization_score() {
    let path = write_input("diamond", &diamond().to_string());
    let output = analyze_with(input_args_with(&path, &["--format", "text"]));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Wave 0"), "missing wave 0 line: {text}");
    assert!(text.contains("Wave 1"), "missing wave 1 line: {text}");
    assert!(text.contains("Wave 2"), "missing wave 2 line: {text}");
    assert!(
        text.contains("Parallelization score:"),
        "missing score line: {text}"
    );
    assert!(
        text.contains("/100"),
        "score must be rendered as N/100: {text}"
    );
}

#[test]
fn text_format_wave_counts_match_the_diamond() {
    let path = write_input("diamond", &diamond().to_string());
    let output = analyze_with(input_args_with(&path, &["--format", "text"]));
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout);
    // Diamond: wave 0 = {a, b}, wave 1 = {c, d}, wave 2 = {e}.
    assert!(text.contains("Wave 0 — 2 issues"), "text: {text}");
    assert!(text.contains("Wave 1 — 2 issues"), "text: {text}");
    assert!(text.contains("Wave 2 — 1 issue"), "text: {text}");
    let summary_line = |prefix: &str, text: &str| -> String {
        text.lines()
            .find(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing {prefix} line: {text}"))
            .to_string()
    };
    assert!(
        summary_line("Initially ready:", &text)
            .trim_end()
            .ends_with('2'),
        "text: {text}"
    );
    assert!(
        summary_line("Critical path:", &text)
            .trim_end()
            .ends_with('3'),
        "text: {text}"
    );
}

#[test]
fn empty_graph_is_analyzed_without_error() {
    let path = write_input(
        "empty",
        &serde_json::json!({ "issues": [], "hard_edges": [] }).to_string(),
    );
    let output = analyze_with(input_args(&path));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be JSON");
    assert_eq!(report["issue_count"], 0);
    assert_eq!(report["initial_width"], 0);
    assert_eq!(report["critical_path_length"], 0);
}

#[test]
fn unreadable_input_path_is_diagnostic() {
    let missing = std::env::temp_dir().join(format!(
        "autospec-graph-analyze-{}-does-not-exist.json",
        std::process::id()
    ));
    let output = analyze_with(input_args(&missing));
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "missing file must not panic: {stderr}"
    );
}
