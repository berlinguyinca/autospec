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
fn spec_parser_carries_gates_files_and_budget() {
    let source = r#"# Budgeted Example Spec

## Version

V99

## Objective

Exercise the extended spec metadata fields.

## Dependencies

- `v62-rust-core-workspace`

## Files To Create/Modify

- Create: `crates/autospec-core/src/spec/model.rs`
- Modify: docs/concepts.md

## Blocking Gates

- doc-sync
- file-size-ceiling

## Run Budget

40 tool calls, 3 self-review iterations

## Acceptance Criteria

- [ ] Model carries named blocking gates.

## Validation Commands

```bash
cargo test --all spec_parser
```
"#;

    let metadata = parse_spec(source).expect("extended spec parses");

    assert_eq!(
        metadata.blocking_gates,
        vec!["doc-sync", "file-size-ceiling"]
    );
    assert_eq!(
        metadata.files_to_touch,
        vec!["crates/autospec-core/src/spec/model.rs", "docs/concepts.md"]
    );
    assert_eq!(
        metadata.run_budget,
        "40 tool calls, 3 self-review iterations"
    );
    assert_eq!(metadata.dependencies, vec!["v62-rust-core-workspace"]);
}

#[test]
fn spec_parser_reads_files_from_generated_package() {
    let path = repo_root().join(
        ".autospec/generated-spec-packages/v62-final-platform/specs/v62-rust-core-workspace.md",
    );
    let source = fs::read_to_string(path).expect("generated package spec exists");

    let metadata = parse_spec(&source).expect("generated package spec parses");

    assert!(
        metadata
            .files_to_touch
            .iter()
            .any(|file| file == "Cargo.toml"),
        "files_to_touch should include the workspace Cargo.toml, got {:?}",
        metadata.files_to_touch
    );
    assert!(
        metadata
            .files_to_touch
            .iter()
            .any(|file| file == "README.md"),
        "files_to_touch should include the README, got {:?}",
        metadata.files_to_touch
    );
    // The v62 spec predates the blocking-gate and run-budget sections.
    assert!(
        metadata.blocking_gates.is_empty(),
        "v62 spec has no blocking gates"
    );
    assert!(metadata.run_budget.is_empty(), "v62 spec has no run budget");
}

#[test]
fn spec_metadata_json_serializes_extended_fields() {
    let source = r#"# Json Shape Spec

## Version

V98

## Objective

Verify the extended JSON shape.

## Blocking Gates

- doc-sync

## Files To Create/Modify

- Create: `README.md`

## Run Budget

20 tool calls
"#;

    let metadata = parse_spec(source).expect("spec parses");
    let json = metadata.to_json();

    assert!(json.contains("\"blocking_gates\":[\"doc-sync\"]"));
    assert!(json.contains("\"files_to_touch\":[\"README.md\"]"));
    assert!(json.contains("\"run_budget\":\"20 tool calls\""));
    // No Validation Commands section -> empty command, still serialized.
    assert!(json.contains("\"validation_command\":\"\""));
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
