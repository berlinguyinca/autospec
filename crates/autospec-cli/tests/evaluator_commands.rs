//! End-to-end coverage for `autospec anchor` (issue #3558): register a frozen
//! suite, read the redacted mutation view, read the full qualification view,
//! and fail verification once a case artifact has been tampered with.
//!
//! Fixtures are staged rather than committed: a committed `v1.json` would carry
//! digests of committed artifacts, and a test that tampers with a committed
//! artifact tampers with the repository.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use autospec_core::evaluation::digest::Digest;

#[path = "support/temp_directory.rs"]
mod temp_directory;
use temp_directory::unique;

const SUITE: &str = "architecture-fixture";

fn autospec() -> Command {
    Command::new(env!("CARGO_BIN_EXE_autospec"))
}

fn run(root: &Path, args: &[&str]) -> Output {
    autospec()
        .args(args)
        .current_dir(root)
        .output()
        .expect("autospec runs")
}

#[track_caller]
fn ok(root: &Path, args: &[&str]) -> String {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "autospec {args:?} exited {}\nstderr:\n{}",
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[track_caller]
fn fails(root: &Path, args: &[&str]) -> String {
    let output = run(root, args);
    assert!(
        !output.status.success(),
        "autospec {args:?} unexpectedly succeeded\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Writes `<base>/cases/<case-id>/patch.diff` and returns the suite JSON entry
/// for it, with `content_digest` sealed over the bytes just written.
fn stage_case(
    base_rel: &str,
    base: &Path,
    id: usize,
    label: &str,
    visibility: &str,
    severity: &str,
    tags: &[&str],
) -> serde_json::Value {
    let name = format!("case-{id:02}");
    let directory = base.join("cases").join(&name);
    std::fs::create_dir_all(&directory).expect("stage case directory");
    let content = format!("--- a/{name}\n+++ b/{name}\n+{name}\n");
    std::fs::write(directory.join("patch.diff"), content.as_bytes()).expect("stage case artifact");
    serde_json::json!({
        "case_id": name,
        "artifact_ref": format!("{base_rel}/cases/{name}/patch.diff"),
        "expected_label": label,
        "severity": severity,
        "tags": tags,
        "visibility": visibility,
        "source": "handcrafted fixture",
        "adjudication": null,
        "content_digest": Digest::of_bytes(content.as_bytes()).to_string(),
    })
}

/// The `critical` subset the 40-case fixture carries. Suites whose cases do not
/// carry the tag are staged without it: a subset no case satisfies is invalid.
fn critical_subset() -> serde_json::Value {
    serde_json::json!({
        "name": "critical",
        "tag": "critical-security",
        "max_false_accept": 0,
        "max_false_reject": 0,
        "regression_tolerance_cases": 0,
    })
}

/// Writes `v1.json` into the staged `base` and returns its path **relative to
/// the run root**, which is what `--file` is handed (`run` executes there).
fn stage_suite(
    base: &Path,
    cases: Vec<serde_json::Value>,
    minimum: usize,
    subsets: &[serde_json::Value],
) -> PathBuf {
    let document = serde_json::json!({
        "schema": 1,
        "suite_id": SUITE,
        "version": 1,
        "slot": "architecture",
        "cases": cases,
        "minimum_case_count": minimum,
        "required_subsets": subsets,
        "provenance": {
            "created_by": "evaluator_commands test",
            "source": "handcrafted fixture",
            "notes": null,
        },
    });
    std::fs::write(
        base.join("v1.json"),
        serde_json::to_vec_pretty(&document).expect("serialize suite"),
    )
    .expect("stage suite");
    base.join("v1.json")
}

/// 40 cases: `case-01..20` development, `case-21..30` public regression,
/// `case-31..35` critical-security protected holdout (reject), `case-36..40`
/// protected holdout (accept). Returns the suite path relative to `root`.
fn stage_fixtures(root: &Path) -> PathBuf {
    let base_rel = "fixtures/anchors/architecture-fixture";
    let base = root.join(base_rel);
    std::fs::create_dir_all(&base).expect("stage fixture directory");
    let mut cases = Vec::new();
    for id in 1..=20 {
        cases.push(stage_case(
            base_rel,
            &base,
            id,
            "accept",
            "development",
            "low",
            &["development"],
        ));
    }
    for id in 21..=30 {
        cases.push(stage_case(
            base_rel,
            &base,
            id,
            "accept",
            "public_regression",
            "medium",
            &["public-regression"],
        ));
    }
    for id in 31..=35 {
        cases.push(stage_case(
            base_rel,
            &base,
            id,
            "reject",
            "protected_holdout",
            "critical",
            &["critical-security"],
        ));
    }
    for id in 36..=40 {
        cases.push(stage_case(
            base_rel,
            &base,
            id,
            "accept",
            "protected_holdout",
            "high",
            &["holdout"],
        ));
    }
    stage_suite(&base, cases, 40, &[critical_subset()])
}

#[test]
fn anchor_register_list_show_verify() {
    let root = unique("autospec-anchor");
    let suite = stage_fixtures(&root);
    let suite = suite.to_str().expect("utf-8 fixture path");

    ok(root.as_path(), &["anchor", "register", "--file", suite]);
    let listed = ok(root.as_path(), &["anchor", "list"]);
    assert!(listed.contains(SUITE), "list must name the suite: {listed}");

    // The default view is the mutation view: the holdout case ids stay (a
    // mutation run still targets them) but their labels are gone.
    let redacted = ok(root.as_path(), &["anchor", "show", &format!("{SUITE}@1")]);
    assert_eq!(redacted.matches(r#""expected_label": ""#).count(), 30);
    assert!(redacted.contains("case-40"), "mutation view keeps ids");

    // The qualification view sees everything.
    let full = ok(
        root.as_path(),
        &[
            "anchor",
            "show",
            &format!("{SUITE}@1"),
            "--role",
            "qualification",
        ],
    );
    assert_eq!(full.matches(r#""expected_label": ""#).count(), 40);

    ok(root.as_path(), &["anchor", "verify", &format!("{SUITE}@1")]);

    std::fs::write(
        root.join("fixtures/anchors/architecture-fixture/cases/case-07/patch.diff"),
        "tampered\n",
    )
    .expect("tamper fixture");

    let output = run(root.as_path(), &["anchor", "verify", &format!("{SUITE}@1")]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("case-07"),
        "stderr must name the tampered case"
    );
}

#[test]
fn anchor_register_is_write_once() {
    let root = unique("autospec-anchor-immutable");
    let suite = stage_fixtures(&root);
    let suite = suite.to_str().expect("utf-8 fixture path");

    ok(root.as_path(), &["anchor", "register", "--file", suite]);
    let stderr = fails(root.as_path(), &["anchor", "register", "--file", suite]);
    assert!(
        stderr.starts_with("immutable:"),
        "a second registration must be an immutable failure: {stderr}"
    );
}

#[test]
fn anchor_register_refuses_suites_whose_artifacts_do_not_match() {
    let root = unique("autospec-anchor-torn");
    let suite = stage_fixtures(&root);
    let suite = suite.to_str().expect("utf-8 fixture path");
    std::fs::write(
        root.join("fixtures/anchors/architecture-fixture/cases/case-03/patch.diff"),
        "rewritten after sealing\n",
    )
    .expect("tamper fixture");

    let stderr = fails(root.as_path(), &["anchor", "register", "--file", suite]);
    assert!(stderr.starts_with("integrity:"), "got: {stderr}");
    assert!(stderr.contains("case-03"), "got: {stderr}");
    // Nothing half-registered survives a refused registration.
    assert!(!root
        .join(".autospec/evaluation/anchors/architecture-fixture/v1.json")
        .exists());
}

#[test]
fn anchor_show_drops_quarantine_cases_for_mutation_role_only() {
    let root = unique("autospec-anchor-quarantine");
    let base_rel = "fixtures/anchors/architecture-fixture";
    let base = root.join(base_rel);
    std::fs::create_dir_all(&base).expect("stage fixture directory");
    let mut cases = vec![stage_case(
        base_rel,
        &base,
        1,
        "accept",
        "development",
        "low",
        &["development"],
    )];
    cases.push(stage_case(
        base_rel,
        &base,
        2,
        "reject",
        "quarantine",
        "high",
        &["disputed"],
    ));
    let suite = stage_suite(&base, cases, 1, &[]);
    let suite = suite.to_str().expect("utf-8 fixture path");
    ok(root.as_path(), &["anchor", "register", "--file", suite]);

    let mutation = ok(root.as_path(), &["anchor", "show", &format!("{SUITE}@1")]);
    assert!(
        !mutation.contains("quarantine"),
        "quarantine cases must not reach the mutation view: {mutation}"
    );
    let operator = ok(
        root.as_path(),
        &[
            "anchor",
            "show",
            &format!("{SUITE}@1"),
            "--role",
            "operator",
        ],
    );
    assert!(
        operator.contains("quarantine"),
        "the operator view is the stored suite: {operator}"
    );
}

#[test]
fn anchor_rejects_malformed_requests() {
    let root = unique("autospec-anchor-args");

    let stderr = fails(root.as_path(), &["anchor", "show", &format!("{SUITE}@1")]);
    assert!(stderr.starts_with("io:"), "missing suite: {stderr}");

    let stderr = fails(root.as_path(), &["anchor", "show", "no-version-here"]);
    assert!(stderr.starts_with("parse:"), "target shape: {stderr}");

    let stderr = fails(root.as_path(), &["anchor", "show", &format!("{SUITE}@0")]);
    assert!(stderr.starts_with("parse:"), "zero version: {stderr}");

    let stderr = fails(root.as_path(), &["anchor", "register"]);
    assert!(stderr.starts_with("parse:"), "missing --file: {stderr}");

    let stderr = fails(root.as_path(), &["anchor", "bogus"]);
    assert!(
        stderr.contains("unknown anchor subcommand"),
        "unknown subcommand: {stderr}"
    );
}

#[test]
fn anchor_list_is_empty_and_green_before_any_registration() {
    let root = unique("autospec-anchor-empty");
    let listed = ok(root.as_path(), &["anchor", "list"]);
    assert!(listed.is_empty(), "no suites registered: {listed:?}");
    let json = ok(root.as_path(), &["anchor", "list", "--json"]);
    assert!(json.contains("\"suites\""), "json envelope: {json}");
}
