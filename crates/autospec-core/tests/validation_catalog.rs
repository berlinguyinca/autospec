use std::collections::BTreeSet;

use autospec_core::validation::{
    CheckModes, CheckOwner, CheckReachability, ExternalCheck, StructuralCheck, ValidationCatalog,
    ValidationCheck,
};

const FROZEN_CATALOG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../autospec-cli/tests/fixtures/validation-cutover/catalog-v1.json"
));

fn frozen_catalog_ids() -> Vec<&'static str> {
    FROZEN_CATALOG
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_suffix(',')
                .unwrap_or(line.trim())
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
        })
        .collect()
}

#[test]
fn catalog_has_one_owner_slot_for_every_frozen_gate() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(catalog.ids(), frozen_catalog_ids());
    assert!(catalog.validate().is_ok());
}

#[test]
fn catalog_records_legacy_execution_reachability_without_expanding_it() {
    let catalog = ValidationCatalog::standard();
    let calls = catalog.legacy_top_level_calls();

    assert_eq!(calls.len(), 144); // +1: check_reference_pointer_integrity (#3158)
    assert_eq!(calls.iter().copied().collect::<BTreeSet<_>>().len(), 139); // a call no gate repeats
    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_lockstep")
            .map(|check| check.reachability),
        Some(CheckReachability::InternalComponent)
    );
    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_architecture_fitness_engine")
            .map(|check| check.reachability),
        Some(CheckReachability::LegacyUnreachable)
    );
    assert!(calls.contains(&"check_bash_syntax"));
    assert_eq!(
        calls
            .iter()
            .filter(|&&id| id == "check_bash_syntax")
            .count(),
        2
    );
}

#[test]
fn frozen_catalog_contains_every_named_shell_gate() {
    assert_eq!(frozen_catalog_ids().len(), 155);
}

#[test]
fn frozen_catalog_keeps_the_flag_sentinel_docs_gate_in_declaration_order() {
    let ids = frozen_catalog_ids();

    assert_eq!(ids.len(), 155);
    assert_eq!(ids[5], "check_flag_sentinel_docs");
}

#[test]
fn catalog_assigns_flag_sentinel_docs_to_a_rust_owner() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_flag_sentinel_docs")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(StructuralCheck::FlagSentinelDocs))
    );
}

#[test]
fn catalog_assigns_per_skill_model_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_subagent_model_tier",
            StructuralCheck::SubagentModelTier,
        ),
        (
            "check_harness_detection_block",
            StructuralCheck::HarnessDetection,
        ),
        (
            "check_monitor_batch_exit",
            StructuralCheck::MonitorBatchExit,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_grooming_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::GroomingContract)),
        "check_grooming_contract must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_mutation_and_negative_path")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::MutationAndNegativePath
        )),
        "check_mutation_and_negative_path must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_lint_implementation_helpers")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::LintImplementationHelpers
        )),
        "check_lint_implementation_helpers must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_lint_issue_helpers")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::LintIssueHelpers)),
        "check_lint_issue_helpers must have a typed external owner"
    );

    for (id, owner) in [
        (
            "check_phase4_ci_status_compare",
            ExternalCheck::Phase4CiStatusCompare,
        ),
        (
            "check_define_spec_worktree_routing",
            ExternalCheck::DefineSpecWorktreeRouting,
        ),
        (
            "check_run_groom_preflight_contract",
            ExternalCheck::RunGroomPreflightContract,
        ),
        ("check_grow_run_contract", ExternalCheck::GrowRunContract),
        (
            "check_performance_workstream_contract",
            ExternalCheck::PerformanceWorkstream,
        ),
        (
            "check_ux_ui_workstream_contract",
            ExternalCheck::UxUiWorkstream,
        ),
        (
            "check_token_baseline_fresh",
            ExternalCheck::TokenBaselineFresh,
        ),
        (
            "check_architecture_fitness_engine",
            ExternalCheck::ArchitectureFitnessEngine,
        ),
        ("check_phase4_tests", ExternalCheck::Phase4TestSuites),
        (
            "check_validation_matrix_smoke",
            ExternalCheck::ValidationMatrixSmoke,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_repository_presence_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_agents_md_subagent_section",
            StructuralCheck::AgentsMdSubagentSection,
        ),
        (
            "check_agents_md_subagent_matrix",
            StructuralCheck::AgentsMdSubagentMatrix,
        ),
        (
            "check_autospec_listen_files",
            StructuralCheck::AutospecListenFiles,
        ),
        ("check_examples_dir", StructuralCheck::ExamplesDirectory),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_documentation_and_skill_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_governance_headings",
            StructuralCheck::GovernanceHeadings,
        ),
        (
            "check_autospec_stl_design_guardrails",
            StructuralCheck::StlDesignGuardrails,
        ),
        (
            "check_existing_spec_mode",
            StructuralCheck::ExistingSpecMode,
        ),
        (
            "check_docs_amendment_presence",
            StructuralCheck::DocsAmendmentPresence,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_autospec_review_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_autospec_review_skill_present",
            StructuralCheck::AutospecReviewSkill,
        ),
        (
            "check_autospec_review_tier_a_directives",
            StructuralCheck::AutospecReviewTierADirectives,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_autospec_run_review_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_autospec_run_priority_sort_lockstep",
            StructuralCheck::AutospecRunPrioritySortLockstep,
        ),
        (
            "check_autospec_run_regression_review_lockstep",
            StructuralCheck::AutospecRunRegressionReviewLockstep,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_bounded_context_and_fleet_gui_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_phase1_bounded_context_contract",
            StructuralCheck::Phase1BoundedContext,
        ),
        (
            "check_fleet_gui_subcommand_lockstep",
            StructuralCheck::FleetGuiSubcommandLockstep,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_fleet_scripts_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_fleet_scripts")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::FleetScripts))
    );
}

#[test]
fn catalog_assigns_generated_yaml_parse_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_generated_yaml_parse")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::GeneratedYamlParse
        ))
    );
}

#[test]
fn catalog_assigns_autospec_sweep_config_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_sweep_config_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecSweepConfig
        ))
    );
}

#[test]
fn catalog_assigns_agents_git_hygiene_to_a_rust_owner() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_agents_md_git_hygiene")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(StructuralCheck::AgentsMdGitHygiene))
    );
}

#[test]
fn catalog_assigns_palette_single_source_to_a_rust_owner() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_palette_single_source")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(
            StructuralCheck::PaletteSingleSource
        ))
    );
}

#[test]
fn catalog_assigns_static_documentation_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_mermaid_documentation_contract",
            StructuralCheck::MermaidDocumentation,
        ),
        (
            "check_qa_documentation_gate",
            StructuralCheck::QaDocumentationGate,
        ),
        (
            "check_autospec_harmonize_contract",
            StructuralCheck::AutospecHarmonize,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_autonomous_and_team_policy_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_autospec_autonomous_skill_contract",
            StructuralCheck::AutospecAutonomousSkill,
        ),
        (
            "check_autospec_explore_userspace_roster_contract",
            StructuralCheck::AutospecExploreUserspaceRoster,
        ),
        (
            "check_autospec_explore_parallel_validation_contract",
            StructuralCheck::AutospecExploreParallelValidation,
        ),
        (
            "check_autospec_autonomous_tier4_discovery_contract",
            StructuralCheck::AutospecAutonomousTier4Discovery,
        ),
        (
            "check_team_personality_selection_contract",
            StructuralCheck::TeamPersonalitySelection,
        ),
        (
            "check_team_personality_issue_template_contract",
            StructuralCheck::TeamPersonalityIssueTemplate,
        ),
        (
            "check_team_personality_phase4_and_docs_contract",
            StructuralCheck::TeamPersonalityPhase4AndDocs,
        ),
        (
            "check_team_personality_contract",
            StructuralCheck::TeamPersonality,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_phase4_static_policy_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        ("check_closeout_contract", StructuralCheck::CloseoutContract),
        (
            "check_phase4_guardian_block_lockstep",
            StructuralCheck::Phase4GuardianBlockLockstep,
        ),
        (
            "check_phase4_issue_start_summary",
            StructuralCheck::Phase4IssueStartSummary,
        ),
        (
            "check_phase4_immediate_next_issue_pickup",
            StructuralCheck::Phase4ImmediateNextIssuePickup,
        ),
        (
            "check_autospec_run_continuation_contract",
            StructuralCheck::AutospecRunContinuation,
        ),
        (
            "check_autospec_run_codex_bounded_handoff",
            StructuralCheck::AutospecRunCodexBoundedHandoff,
        ),
        (
            "check_phase4_adaptive_retry",
            StructuralCheck::Phase4AdaptiveRetry,
        ),
        (
            "check_phase4_full_test_suite_gate",
            StructuralCheck::Phase4FullTestSuite,
        ),
        (
            "check_data_scope_review_lens",
            StructuralCheck::DataScopeReviewLens,
        ),
        (
            "check_phase4_cost_epic_parity_lockstep",
            StructuralCheck::Phase4CostEpicParity,
        ),
        (
            "check_docs_drift_gate_regen_conditional_parity",
            StructuralCheck::DocsDriftGateRegenConditionalParity,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_release_and_qa_verdict_contracts_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_autospec_release_contract",
            StructuralCheck::AutospecReleaseContract,
        ),
        (
            "check_qa_verdict_contract",
            StructuralCheck::QaVerdictContract,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::RustNative(owner)),
            "{id} must have a direct Rust owner"
        );
    }
}

#[test]
fn catalog_assigns_release_support_gates_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();
    let (id, owner) = (
        "check_release_verdict_script",
        ExternalCheck::ReleaseVerdictScript,
    );
    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == id)
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(owner)),
        "{id} must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_brute_force_rule_ids")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(StructuralCheck::BruteForceRuleIds)),
        "check_brute_force_rule_ids must have a direct Rust owner"
    );

    for (id, suite) in [
        (
            "check_lint_heredoc_handling",
            "tests/lint/test_complexity_heredoc.bats",
        ),
        (
            "check_lint_reuse_triage",
            "tests/lint/test_reuse_triage.bats",
        ),
        ("check_ship_completeness", "tests/ship-completeness.bats"),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(suite))),
            "{id} must have a typed Bats owner"
        );
    }

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_reviewer_reuse_lens")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::ReviewerReuseLens)),
        "check_reviewer_reuse_lens must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_usage_limit_helper")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::BashHelpUsage(
            "scripts/autospec-usage-limit.sh"
        ))),
        "check_usage_limit_helper must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_supersession_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::SupersessionContract
        )),
        "check_supersession_contract must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_run_summary_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::RunSummaryContract
        )),
        "check_autospec_run_summary_contract must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_db_module_install")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::DbModuleInstall)),
        "check_db_module_install must have a typed external owner"
    );

    for (id, directory) in [
        ("check_autonomous_phase2_suite", "tests/autonomous"),
        ("check_persona_suite", "tests/persona"),
        ("check_reuse_lens_suite", "tests/reuse-lens"),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(ExternalCheck::BatsDirectory(
                directory
            ))),
            "{id} must have a typed Bats directory owner"
        );
    }

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_upgrade_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecUpgradeContract
        )),
        "check_autospec_upgrade_contract must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_sweep_area_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecSweepAreaContract
        )),
        "check_autospec_sweep_area_contract must have a typed external owner"
    );

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_fab_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecFabContract
        )),
        "check_autospec_fab_contract must have a typed external owner"
    );

    for (id, owner) in [
        (
            "check_autospec_test_skill_present",
            ExternalCheck::AutospecTestSkill,
        ),
        (
            "check_autospec_playwright_skill_present",
            ExternalCheck::AutospecPlaywrightSkill,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_claim_guard_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_claim_guard_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::ClaimGuardContract
        )),
        "check_claim_guard_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_qa_and_loop_support_contracts_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_autospec_qa_cluster_contract",
            ExternalCheck::AutospecQaClusterContract,
        ),
        (
            "check_autospec_qa_bug_class_contract",
            ExternalCheck::AutospecQaBugClassContract,
        ),
        (
            "check_loop_handoff_harness_awareness",
            ExternalCheck::LoopHandoffHarnessAwareness,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_claim_cas_guard_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_claim_cas_guard")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::ClaimCasGuard)),
        "check_claim_cas_guard must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_watchdog_gc_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_watchdog_worktree_gc")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::WatchdogWorktreeGc
        )),
        "check_watchdog_worktree_gc must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_block_expansion_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_block_expansion")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::BlockExpansion)),
        "check_block_expansion must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_implementer_base_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_explore_implementer_base")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecExploreImplementerBase
        )),
        "check_autospec_explore_implementer_base must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_researcher_contracts_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_autospec_explore_researchers_deterministic",
            ExternalCheck::AutospecExploreResearchersDeterministic,
        ),
        (
            "check_autospec_explore_researchers_llm",
            ExternalCheck::AutospecExploreResearchersLlm,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_explore_specialist_discovery_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_explore_specialists_discovery")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecExploreSpecialistsDiscovery
        )),
        "check_autospec_explore_specialists_discovery must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_stage2_intersect_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_explore_stage2_intersect_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecExploreStage2Intersect
        )),
        "check_autospec_explore_stage2_intersect_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_worktree_assert_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_explore_trio_worktree_assert")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::ExploreTrioWorktreeAssert
        )),
        "check_explore_trio_worktree_assert must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_spec_first_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_explore_spec_first_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecExploreSpecFirst
        )),
        "check_autospec_explore_spec_first_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_qa_gate_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_explore_qa_gate_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecExploreQaGate
        )),
        "check_autospec_explore_qa_gate_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_style_normalization_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_explore_style_normalization_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecExploreStyleNormalization
        )),
        "check_autospec_explore_style_normalization_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_orchestrator_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_explore_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecExploreOrchestrator
        )),
        "check_autospec_explore_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_explore_discovery_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_explore_discovery_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecExploreDiscovery
        )),
        "check_autospec_explore_discovery_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_qa_root_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_qa_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecQaContract
        )),
        "check_autospec_qa_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_qa_deployment_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_qa_deploy_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::QaDeployContract)),
        "check_qa_deploy_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_qa_verify_first_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_qa_verify_first_discipline")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::QaVerifyFirstDiscipline
        )),
        "check_qa_verify_first_discipline must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_qa_exhaustiveness_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_qa_exhaustiveness_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::QaExhaustivenessContract
        )),
        "check_qa_exhaustiveness_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_qa_incident_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_qa_incident_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::QaIncidentContract
        )),
        "check_qa_incident_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_qa_heal_loop_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_qa_heal_loop_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::QaHealLoopContract
        )),
        "check_qa_heal_loop_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_quality_differential_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_quality_differential")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::QualityDifferential
        )),
        "check_quality_differential must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_release_area_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_release_area_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::ReleaseAreaContract
        )),
        "check_autospec_release_area_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_release_worktree_assert_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_release_trio_worktree_assert")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::ReleaseWorktreeAssert
        )),
        "check_release_trio_worktree_assert must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_fab_container_pin_lint_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_fab_container_dockerfile")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::FabContainerPinLint
        )),
        "check_fab_container_dockerfile must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_repo_quality_audit_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_repo_quality_audit_loop")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::RepoQualityAudit)),
        "check_repo_quality_audit_loop must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_autonomous_mode_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_autonomous_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecAutonomousContract
        )),
        "check_autospec_autonomous_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_dogfood_detectors_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_dogfood_detectors")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::DogfoodDetectors)),
        "check_dogfood_detectors must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_parallel_dispatch_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_parallel_dispatch_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::AutospecParallelDispatch
        )),
        "check_autospec_parallel_dispatch_contract must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_growth_and_telemetry_contracts_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        ("check_growth_shared_contract", ExternalCheck::GrowthShared),
        (
            "check_growth_candidate_pipeline_contract",
            ExternalCheck::GrowthCandidatePipeline,
        ),
        (
            "check_grow_run_pipeline_contract",
            ExternalCheck::GrowRunPipeline,
        ),
        ("check_db_telemetry_contract", ExternalCheck::DbTelemetry),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_worktree_ladder_parity_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_worktree_ladder_assert_parity")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::WorktreeLadderAssertParity
        )),
        "check_worktree_ladder_assert_parity must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_phase4_policy_gates_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_phase4_single_agent_discipline",
            ExternalCheck::Phase4SingleAgentDiscipline,
        ),
        (
            "check_phase4_final_quality_gate",
            ExternalCheck::Phase4FinalQualityGate,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_refine_continue_and_loop_contracts_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_autospec_refine_contract",
            ExternalCheck::AutospecRefineContract,
        ),
        (
            "check_autospec_continue_contract",
            ExternalCheck::AutospecContinueContract,
        ),
        (
            "check_autospec_loop_contract",
            ExternalCheck::AutospecLoopContract,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_resume_contract_components_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_autospec_resume_structure",
            ExternalCheck::AutospecResumeStructure,
        ),
        (
            "check_autospec_supervisor_structure",
            ExternalCheck::AutospecSupervisorStructure,
        ),
        (
            "check_autospec_resume_contract",
            ExternalCheck::AutospecResumeContract,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_prompt_contracts_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_implementer_contract",
            ExternalCheck::ImplementerContract,
        ),
        ("check_reviewer_contract", ExternalCheck::ReviewerContract),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_autonomy_wiring_contracts_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_conductor_wiring_contract",
            ExternalCheck::ConductorWiringContract,
        ),
        (
            "check_autonomy_guardrails_foundation",
            ExternalCheck::AutonomyGuardrailsFoundation,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_python_suites_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_python_suites")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::PythonSuites)),
        "check_python_suites must have a typed external owner"
    );
}

#[test]
fn catalog_assigns_growth_and_documentation_contracts_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_grow_define_contract",
            ExternalCheck::GrowDefineContract,
        ),
        (
            "check_autospec_doc_contract",
            ExternalCheck::AutospecDocContract,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn catalog_assigns_final_legacy_contracts_to_typed_external_batches() {
    let catalog = ValidationCatalog::standard();

    for (id, owner) in [
        (
            "check_constitution_validation_contract",
            ExternalCheck::ConstitutionValidation,
        ),
        ("check_install_tests", ExternalCheck::InstallTests),
        (
            "check_control_plane_bootstrap_contract",
            ExternalCheck::ControlPlaneBootstrap,
        ),
    ] {
        assert_eq!(
            catalog
                .checks()
                .iter()
                .find(|check| check.id == id)
                .map(|check| &check.owner),
            Some(&CheckOwner::ExternalBatch(owner)),
            "{id} must have a typed external owner"
        );
    }
}

#[test]
fn standard_catalog_owner_mapping_is_total() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(catalog.checks().len(), catalog.ids().len());
    catalog
        .validate()
        .expect("every frozen catalog check has a direct owner");
}

#[test]
fn catalog_rejects_empty_and_duplicate_ids() {
    let entry = |id| ValidationCheck {
        id,
        required: true,
        independent: false,
        modes: CheckModes::CatalogSlot,
        reachability: CheckReachability::TopLevel,
        owner: CheckOwner::RustNative(StructuralCheck::TrioLockstep),
    };
    let empty = ValidationCatalog::from_checks(vec![entry("")]);
    let duplicate = ValidationCatalog::from_checks(vec![entry("check_once"), entry("check_once")]);

    assert!(empty.validate().is_err());
    assert!(duplicate.validate().is_err());
}

#[test]
fn catalog_assigns_self_update_gates_to_rust_owners() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_self_update")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(StructuralCheck::SelfUpdateTrio))
    );
    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_self_update_duo")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(StructuralCheck::SelfUpdateDuo))
    );
}

#[test]
fn catalog_assigns_keyword_routing_to_a_rust_owner() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_keyword_routing_section")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(StructuralCheck::KeywordRouting))
    );
}

#[test]
fn catalog_assigns_codex_skills_install_to_a_rust_owner() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_codex_skills_install")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(StructuralCheck::CodexSkillsInstall))
    );
}

#[test]
fn catalog_assigns_shared_script_install_to_a_rust_owner() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_shared_script_install")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(
            StructuralCheck::SharedScriptInstall
        ))
    );
}

#[test]
fn catalog_assigns_root_helper_wrapper_policy_to_a_rust_owner() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_root_helper_wrapper_policy")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(
            StructuralCheck::RootHelperWrapperPolicy
        ))
    );
}

#[test]
fn catalog_assigns_startup_preflight_to_a_rust_owner() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_startup_preflight")
            .map(|check| &check.owner),
        Some(&CheckOwner::RustNative(StructuralCheck::StartupPreflight))
    );
}

#[test]
fn catalog_assigns_derive_trio_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_derive_trio_consistency")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(
            ExternalCheck::DeriveTrioConsistency
        ))
    );
}

#[test]
fn catalog_assigns_bash_syntax_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_bash_syntax")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::BashSyntax))
    );
}

#[test]
fn catalog_assigns_frontmatter_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_frontmatter")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::Frontmatter))
    );
}

#[test]
fn catalog_assigns_gap_miner_contract_to_a_typed_external_batch() {
    let catalog = ValidationCatalog::standard();

    assert_eq!(
        catalog
            .checks()
            .iter()
            .find(|check| check.id == "check_autospec_gap_miner_contract")
            .map(|check| &check.owner),
        Some(&CheckOwner::ExternalBatch(ExternalCheck::GapMinerContract))
    );
}
