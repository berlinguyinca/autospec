use autospec_core::spec::{parse_spec, ParseErrorKind, SpecStatus};
use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("crate is under crates/autospec-core")
        .to_path_buf()
}

#[test]
fn spec_parser_loads_generated_package_spec() {
    let path = repo_root().join(
        ".autospec/generated-spec-packages/v62-final-platform/specs/v62-rust-core-workspace.md",
    );
    let source = fs::read_to_string(path).expect("generated package spec exists");

    let metadata = parse_spec(&source).expect("generated package spec parses");

    assert_eq!(metadata.id.as_str(), "v62-rust-core-workspace");
    assert_eq!(metadata.title, "Rust Core Workspace Recovery");
    assert_eq!(metadata.version.as_str(), "V62");
    assert_eq!(metadata.status, SpecStatus::Ready);
    assert_eq!(
        metadata.dependencies,
        vec!["v61-recovery-public-launch-validation"]
    );
    assert!(metadata.objective.contains("Rust workspace"));
    assert!(metadata.validation_command.contains("cargo test --all"));
    assert!(metadata
        .to_json()
        .contains("\"id\":\"v62-rust-core-workspace\""));
}

#[test]
fn spec_parser_reports_missing_required_objective() {
    let source = "# Example Spec\n\n## Version\n\nV99\n";

    let error = parse_spec(source).expect_err("objective is required");

    assert_eq!(error.kind, ParseErrorKind::MissingRequiredField);
    assert_eq!(error.field.as_deref(), Some("objective"));
    assert!(error.line.is_none());
}

#[test]
fn spec_parser_rejects_malformed_dependency() {
    let source = "\
# Example Spec

## Version

V99

## Objective

Exercise dependency validation.

## Dependencies

- not a valid dependency id
";

    let error = parse_spec(source).expect_err("dependency id should be strict");

    assert_eq!(error.kind, ParseErrorKind::MalformedDependency);
    assert_eq!(error.line, Some(13));
}
