use std::fs;
use std::path::PathBuf;

use autospec_core::validation::{
    CheckModes, CheckOwner, CheckReachability, CheckResult, ExternalCheck, StructuralCheck,
    ToolCommand, ValidationCatalog, ValidationCheck, ValidationExecutionReport, ValidationRunner,
};

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
fn tool_commands_reject_launcher_executables() {
    assert!(ToolCommand::new("env", ["bash", "-c", "echo unsafe"]).is_err());
    assert!(ToolCommand::new("env", ["-a", "ARG", "bash", "-c", "echo unsafe"]).is_err());
    assert!(ToolCommand::new("env", ["-ia", "ARG", "bash", "-c", "echo unsafe"]).is_err());
    assert!(ToolCommand::new("env", ["CARGO_TERM_COLOR=never", "cargo", "--version"]).is_err());
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
fn tool_commands_can_target_the_validation_root() {
    let command = ToolCommand::new(env!("CARGO"), ["--version"]).expect("safe command");
    let target_root = PathBuf::from("/tmp/autospec-validation-root");

    assert_eq!(command.working_directory_for(&target_root), target_root);
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

#[cfg(unix)]
#[test]
fn signaled_children_are_non_success_typed_results() {
    let command = ToolCommand::new(
        std::env::current_exe().expect("current test binary resolves"),
        [
            "--ignored",
            "--exact",
            "self_terminating_child_helper",
            "--nocapture",
        ],
    )
    .expect("direct child helper command is safe");

    let result = command.execute("signaled-child", true);

    assert_eq!(result.exit_code, None);
    assert_eq!(result.spawn_count, 1);
    assert!(result.is_failure());
}

#[cfg(unix)]
#[test]
#[ignore = "runs only as the signal-termination child helper"]
fn self_terminating_child_helper() {
    std::process::abort();
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

#[test]
fn runner_executes_rust_owners_in_catalog_order_and_fails_unimplemented_slots() {
    let catalog = ValidationCatalog::from_checks(vec![
        ValidationCheck {
            id: "check_lockstep",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::RustNative(StructuralCheck::TrioLockstep),
        },
        ValidationCheck {
            id: "check_unported",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::RustNative(StructuralCheck::CatalogSlot),
        },
    ]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("valid-skill"));

    assert_eq!(
        report
            .results
            .iter()
            .map(|result| result.id.as_str())
            .collect::<Vec<_>>(),
        ["check_lockstep", "check_unported"]
    );
    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 0);
    assert_eq!(report.results[1].exit_code, Some(1));
    assert!(report.results[1].stderr_bytes > 0);
}

#[test]
fn runner_aggregates_typed_derive_trio_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_derive_trio_consistency",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::DeriveTrioConsistency),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("derive-trio"));

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_aggregates_typed_bash_syntax_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_bash_syntax",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::BashSyntax),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("valid-skill"));

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_validates_frontmatter_for_each_discovered_trio_member() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_frontmatter",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::Frontmatter),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("frontmatter"));

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_rejects_empty_trio_frontmatter() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_frontmatter",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::Frontmatter),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("frontmatter-empty"));

    assert_eq!(report.results[0].exit_code, Some(1));
    assert_eq!(report.results[0].spawn_count, 0);
}

#[test]
fn runner_rejects_blank_only_trio_frontmatter() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_frontmatter",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::Frontmatter),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("frontmatter-blank"));

    assert_eq!(report.results[0].exit_code, Some(1));
    assert_eq!(report.results[0].spawn_count, 0);
}

#[test]
fn runner_executes_the_gap_miner_contract_as_a_typed_batch() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_gap_miner_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::GapMinerContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("gap-miner"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_checks_fleet_scripts_with_explicit_bash_syntax_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_fleet_scripts",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::FleetScripts),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("fleet-scripts"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 3);
}

#[test]
fn runner_parses_generated_yaml_through_a_typed_python_command() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_generated_yaml_parse",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::GeneratedYamlParse),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("generated-yaml"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 1);
}

#[test]
fn runner_checks_autospec_sweep_config_with_explicit_bash_syntax_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_sweep_config_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecSweepConfig),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("autospec-sweep-config"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 3);
}

#[test]
fn runner_checks_release_verdict_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_release_verdict_script",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ReleaseVerdictScript),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("release-verdict-script"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

fn repository_root() -> PathBuf {
    fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("workspace root resolves")
}

fn validation_fixture(name: &str) -> PathBuf {
    repository_root()
        .join("crates/autospec-cli/tests/fixtures/validation-cutover")
        .join(name)
}
