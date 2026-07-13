use std::fs;
use std::path::PathBuf;

use autospec_core::validation::{CheckResult, ToolCommand, ValidationExecutionReport};

#[test]
fn tool_commands_reject_shell_execution_shapes() {
    assert!(ToolCommand::new("sh", ["-c", "echo unsafe"]).is_err());
    assert!(ToolCommand::new("bash", ["-c", "echo unsafe"]).is_err());
    assert!(ToolCommand::new("bash", ["-ic", "echo unsafe"]).is_err());
    assert!(ToolCommand::new("bash", ["--command", "echo unsafe"]).is_err());
    assert!(ToolCommand::new("bash", ["--command=echo unsafe"]).is_err());
    assert!(ToolCommand::new("fish", ["-c", "echo unsafe"]).is_err());
}

#[test]
fn tool_commands_allow_non_executing_shell_flags() {
    assert!(ToolCommand::new("bash", ["-n", "scripts/validate.sh"]).is_ok());
}

#[test]
fn tool_commands_execute_explicit_arguments_from_the_repository_root() {
    let command = ToolCommand::new(env!("CARGO"), ["--version"]).expect("safe command");

    assert_eq!(command.working_directory(), repository_root());

    let result = command.execute("cargo-version", true);

    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.spawn_count, 1);
    assert!(result.stdout_bytes > 0);
}

#[test]
fn missing_programs_are_non_success_typed_results() {
    let command = ToolCommand::new("autospec-task-two-missing-program", ["--version"])
        .expect("safe missing command definition");

    let result = command.execute("missing-tool", true);

    assert_eq!(result.exit_code, None);
    assert_eq!(result.spawn_count, 0);
    assert!(result.is_failure());
}

#[test]
fn completed_result_serializes_execution_metadata() {
    let result = CheckResult::completed("lockstep", true, 0, 12, 1, 4, 0, "digest");

    assert!(result.to_json().contains("\"elapsed_ms\":12"));
    assert!(result.to_json().contains("\"schema\":2"));
}

#[test]
fn execution_report_aggregates_required_failures_as_schema_two() {
    let report = ValidationExecutionReport::new(vec![
        CheckResult::completed("lockstep", true, 0, 12, 1, 4, 0, "passed"),
        CheckResult::completed("missing-tool", true, 1, 5, 1, 0, 3, "failed"),
        CheckResult::completed("advisory", false, 1, 1, 1, 0, 1, "optional"),
    ]);

    let aggregate = report.aggregate().expect("execution results aggregate");

    assert_eq!(aggregate.required_failed, 1);
    assert_eq!(aggregate.optional_failed, 1);
    assert!(aggregate.to_json().contains("\"schema\":2"));
    assert!(report
        .to_json()
        .expect("execution report renders")
        .contains("\"results\""));
}

fn repository_root() -> PathBuf {
    fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("workspace root resolves")
}
