use std::fs;
use std::path::PathBuf;

use autospec_core::validation::{
    CheckModes, CheckOwner, CheckReachability, CheckResult, ExternalCheck, Jobs, StructuralCheck,
    ToolCommand, ValidationCatalog, ValidationCheck, ValidationExecutionReport, ValidationOptions,
    ValidationPlan, ValidationRunner, ValidationStatus,
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
    assert!(ToolCommand::new("bash", ["-n", "scripts/lint-issue.sh"]).is_ok());
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
fn runner_executes_rust_owners_in_catalog_order() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_lockstep",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::RustNative(StructuralCheck::TrioLockstep),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("valid-skill"));

    assert_eq!(
        report
            .results
            .iter()
            .map(|result| result.id.as_str())
            .collect::<Vec<_>>(),
        ["check_lockstep"]
    );
    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 0);
}

#[test]
fn direct_plan_keeps_reachable_occurrences_and_excludes_fast_only_suites() {
    let catalog = ValidationCatalog::standard();
    let full = ValidationPlan::build(&catalog, &ValidationOptions::default())
        .expect("full validation plan builds");
    let fast = ValidationPlan::build(
        &catalog,
        &ValidationOptions::parse(["--fast"]).expect("fast validation options parse"),
    )
    .expect("fast validation plan builds");

    assert_eq!(full.ids().len(), 159); // +9: orphaned-suite ratchet (#3360); +1: code intelligence; +2: #3485 orphan owners; +1: deferral-ref lint (#3497); +2: loud-failure gates (#3535)
    assert_eq!(full.unique_ids().len(), 154); // reached directly, duplicated by nothing
    assert!(!full.ids().contains(&"check_architecture_fitness_engine"));
    assert!(full.ids().contains(&"check_python_suites"));
    assert!(full.ids().contains(&"check_install_tests"));
    assert!(!fast.ids().contains(&"check_python_suites"));
    // The orphan ratchet itself is filesystem-only, so it stays in --fast; the
    // suites it caught are BatsSuite owners and drop out.
    assert!(fast.ids().contains(&"check_bats_suite_registration"));
    assert_eq!(fast.ids().len(), 137);
    assert!(!fast.ids().contains(&"check_install_tests"));
    assert!(fast.ids().iter().all(|id| {
        !matches!(
            *id,
            "check_lint_heredoc_handling"
                | "check_lint_reuse_triage"
                | "check_ship_completeness"
                | "check_autonomous_phase2_suite"
                | "check_persona_suite"
                | "check_reuse_lens_suite"
                | "check_bats_negation_ratchet"
                | "check_code_intelligence_contract"
                | "check_autospec_fleet_enabled_false"
                | "check_autospec_sweep_enabled_false"
                | "check_classify_lang_labels"
                | "check_classify_language"
                | "check_define_phase0_language"
                | "check_language_axis_integration"
                | "check_language_table"
                | "check_proxy_direct_borrow_lifetime"
                | "check_qa_function_ranges_string_literals"
                | "check_verify_gate"
                | "check_verify_produced_work"
        )
    }));
}

#[test]
fn scoped_direct_plan_records_git_inputs_without_skipping_fail_safe_global_checks() {
    let catalog = ValidationCatalog::standard();
    let options =
        ValidationOptions::parse(["--changed=HEAD"]).expect("scoped validation options parse");
    let full = ValidationPlan::build(&catalog, &ValidationOptions::default())
        .expect("full direct plan builds");
    let scoped = ValidationPlan::build_with_changed_paths(
        &catalog,
        &options,
        ["skills/autospec-run/SKILL.md"],
    )
    .expect("scoped direct plan builds");

    assert_eq!(scoped.changed_base(), Some("HEAD"));
    assert_eq!(
        scoped.changed_paths(),
        &["skills/autospec-run/SKILL.md".to_string()]
    );
    assert_eq!(scoped.ids(), full.ids());
}

#[test]
fn parallel_direct_plan_keeps_results_in_catalog_order() {
    let checks = ["first", "second"]
        .into_iter()
        .map(|id| ValidationCheck {
            id,
            required: true,
            independent: true,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::RustNative(StructuralCheck::TrioLockstep),
        })
        .collect();
    let plan = ValidationPlan::from_checks(checks, false, Jobs::Fixed(2));

    let report = ValidationRunner::run_plan(&plan, &validation_fixture("valid-skill"));

    assert_eq!(
        report
            .results
            .iter()
            .map(|result| result.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn direct_plan_preserves_repeated_legacy_check_ids_as_distinct_occurrences() {
    let checks = ["check_bash_syntax", "check_bash_syntax"]
        .into_iter()
        .map(|id| ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::RustNative(StructuralCheck::TrioLockstep),
        })
        .collect();
    let plan = ValidationPlan::from_checks(checks, false, Jobs::Fixed(1));

    let report = ValidationRunner::run_plan(&plan, &validation_fixture("valid-skill"));

    assert_eq!(report.results.len(), 2);
    assert!(report.to_json().is_ok());
}

#[test]
fn fast_direct_plan_skips_embedded_bats_without_skipping_static_contracts() {
    let plan = ValidationPlan::from_checks(
        vec![ValidationCheck {
            id: "check_autospec_doc_contract",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecDocContract),
        }],
        true,
        Jobs::Fixed(1),
    );

    let report = ValidationRunner::run_plan(&plan, &validation_fixture("autospec-doc-contract"));

    // The static contracts still ran and passed; the embedded Bats suite was skipped, and
    // a skipped suite is now unmeasured rather than exit code 0. Fast mode makes
    // validation quicker; it must not make uncovered ground look green (#3535).
    assert_eq!(report.results[0].exit_code, None);
    assert!(report.results[0].is_unmeasured(), "{:?}", report.results[0]);
    assert!(!report.results[0].is_success());
    assert_eq!(
        report.aggregate().expect("plan aggregates").status,
        ValidationStatus::Unknown
    );
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
    assert_eq!(report.results[0].spawn_count, 3);
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
    assert_eq!(report.results[0].spawn_count, 1);
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

#[test]
fn runner_skips_missing_bats_but_uses_a_direct_bats_command_when_available() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_lint_heredoc_handling",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(
            "tests/compute-release-verdict.bats",
        )),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("release-verdict-script"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 1);
}

#[test]
fn runner_checks_reviewer_reuse_policy_before_running_its_bats_suite() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_reviewer_reuse_lens",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ReviewerReuseLens),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("reviewer-reuse-lens"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 1);
}

#[test]
fn runner_checks_bash_help_with_direct_argument_vectors() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_usage_limit_helper",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::BashHelpUsage(
            "scripts/autospec-usage-limit.sh",
        )),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("bash-help-usage"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_rejects_bash_help_without_a_usage_line() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_usage_limit_helper",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::BashHelpUsage(
            "scripts/autospec-usage-limit.sh",
        )),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("bash-help-no-usage"));

    assert_eq!(report.results[0].exit_code, Some(1));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_checks_supersession_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_supersession_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::SupersessionContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("supersession-contract"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((2..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_run_summary_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_run_summary_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::RunSummaryContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("run-summary-contract"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((2..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_optional_database_module_install_contract_before_bats() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_db_module_install",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::DbModuleInstall),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("db-module-install"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 1);
}

#[test]
fn runner_runs_a_sorted_bats_directory_without_shell_globbing() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_persona_suite",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::BatsDirectory("tests/persona")),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("bats-directory"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 1);
}

#[test]
fn runner_checks_upgrade_skill_tokens_before_its_bats_directory() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_upgrade_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecUpgradeContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("autospec-upgrade-contract"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 1);
}

#[test]
fn runner_checks_claim_guard_script_and_required_bats_suites_directly() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_claim_guard_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ClaimGuardContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("claim-guard-contract"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=4).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_qa_and_loop_support_contracts_without_the_shell_harness() {
    let catalog = ValidationCatalog::from_checks(vec![
        ValidationCheck {
            id: "check_autospec_qa_cluster_contract",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecQaClusterContract),
        },
        ValidationCheck {
            id: "check_autospec_qa_bug_class_contract",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecQaBugClassContract),
        },
        ValidationCheck {
            id: "check_loop_handoff_harness_awareness",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::LoopHandoffHarnessAwareness),
        },
    ]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("qa-loop-support-contracts"));

    assert!(report
        .results
        .iter()
        .all(|result| result.exit_code == Some(0)));
    assert!(report.results.iter().all(|result| result.spawn_count == 1));
}

#[test]
fn runner_rejects_an_unguarded_claim_label_swap_without_the_shell_harness() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_claim_cas_guard",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ClaimCasGuard),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("claim-cas-guard-unguarded"));

    assert_eq!(report.results[0].exit_code, Some(1));
    assert_eq!(report.results[0].spawn_count, 0);
}

#[test]
fn runner_checks_claim_cas_simulation_coverage_with_direct_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_claim_cas_guard",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ClaimCasGuard),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("claim-cas-guard"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 2);
}

#[test]
fn runner_rejects_watchdog_gc_that_deletes_worktrees_with_rm_rf() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_watchdog_worktree_gc",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::WatchdogWorktreeGc),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("watchdog-gc-rm-rf"));

    assert_eq!(report.results[0].exit_code, Some(1));
    assert_eq!(report.results[0].spawn_count, 1);
}

#[test]
fn runner_checks_watchdog_gc_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_watchdog_worktree_gc",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::WatchdogWorktreeGc),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("watchdog-gc"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=2).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_hashes_expanded_skill_members_without_a_shell_pipeline() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_block_expansion",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::BlockExpansion),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("block-expansion"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 6);
}

#[test]
fn runner_reports_block_expansion_golden_mismatches_after_hashing() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_block_expansion",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::BlockExpansion),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("block-expansion-mismatch"));

    assert_eq!(report.results[0].exit_code, Some(1));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_checks_explore_implementer_base_with_direct_bats_coverage() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_explore_implementer_base",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreImplementerBase),
    }]);

    let report = ValidationRunner::run(
        &catalog,
        &validation_fixture("explore-implementer-base-contract"),
    );

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 1);
}

#[test]
fn runner_checks_explore_researcher_contracts_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![
        ValidationCheck {
            id: "check_autospec_explore_researchers_deterministic",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(
                ExternalCheck::AutospecExploreResearchersDeterministic,
            ),
        },
        ValidationCheck {
            id: "check_autospec_explore_researchers_llm",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreResearchersLlm),
        },
    ]);

    let report = ValidationRunner::run(
        &catalog,
        &validation_fixture("explore-researcher-contracts"),
    );

    assert!(report
        .results
        .iter()
        .all(|result| result.exit_code == Some(0)));
    assert!((5..=7).contains(&report.results[0].spawn_count));
    assert!((3..=5).contains(&report.results[1].spawn_count));
}

#[test]
fn runner_checks_explore_specialist_schema_without_shell_scanner_requirement() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_explore_specialists_discovery",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreSpecialistsDiscovery),
    }]);

    let report = ValidationRunner::run(
        &catalog,
        &validation_fixture("explore-specialists-contract"),
    );

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 2);
}

#[test]
fn runner_checks_explore_stage2_intersect_contract_with_direct_bash_syntax() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_explore_stage2_intersect_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreStage2Intersect),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("explore-stage2-intersect"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 1);
}

#[test]
fn runner_checks_explore_worktree_assert_contract_without_the_shell_harness() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_explore_trio_worktree_assert",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ExploreTrioWorktreeAssert),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("explore-worktree-assert"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!(report.results[0].spawn_count <= 1);
}

#[test]
fn runner_checks_explore_spec_first_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_explore_spec_first_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreSpecFirst),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("explore-spec-first"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_explore_qa_gate_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_explore_qa_gate_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreQaGate),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("explore-qa-gate"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_explore_style_normalization_with_direct_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_explore_style_normalization_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreStyleNormalization),
    }]);

    let report =
        ValidationRunner::run(&catalog, &validation_fixture("explore-style-normalization"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=2).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_explore_orchestrator_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_explore_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreOrchestrator),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("explore-orchestrator"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((10..=11).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_explore_discovery_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_explore_discovery_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreDiscovery),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("explore-discovery"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((5..=11).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_qa_root_contract_with_a_direct_bash_syntax_command() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_qa_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecQaContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("qa-root-contract"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 1);
}

#[test]
fn runner_checks_qa_deployment_contract_with_typed_tool_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_qa_deploy_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::QaDeployContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("qa-deploy-contract"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=4).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_qa_verify_first_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_qa_verify_first_discipline",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::QaVerifyFirstDiscipline),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("qa-verify-first"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((3..=5).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_qa_exhaustiveness_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_qa_exhaustiveness_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::QaExhaustivenessContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("qa-exhaustiveness"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=2).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_qa_incident_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_qa_incident_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::QaIncidentContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("qa-incident"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=2).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_qa_heal_loop_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_qa_heal_loop_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::QaHealLoopContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("qa-heal-loop"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((2..=4).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_quality_differential_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_quality_differential",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::QualityDifferential),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("quality-differential"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=2).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_release_area_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_release_area_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ReleaseAreaContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("release-area"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=2).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_release_worktree_assert_contract_with_direct_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_release_trio_worktree_assert",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ReleaseWorktreeAssert),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("release-worktree-assert"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((0..=1).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_fab_container_pin_lint_with_direct_bash_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_fab_container_dockerfile",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::FabContainerPinLint),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("fab-container-pin-lint"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_checks_repo_quality_audit_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_repo_quality_audit_loop",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::RepoQualityAudit),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("repo-quality-audit"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_autonomous_mode_contract_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_autonomous_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecAutonomousContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("autonomous-mode"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((2..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_dogfood_detectors_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_dogfood_detectors",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::DogfoodDetectors),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("dogfood-detectors"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((4..=6).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_parallel_dispatch_with_direct_bash_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_parallel_dispatch_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecParallelDispatch),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("parallel-dispatch"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((1..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn tool_commands_can_remove_a_previously_configured_environment_variable() {
    let command = ToolCommand::new("bash", ["command-env-check.sh"])
        .expect("fixture command uses direct arguments")
        .with_env("AUTOSPEC_VALIDATION_ENV_CHECK", "configured")
        .without_env("AUTOSPEC_VALIDATION_ENV_CHECK");

    let result = command.execute_in(
        "check_environment_removal",
        true,
        &validation_fixture("valid-skill"),
    );

    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.spawn_count, 1);
}

#[test]
fn runner_checks_growth_and_telemetry_contracts_with_typed_batches() {
    for (id, owner, fixture, minimum_spawns, maximum_spawns) in [
        (
            "check_growth_shared_contract",
            ExternalCheck::GrowthShared,
            "growth-shared",
            7,
            14,
        ),
        (
            "check_growth_candidate_pipeline_contract",
            ExternalCheck::GrowthCandidatePipeline,
            "growth-candidate-pipeline",
            4,
            9,
        ),
        (
            "check_grow_run_pipeline_contract",
            ExternalCheck::GrowRunPipeline,
            "grow-run-pipeline",
            8,
            14,
        ),
        (
            "check_db_telemetry_contract",
            ExternalCheck::DbTelemetry,
            "db-telemetry",
            1,
            4,
        ),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(owner),
        }]);

        let report = ValidationRunner::run(&catalog, &validation_fixture(fixture));

        assert_eq!(report.results[0].exit_code, Some(0), "{id}");
        assert!(
            (minimum_spawns..=maximum_spawns).contains(&report.results[0].spawn_count),
            "{id}"
        );
    }
}

#[test]
fn runner_checks_worktree_ladder_parity_with_typed_stdin_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_worktree_ladder_assert_parity",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::WorktreeLadderAssertParity),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("worktree-ladder"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 1); // #3262 narrowed it to autospec-run
}

#[test]
fn runner_checks_phase4_policy_gates_with_direct_bats_commands() {
    for (id, owner, fixture, minimum_spawns, maximum_spawns) in [
        (
            "check_phase4_single_agent_discipline",
            ExternalCheck::Phase4SingleAgentDiscipline,
            "phase4-single-agent",
            0,
            1,
        ),
        (
            "check_phase4_final_quality_gate",
            ExternalCheck::Phase4FinalQualityGate,
            "phase4-final-quality",
            0,
            2,
        ),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(owner),
        }]);

        let report = ValidationRunner::run(&catalog, &validation_fixture(fixture));

        assert_eq!(report.results[0].exit_code, Some(0), "{id}");
        assert!(
            (minimum_spawns..=maximum_spawns).contains(&report.results[0].spawn_count),
            "{id}"
        );
    }
}

#[test]
fn runner_phase4_final_quality_gate_fails_closed_when_discovery_suite_is_missing() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_phase4_final_quality_gate",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::Phase4FinalQualityGate),
    }]);

    let report = ValidationRunner::run(
        &catalog,
        &validation_fixture("phase4-final-quality-missing-discovery"),
    );

    assert_eq!(report.results[0].exit_code, Some(1));
    assert_eq!(report.results[0].spawn_count, 0);
    assert_eq!(
        report.results[0].stderr_bytes,
        b"tests/unit/test_quality_gate_discovery.bats: bats coverage missing".len()
    );
}

#[test]
fn runner_checks_refine_continue_and_loop_contracts_with_typed_batches() {
    for (id, owner, fixture, minimum_spawns, maximum_spawns) in [
        (
            "check_autospec_refine_contract",
            ExternalCheck::AutospecRefineContract,
            "autospec-refine",
            5,
            7,
        ),
        (
            "check_autospec_continue_contract",
            ExternalCheck::AutospecContinueContract,
            "autospec-continue",
            3,
            5,
        ),
        (
            "check_autospec_loop_contract",
            ExternalCheck::AutospecLoopContract,
            "autospec-loop",
            2,
            2,
        ),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(owner),
        }]);

        let report = ValidationRunner::run(&catalog, &validation_fixture(fixture));

        assert_eq!(report.results[0].exit_code, Some(0), "{id}");
        assert!(
            (minimum_spawns..=maximum_spawns).contains(&report.results[0].spawn_count),
            "{id}"
        );
    }
}

#[test]
fn runner_checks_resume_contract_components_with_typed_batches() {
    for (id, owner, minimum_spawns, maximum_spawns) in [
        (
            "check_autospec_resume_structure",
            ExternalCheck::AutospecResumeStructure,
            3,
            3,
        ),
        (
            "check_autospec_supervisor_structure",
            ExternalCheck::AutospecSupervisorStructure,
            2,
            2,
        ),
        (
            "check_autospec_resume_contract",
            ExternalCheck::AutospecResumeContract,
            6,
            7,
        ),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(owner),
        }]);

        let report = ValidationRunner::run(&catalog, &validation_fixture("autospec-resume"));

        assert_eq!(report.results[0].exit_code, Some(0), "{id}");
        assert!(
            (minimum_spawns..=maximum_spawns).contains(&report.results[0].spawn_count),
            "{id}"
        );
    }
}

#[test]
fn runner_checks_prompt_contracts_with_captured_typed_output() {
    for (id, owner) in [
        (
            "check_implementer_contract",
            ExternalCheck::ImplementerContract,
        ),
        ("check_reviewer_contract", ExternalCheck::ReviewerContract),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(owner),
        }]);

        let report = ValidationRunner::run(&catalog, &validation_fixture("prompt-contracts"));

        assert_eq!(report.results[0].exit_code, Some(0), "{id}");
        assert_eq!(report.results[0].spawn_count, 1, "{id}");
    }
}

#[test]
fn runner_checks_autonomy_wiring_contracts_with_direct_bash_and_bats() {
    for (id, owner, fixture, minimum_spawns, maximum_spawns) in [
        (
            "check_conductor_wiring_contract",
            ExternalCheck::ConductorWiringContract,
            "conductor-wiring",
            2,
            3,
        ),
        (
            "check_autonomy_guardrails_foundation",
            ExternalCheck::AutonomyGuardrailsFoundation,
            "autonomy-guardrails",
            1,
            2,
        ),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(owner),
        }]);

        let report = ValidationRunner::run(&catalog, &validation_fixture(fixture));

        assert_eq!(report.results[0].exit_code, Some(0), "{id}");
        assert!(
            (minimum_spawns..=maximum_spawns).contains(&report.results[0].spawn_count),
            "{id}"
        );
    }
}

#[test]
fn runner_checks_python_suites_with_a_typed_pytest_command() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_python_suites",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::PythonSuites),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("python-suites"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_checks_growth_and_documentation_contracts_with_typed_tool_batches() {
    for (id, owner, fixture, expected_spawns) in [
        (
            "check_grow_define_contract",
            ExternalCheck::GrowDefineContract,
            "grow-define-contract",
            8,
        ),
        (
            "check_autospec_doc_contract",
            ExternalCheck::AutospecDocContract,
            "autospec-doc-contract",
            3,
        ),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(owner),
        }]);

        let report = ValidationRunner::run(&catalog, &validation_fixture(fixture));

        assert_eq!(report.results[0].exit_code, Some(0), "{id}");
        assert_eq!(report.results[0].spawn_count, expected_spawns, "{id}");
    }
}

#[test]
fn runner_checks_constitution_and_install_contracts_with_typed_tool_batches() {
    for (id, owner, fixture, expected_spawns) in [
        (
            "check_constitution_validation_contract",
            ExternalCheck::ConstitutionValidation,
            "constitution-validation",
            50,
        ),
        (
            "check_install_tests",
            ExternalCheck::InstallTests,
            "install-tests",
            1,
        ),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(owner),
        }]);

        let report = ValidationRunner::run(&catalog, &validation_fixture(fixture));

        assert_eq!(report.results[0].exit_code, Some(0), "{id}");
        assert_eq!(report.results[0].spawn_count, expected_spawns, "{id}");
    }
}

#[test]
fn runner_checks_control_plane_bootstrap_with_typed_tool_batches() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_control_plane_bootstrap_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ControlPlaneBootstrap),
    }]);

    let report = ValidationRunner::run(
        &catalog,
        &validation_fixture("control-plane-bootstrap-contract"),
    );

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 22);
}

#[test]
fn runner_checks_sweep_area_contract_with_direct_syntax_and_bats_commands() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_sweep_area_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecSweepAreaContract),
    }]);

    let report = ValidationRunner::run(
        &catalog,
        &validation_fixture("autospec-sweep-area-contract"),
    );

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((2..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_checks_fab_skill_tokens_before_its_bats_directory() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_autospec_fab_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecFabContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("autospec-fab-contract"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert!((2..=3).contains(&report.results[0].spawn_count));
}

#[test]
fn runner_executes_per_skill_validators_with_direct_argument_vectors() {
    let catalog = ValidationCatalog::from_checks(vec![
        ValidationCheck {
            id: "check_autospec_test_skill_present",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecTestSkill),
        },
        ValidationCheck {
            id: "check_autospec_playwright_skill_present",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::AutospecPlaywrightSkill),
        },
    ]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("autospec-skill-validators"));

    assert!(report
        .results
        .iter()
        .all(|result| result.exit_code == Some(0)));
    assert!(report.results.iter().all(|result| result.spawn_count == 2));
}

#[test]
fn runner_requires_and_runs_the_complete_grooming_suite_batch() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_grooming_contract",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::GroomingContract),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("grooming-contract"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 10);
}

#[test]
fn runner_runs_the_optional_mutation_gate_with_direct_processes() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_mutation_and_negative_path",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::MutationAndNegativePath),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("mutation-negative-path"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 4);
}

#[test]
fn runner_checks_lint_implementation_help_for_every_rule_id() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_lint_implementation_helpers",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::LintImplementationHelpers),
    }]);

    let report =
        ValidationRunner::run(&catalog, &validation_fixture("lint-implementation-helpers"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 2);
}

#[test]
fn runner_checks_lint_issue_fixtures_and_its_direct_bats_suite() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_lint_issue_helpers",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::LintIssueHelpers),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("lint-issue-helpers"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 3);
}

#[test]
fn runner_executes_ci_status_and_define_worktree_smoke_contracts_directly() {
    let catalog = ValidationCatalog::from_checks(vec![
        ValidationCheck {
            id: "check_phase4_ci_status_compare",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::Phase4CiStatusCompare),
        },
        ValidationCheck {
            id: "check_define_spec_worktree_routing",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::DefineSpecWorktreeRouting),
        },
    ]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("phase4-script-contracts"));

    assert!(report
        .results
        .iter()
        .all(|result| result.exit_code == Some(0)));
    assert_eq!(report.results[0].spawn_count, 3);
    assert_eq!(report.results[1].spawn_count, 1);
}

#[test]
fn runner_checks_groom_preflight_and_grow_run_contracts_directly() {
    let catalog = ValidationCatalog::from_checks(vec![
        ValidationCheck {
            id: "check_run_groom_preflight_contract",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::RunGroomPreflightContract),
        },
        ValidationCheck {
            id: "check_grow_run_contract",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::GrowRunContract),
        },
    ]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("growth-groom-contracts"));

    assert!(report
        .results
        .iter()
        .all(|result| result.exit_code == Some(0)));
    assert_eq!(report.results[0].spawn_count, 2);
    assert_eq!(report.results[1].spawn_count, 1);
}

#[test]
fn runner_checks_performance_and_ux_workstream_contracts_directly() {
    let catalog = ValidationCatalog::from_checks(vec![
        ValidationCheck {
            id: "check_performance_workstream_contract",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::PerformanceWorkstream),
        },
        ValidationCheck {
            id: "check_ux_ui_workstream_contract",
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::UxUiWorkstream),
        },
    ]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("workstream-contracts"));

    assert!(report
        .results
        .iter()
        .all(|result| result.exit_code == Some(0)));
    assert!(report.results.iter().all(|result| result.spawn_count == 1));
}

#[test]
fn runner_keeps_token_baseline_freshness_as_a_warn_only_check() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_token_baseline_fresh",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::TokenBaselineFresh),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("token-baseline-fresh"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 1);
}

#[test]
fn runner_owns_the_legacy_unreachable_architecture_fitness_contract() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_architecture_fitness_engine",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::LegacyUnreachable,
        owner: CheckOwner::ExternalBatch(ExternalCheck::ArchitectureFitnessEngine),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("architecture-fitness"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 3);
}

#[test]
fn runner_runs_phase4_and_docs_shell_tests_with_a_typed_fleet_environment() {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_phase4_tests",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::Phase4TestSuites),
    }]);

    let report = ValidationRunner::run(&catalog, &validation_fixture("phase4-tests"));

    assert_eq!(report.results[0].exit_code, Some(0));
    assert_eq!(report.results[0].spawn_count, 3);
}

fn repository_root() -> PathBuf {
    fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("workspace root resolves")
}

#[test]
fn runner_fails_a_bats_suite_that_no_validate_check_invokes() {
    let root = bats_registration_root("orphaned", &["orphan.bats"], "");

    let report = run_bats_registration(&root);

    assert_eq!(
        report.results[0].exit_code,
        Some(1),
        "a suite referenced by nothing must fail the check"
    );
    // The message text is not carried on CheckResult, only its length and digest,
    // so this pins the exact rendered string rather than merely "non-empty".
    let expected = "tests/unit/orphan.bats: bats suite invoked by no validate check; \
register it in crates/autospec-core/src/validation or, if it is genuinely not a \
suite, say so in bats_registration_baseline.rs";
    assert_eq!(
        report.results[0].stderr_bytes,
        expected.len(),
        "the failure must name the orphaned suite and how to resolve it"
    );
    assert_eq!(
        report.results[0].output_digest, "d3696501143320ff",
        "the digest must bind the exact failure message, not only its length"
    );
}

#[test]
fn runner_ignores_comments_and_unrelated_strings_that_name_a_bats_suite() {
    let root = bats_registration_root(
        "source-spoof",
        &["orphan.bats"],
        "// not a registration: \"tests/unit/orphan.bats\"\n\
         const UNRELATED: &str = \"tests/unit/orphan.bats\";\n",
    );

    let report = run_bats_registration(&root);

    assert_eq!(
        report.results[0].exit_code,
        Some(1),
        "only a typed catalog owner may register a suite"
    );
}

#[test]
fn runner_accepts_bats_suites_with_typed_catalog_owners() {
    let root = bats_registration_root(
        "registered-only",
        &["test_quality_gate_discovery.bats"],
        "ExternalCheck::BatsSuite(\"tests/unit/test_quality_gate_discovery.bats\")\n",
    );

    let report = run_bats_registration(&root);

    assert_eq!(
        report.results[0].exit_code,
        Some(0),
        "a suite with a typed owner in the validation catalog is registered"
    );
}

#[test]
fn runner_fails_closed_when_a_suite_inventory_directory_cannot_be_read() {
    let root = bats_registration_root("unreadable-inventory", &[], "");
    fs::remove_dir(root.join("tests/lint")).expect("empty lint inventory directory removed");
    fs::write(root.join("tests/lint"), "not a directory\n")
        .expect("unreadable inventory path fixture");

    let report = run_bats_registration(&root);

    assert_eq!(
        report.results[0].exit_code,
        Some(1),
        "required suite inventory I/O errors must fail validation"
    );
    assert!(
        report.results[0].stderr_bytes > "tests/lint".len(),
        "the failure evidence must identify the unreadable inventory path"
    );
}

/// The real repository, not a fixture: this is the check's whole point, and it is
/// what caught `tests/lint/test_bats_negation_checker.bats` and
/// `tests/unit/test_quality_gate_discovery.bats` before they were wired up.
#[test]
fn every_unbaselined_bats_suite_in_this_repository_is_registered() {
    let report = run_bats_registration(&repository_root());

    assert_eq!(
        report.results[0].exit_code,
        Some(0),
        "a bats suite under tests/unit or tests/lint is invoked by no validate check; \
         register it, or add it to BATS_REGISTRATION_BASELINE if it is not a suite"
    );
}

/// The registration is only worth anything if the runner can actually execute the
/// suites it now owns. Registry-red while standalone-green is the defect #3360
/// describes, mirrored.
#[test]
fn runner_executes_the_newly_registered_bats_suites() {
    for (id, suite) in [
        ("check_bats_negation_ratchet", "tests/lint/test_bats_negation_checker.bats"),
        ("check_autospec_fleet_enabled_false", "tests/unit/test_autospec_fleet_enabled_false.bats"),
        ("check_autospec_sweep_enabled_false", "tests/unit/test_autospec_sweep_enabled_false.bats"),
        ("check_classify_lang_labels", "tests/unit/test_classify_lang_labels.bats"),
        ("check_classify_language", "tests/unit/test_classify_language.bats"),
        ("check_define_phase0_language", "tests/unit/test_define_phase0_language.bats"),
        ("check_language_axis_integration", "tests/unit/test_language_axis_integration.bats"),
        ("check_language_table", "tests/unit/test_language_table.bats"),
        ("check_proxy_direct_borrow_lifetime", "tests/unit/proxy-direct-borrow-lifetime.bats"),
        ("check_qa_function_ranges_string_literals", "tests/unit/qa-function-ranges-string-literals.bats"),
        // Registered by #3535: the loud-failure verification gates.
        ("check_verify_gate", "tests/unit/test_verify_gate.bats"),
        ("check_verify_produced_work", "tests/unit/test_verify_produced_work.bats"),
    ] {
        let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::TopLevel,
            owner: CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(suite)),
        }]);

        let report = ValidationRunner::run(&catalog, &repository_root());

        assert_eq!(
            report.results[0].exit_code,
            Some(0),
            "{id} must pass when the runner invokes {suite}, not only when bats is run by hand"
        );
    }
}

fn run_bats_registration(root: &std::path::Path) -> ValidationExecutionReport {
    let catalog = ValidationCatalog::from_checks(vec![ValidationCheck {
        id: "check_bats_suite_registration",
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::ExternalBatch(ExternalCheck::BatsSuiteRegistration),
    }]);
    ValidationRunner::run(&catalog, root)
}

/// Builds a throwaway autospec repository marker plus whichever suite files and
/// irrelevant validation source text a test needs.
fn bats_registration_root(name: &str, suites: &[&str], validation_source: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "autospec-bats-registration-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let sources = root.join("crates/autospec-core/src/validation");
    fs::create_dir_all(&sources).expect("validation source directory");
    fs::write(sources.join("catalog.rs"), validation_source).expect("validation source fixture");
    let unit = root.join("tests/unit");
    fs::create_dir_all(&unit).expect("unit suite directory");
    fs::create_dir_all(root.join("tests/lint")).expect("lint suite directory");
    for suite in suites {
        fs::write(unit.join(suite), "@test \"placeholder\" { true; }\n").expect("suite file");
    }
    root
}

fn validation_fixture(name: &str) -> PathBuf {
    repository_root()
        .join("crates/autospec-cli/tests/fixtures/validation-cutover")
        .join(name)
}
