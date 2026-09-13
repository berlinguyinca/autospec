//! Golden tests for `autospec_core::issue_skeleton` (issue #4440).
//!
//! These are the authoritative golden pins for the Rust port of
//! `scripts/gen-issue-skeleton.sh`. The shell's Bats suite
//! (`tests/gen-issue-skeleton.bats`) exercised the same fixtures through the
//! script; these tests exercise the same fixtures through the Rust renderer
//! directly, byte-for-byte.
//!
//! The render is byte-stable for inputs that omit the optional
//! `implementation_surface` field, so the existing goldens (which predate that
//! field) are compared unmodified.

use std::path::PathBuf;

use autospec_core::issue_skeleton;

fn fixture_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is `crates/autospec-core`; the fixtures live at the
    // repository root under `tests/fixtures/gen-issue-skeleton`.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("gen-issue-skeleton")
        .canonicalize()
        .expect("fixture dir resolvable")
}

fn read_fixture(name: &str) -> String {
    let path = fixture_dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

/// Assert the rendered body (plus the single trailing newline the CLI emits)
/// is byte-for-byte equal to the golden file.
fn assert_renders_golden(yaml_name: &str, golden_name: &str) {
    let yaml = read_fixture(yaml_name);
    let skeleton =
        issue_skeleton::parse(&yaml).unwrap_or_else(|error| panic!("{yaml_name}: {error}"));
    let body = skeleton.render();
    let expected = read_fixture(golden_name);
    assert_eq!(
        format!("{body}\n"),
        expected,
        "{yaml_name} render differs from {golden_name}"
    );
}

#[test]
fn minimal_renders_byte_identical_golden() {
    assert_renders_golden("minimal.yaml", "expected-minimal.md");
}

#[test]
fn security_database_renders_byte_identical_golden() {
    assert_renders_golden("security-database.yaml", "expected-security-database.md");
}

#[test]
fn minimal_has_no_blocking_findings() {
    let yaml = read_fixture("minimal.yaml");
    let skeleton = issue_skeleton::parse(&yaml).unwrap();
    assert_eq!(skeleton.blocking_finding_count(), 0);
}

#[test]
fn security_database_blocking_findings_do_not_influence_exit() {
    // The security golden carries one non-blocking AS-DAG-001 warning. The
    // exit code is driven only by blocking findings, so it stays 0.
    let yaml = read_fixture("security-database.yaml");
    let skeleton = issue_skeleton::parse(&yaml).unwrap();
    assert_eq!(skeleton.blocking_finding_count(), 0);
    let findings = skeleton.lint_findings();
    assert!(
        findings
            .iter()
            .any(|finding| finding.rule.id() == "AS-DAG-001" && !finding.is_blocking()),
        "expected a non-blocking AS-DAG-001 warning, got: {findings:?}"
    );
}

#[test]
fn vague_goal_fixture_is_rejected_before_render() {
    // The fixture omits required list fields, so parsing is fail-closed
    // (MISSING_FIELD) before any lint runs. The observable contract is: a
    // non-zero outcome and no rendered `## Goal` section.
    let yaml = read_fixture("vague-goal.yaml");
    let result = issue_skeleton::parse(&yaml);
    let err = result.expect_err("vague-goal fixture must not parse cleanly");
    assert!(
        err.starts_with("MISSING_FIELD:"),
        "expected a MISSING_FIELD error, got: {err}"
    );
}

#[test]
fn missing_required_field_is_fail_closed() {
    let yaml = "issue_id: 1\nspec_path: a\nspec_url: b\n";
    let err = issue_skeleton::parse(yaml).expect_err("goal_sentence missing");
    assert_eq!(err, "MISSING_FIELD:goal_sentence");
}

#[test]
fn yaml_parse_error_is_reported() {
    let yaml = "issue_id: 1\n  bad: [indent\n";
    let result = issue_skeleton::parse(yaml);
    assert!(
        result.is_err(),
        "malformed YAML must be rejected, got: {result:?}"
    );
}

#[test]
fn implementation_surface_section_is_rendered_when_present() {
    let mut yaml = String::from(&read_fixture("minimal.yaml"));
    yaml.push_str("\nimplementation_surface: crates/autospec-core (issue_skeleton module)\n");
    let skeleton = issue_skeleton::parse(&yaml).unwrap();
    let body = skeleton.render();
    assert!(
        body.contains(
            "## Implementation scope\n\n- scripts/gen-issue-skeleton.sh with --input <file> and stdin fallback\n\n## Implementation surface\n\ncrates/autospec-core (issue_skeleton module)\n\n## Out of scope"
        ),
        "implementation surface section missing or mis-spaced:\n{body}"
    );
}

#[test]
fn omitting_implementation_surface_keeps_golden_stable() {
    // Regression guard: the optional field must not perturb the byte-stable
    // render when absent.
    let yaml = read_fixture("minimal.yaml");
    let skeleton = issue_skeleton::parse(&yaml).unwrap();
    assert!(skeleton.implementation_surface.is_none());
    assert!(!skeleton.render().contains("## Implementation surface"));
}
