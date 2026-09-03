mod catalog_ids;

use std::collections::BTreeSet;

use super::command::ToolCommand;
use super::external::ExternalCheck;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationCheck {
    pub id: &'static str,
    pub required: bool,
    pub independent: bool,
    pub modes: CheckModes,
    pub reachability: CheckReachability,
    pub owner: CheckOwner,
}

impl ValidationCheck {
    pub fn catalog_entry(id: &'static str) -> Self {
        let owner = match id {
            "check_lockstep" => CheckOwner::RustNative(StructuralCheck::TrioLockstep),
            "check_lockstep_duo" => CheckOwner::RustNative(StructuralCheck::DuoLockstep),
            "check_bash_syntax" => CheckOwner::ExternalBatch(ExternalCheck::BashSyntax),
            "check_frontmatter" => CheckOwner::ExternalBatch(ExternalCheck::Frontmatter),
            "check_required_files" => CheckOwner::RustNative(StructuralCheck::RequiredTrioFiles),
            "check_flag_sentinel_docs" => CheckOwner::RustNative(StructuralCheck::FlagSentinelDocs),
            "check_subagent_model_tier" => {
                CheckOwner::RustNative(StructuralCheck::SubagentModelTier)
            }
            "check_harness_detection_block" => {
                CheckOwner::RustNative(StructuralCheck::HarnessDetection)
            }
            "check_monitor_batch_exit" => CheckOwner::RustNative(StructuralCheck::MonitorBatchExit),
            "check_agents_md_subagent_section" => {
                CheckOwner::RustNative(StructuralCheck::AgentsMdSubagentSection)
            }
            "check_agents_md_subagent_matrix" => {
                CheckOwner::RustNative(StructuralCheck::AgentsMdSubagentMatrix)
            }
            "check_autospec_listen_files" => {
                CheckOwner::RustNative(StructuralCheck::AutospecListenFiles)
            }
            "check_examples_dir" => CheckOwner::RustNative(StructuralCheck::ExamplesDirectory),
            "check_governance_headings" => {
                CheckOwner::RustNative(StructuralCheck::GovernanceHeadings)
            }
            "check_autospec_stl_design_guardrails" => {
                CheckOwner::RustNative(StructuralCheck::StlDesignGuardrails)
            }
            "check_existing_spec_mode" => CheckOwner::RustNative(StructuralCheck::ExistingSpecMode),
            "check_docs_amendment_presence" => {
                CheckOwner::RustNative(StructuralCheck::DocsAmendmentPresence)
            }
            "check_autospec_review_skill_present" => {
                CheckOwner::RustNative(StructuralCheck::AutospecReviewSkill)
            }
            "check_autospec_review_tier_a_directives" => {
                CheckOwner::RustNative(StructuralCheck::AutospecReviewTierADirectives)
            }
            "check_autospec_run_priority_sort_lockstep" => {
                CheckOwner::RustNative(StructuralCheck::AutospecRunPrioritySortLockstep)
            }
            "check_autospec_run_regression_review_lockstep" => {
                CheckOwner::RustNative(StructuralCheck::AutospecRunRegressionReviewLockstep)
            }
            "check_phase1_bounded_context_contract" => {
                CheckOwner::RustNative(StructuralCheck::Phase1BoundedContext)
            }
            "check_fleet_gui_subcommand_lockstep" => {
                CheckOwner::RustNative(StructuralCheck::FleetGuiSubcommandLockstep)
            }
            "check_autospec_fleet_scripts" => {
                CheckOwner::ExternalBatch(ExternalCheck::FleetScripts)
            }
            "check_generated_yaml_parse" => {
                CheckOwner::ExternalBatch(ExternalCheck::GeneratedYamlParse)
            }
            "check_autospec_sweep_config_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecSweepConfig)
            }
            "check_autospec_sweep_area_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecSweepAreaContract)
            }
            "check_autospec_qa_cluster_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecQaClusterContract)
            }
            "check_autospec_qa_bug_class_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecQaBugClassContract)
            }
            "check_loop_handoff_harness_awareness" => {
                CheckOwner::ExternalBatch(ExternalCheck::LoopHandoffHarnessAwareness)
            }
            "check_agents_md_git_hygiene" => {
                CheckOwner::RustNative(StructuralCheck::AgentsMdGitHygiene)
            }
            "check_palette_single_source" => {
                CheckOwner::RustNative(StructuralCheck::PaletteSingleSource)
            }
            "check_mermaid_documentation_contract" => {
                CheckOwner::RustNative(StructuralCheck::MermaidDocumentation)
            }
            "check_qa_documentation_gate" => {
                CheckOwner::RustNative(StructuralCheck::QaDocumentationGate)
            }
            "check_autospec_harmonize_contract" => {
                CheckOwner::RustNative(StructuralCheck::AutospecHarmonize)
            }
            "check_autospec_autonomous_skill_contract" => {
                CheckOwner::RustNative(StructuralCheck::AutospecAutonomousSkill)
            }
            "check_autospec_explore_userspace_roster_contract" => {
                CheckOwner::RustNative(StructuralCheck::AutospecExploreUserspaceRoster)
            }
            "check_autospec_explore_parallel_validation_contract" => {
                CheckOwner::RustNative(StructuralCheck::AutospecExploreParallelValidation)
            }
            "check_autospec_autonomous_tier4_discovery_contract" => {
                CheckOwner::RustNative(StructuralCheck::AutospecAutonomousTier4Discovery)
            }
            "check_team_personality_selection_contract" => {
                CheckOwner::RustNative(StructuralCheck::TeamPersonalitySelection)
            }
            "check_team_personality_issue_template_contract" => {
                CheckOwner::RustNative(StructuralCheck::TeamPersonalityIssueTemplate)
            }
            "check_team_personality_phase4_and_docs_contract" => {
                CheckOwner::RustNative(StructuralCheck::TeamPersonalityPhase4AndDocs)
            }
            "check_team_personality_contract" => {
                CheckOwner::RustNative(StructuralCheck::TeamPersonality)
            }
            "check_autospec_release_contract" => {
                CheckOwner::RustNative(StructuralCheck::AutospecReleaseContract)
            }
            "check_qa_verdict_contract" => {
                CheckOwner::RustNative(StructuralCheck::QaVerdictContract)
            }
            "check_release_verdict_script" => {
                CheckOwner::ExternalBatch(ExternalCheck::ReleaseVerdictScript)
            }
            "check_brute_force_rule_ids" => {
                CheckOwner::RustNative(StructuralCheck::BruteForceRuleIds)
            }
            "check_lint_heredoc_handling" => CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(
                "tests/lint/test_complexity_heredoc.bats",
            )),
            "check_lint_reuse_triage" => CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(
                "tests/lint/test_reuse_triage.bats",
            )),
            "check_bats_suite_registration" => {
                CheckOwner::ExternalBatch(ExternalCheck::BatsSuiteRegistration)
            }
            "check_bats_negation_ratchet" => CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(
                "tests/lint/test_bats_negation_checker.bats",
            )),
            "check_code_intelligence_contract" => CheckOwner::ExternalBatch(
                ExternalCheck::BatsSuite("tests/code-intel/code-intelligence.bats"),
            ),
            "check_autospec_fleet_enabled_false" => CheckOwner::ExternalBatch(
                ExternalCheck::BatsSuite("tests/unit/test_autospec_fleet_enabled_false.bats"),
            ),
            "check_autospec_sweep_enabled_false" => CheckOwner::ExternalBatch(
                ExternalCheck::BatsSuite("tests/unit/test_autospec_sweep_enabled_false.bats"),
            ),
            "check_classify_lang_labels" => CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(
                "tests/unit/test_classify_lang_labels.bats",
            )),
            "check_classify_language" => CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(
                "tests/unit/test_classify_language.bats",
            )),
            "check_define_phase0_language" => CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(
                "tests/unit/test_define_phase0_language.bats",
            )),
            "check_language_axis_integration" => CheckOwner::ExternalBatch(
                ExternalCheck::BatsSuite("tests/unit/test_language_axis_integration.bats"),
            ),
            "check_language_table" => CheckOwner::ExternalBatch(ExternalCheck::BatsSuite(
                "tests/unit/test_language_table.bats",
            )),
            "check_ship_completeness" => {
                CheckOwner::ExternalBatch(ExternalCheck::BatsSuite("tests/ship-completeness.bats"))
            }
            "check_usage_limit_helper" => CheckOwner::ExternalBatch(ExternalCheck::BashHelpUsage(
                "scripts/autospec-usage-limit.sh",
            )),
            "check_supersession_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::SupersessionContract)
            }
            "check_autospec_run_summary_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::RunSummaryContract)
            }
            "check_db_module_install" => CheckOwner::ExternalBatch(ExternalCheck::DbModuleInstall),
            "check_autonomous_phase2_suite" => {
                CheckOwner::ExternalBatch(ExternalCheck::BatsDirectory("tests/autonomous"))
            }
            "check_persona_suite" => {
                CheckOwner::ExternalBatch(ExternalCheck::BatsDirectory("tests/persona"))
            }
            "check_reuse_lens_suite" => {
                CheckOwner::ExternalBatch(ExternalCheck::BatsDirectory("tests/reuse-lens"))
            }
            "check_autospec_upgrade_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecUpgradeContract)
            }
            "check_claim_guard_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::ClaimGuardContract)
            }
            "check_claim_cas_guard" => CheckOwner::ExternalBatch(ExternalCheck::ClaimCasGuard),
            "check_watchdog_worktree_gc" => {
                CheckOwner::ExternalBatch(ExternalCheck::WatchdogWorktreeGc)
            }
            "check_block_expansion" => CheckOwner::ExternalBatch(ExternalCheck::BlockExpansion),
            "check_autospec_explore_implementer_base" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreImplementerBase)
            }
            "check_autospec_explore_researchers_deterministic" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreResearchersDeterministic)
            }
            "check_autospec_explore_researchers_llm" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreResearchersLlm)
            }
            "check_autospec_explore_specialists_discovery" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreSpecialistsDiscovery)
            }
            "check_autospec_explore_stage2_intersect_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreStage2Intersect)
            }
            "check_explore_trio_worktree_assert" => {
                CheckOwner::ExternalBatch(ExternalCheck::ExploreTrioWorktreeAssert)
            }
            "check_autospec_explore_spec_first_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreSpecFirst)
            }
            "check_autospec_explore_qa_gate_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreQaGate)
            }
            "check_autospec_explore_style_normalization_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreStyleNormalization)
            }
            "check_autospec_explore_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreOrchestrator)
            }
            "check_autospec_explore_discovery_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecExploreDiscovery)
            }
            "check_autospec_qa_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecQaContract)
            }
            "check_qa_deploy_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::QaDeployContract)
            }
            "check_qa_verify_first_discipline" => {
                CheckOwner::ExternalBatch(ExternalCheck::QaVerifyFirstDiscipline)
            }
            "check_qa_exhaustiveness_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::QaExhaustivenessContract)
            }
            "check_qa_incident_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::QaIncidentContract)
            }
            "check_qa_heal_loop_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::QaHealLoopContract)
            }
            "check_quality_differential" => {
                CheckOwner::ExternalBatch(ExternalCheck::QualityDifferential)
            }
            "check_autospec_release_area_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::ReleaseAreaContract)
            }
            "check_release_trio_worktree_assert" => {
                CheckOwner::ExternalBatch(ExternalCheck::ReleaseWorktreeAssert)
            }
            "check_fab_container_dockerfile" => {
                CheckOwner::ExternalBatch(ExternalCheck::FabContainerPinLint)
            }
            "check_repo_quality_audit_loop" => {
                CheckOwner::ExternalBatch(ExternalCheck::RepoQualityAudit)
            }
            "check_autospec_autonomous_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecAutonomousContract)
            }
            "check_dogfood_detectors" => CheckOwner::ExternalBatch(ExternalCheck::DogfoodDetectors),
            "check_autospec_parallel_dispatch_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecParallelDispatch)
            }
            "check_growth_shared_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::GrowthShared)
            }
            "check_growth_candidate_pipeline_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::GrowthCandidatePipeline)
            }
            "check_grow_run_pipeline_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::GrowRunPipeline)
            }
            "check_db_telemetry_contract" => CheckOwner::ExternalBatch(ExternalCheck::DbTelemetry),
            "check_worktree_ladder_assert_parity" => {
                CheckOwner::ExternalBatch(ExternalCheck::WorktreeLadderAssertParity)
            }
            "check_phase4_single_agent_discipline" => {
                CheckOwner::ExternalBatch(ExternalCheck::Phase4SingleAgentDiscipline)
            }
            "check_phase4_final_quality_gate" => {
                CheckOwner::ExternalBatch(ExternalCheck::Phase4FinalQualityGate)
            }
            "check_autospec_refine_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecRefineContract)
            }
            "check_autospec_continue_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecContinueContract)
            }
            "check_autospec_loop_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecLoopContract)
            }
            "check_autospec_resume_structure" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecResumeStructure)
            }
            "check_autospec_supervisor_structure" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecSupervisorStructure)
            }
            "check_autospec_resume_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecResumeContract)
            }
            "check_implementer_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::ImplementerContract)
            }
            "check_reviewer_contract" => CheckOwner::ExternalBatch(ExternalCheck::ReviewerContract),
            "check_conductor_wiring_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::ConductorWiringContract)
            }
            "check_autonomy_guardrails_foundation" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutonomyGuardrailsFoundation)
            }
            "check_python_suites" => CheckOwner::ExternalBatch(ExternalCheck::PythonSuites),
            "check_autospec_test_skill_present" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecTestSkill)
            }
            "check_autospec_playwright_skill_present" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecPlaywrightSkill)
            }
            "check_autospec_fab_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecFabContract)
            }
            "check_grooming_contract" => CheckOwner::ExternalBatch(ExternalCheck::GroomingContract),
            "check_mutation_and_negative_path" => {
                CheckOwner::ExternalBatch(ExternalCheck::MutationAndNegativePath)
            }
            "check_lint_implementation_helpers" => {
                CheckOwner::ExternalBatch(ExternalCheck::LintImplementationHelpers)
            }
            "check_lint_issue_helpers" => {
                CheckOwner::ExternalBatch(ExternalCheck::LintIssueHelpers)
            }
            "check_security_artifact_profile" => {
                CheckOwner::ExternalBatch(ExternalCheck::SecurityArtifactProfile)
            }
            "check_phase4_ci_status_compare" => {
                CheckOwner::ExternalBatch(ExternalCheck::Phase4CiStatusCompare)
            }
            "check_define_spec_worktree_routing" => {
                CheckOwner::ExternalBatch(ExternalCheck::DefineSpecWorktreeRouting)
            }
            "check_run_groom_preflight_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::RunGroomPreflightContract)
            }
            "check_grow_run_contract" => CheckOwner::ExternalBatch(ExternalCheck::GrowRunContract),
            "check_performance_workstream_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::PerformanceWorkstream)
            }
            "check_ux_ui_workstream_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::UxUiWorkstream)
            }
            "check_token_baseline_fresh" => {
                CheckOwner::ExternalBatch(ExternalCheck::TokenBaselineFresh)
            }
            "check_architecture_fitness_engine" => {
                CheckOwner::ExternalBatch(ExternalCheck::ArchitectureFitnessEngine)
            }
            "check_phase4_tests" => CheckOwner::ExternalBatch(ExternalCheck::Phase4TestSuites),
            "check_validation_matrix_smoke" => {
                CheckOwner::ExternalBatch(ExternalCheck::ValidationMatrixSmoke)
            }
            "check_reviewer_reuse_lens" => {
                CheckOwner::ExternalBatch(ExternalCheck::ReviewerReuseLens)
            }
            "check_closeout_contract" => CheckOwner::RustNative(StructuralCheck::CloseoutContract),
            "check_phase4_guardian_block_lockstep" => {
                CheckOwner::RustNative(StructuralCheck::Phase4GuardianBlockLockstep)
            }
            "check_phase4_issue_start_summary" => {
                CheckOwner::RustNative(StructuralCheck::Phase4IssueStartSummary)
            }
            "check_phase4_immediate_next_issue_pickup" => {
                CheckOwner::RustNative(StructuralCheck::Phase4ImmediateNextIssuePickup)
            }
            "check_autospec_run_continuation_contract" => {
                CheckOwner::RustNative(StructuralCheck::AutospecRunContinuation)
            }
            "check_autospec_run_codex_bounded_handoff" => {
                CheckOwner::RustNative(StructuralCheck::AutospecRunCodexBoundedHandoff)
            }
            "check_phase4_adaptive_retry" => {
                CheckOwner::RustNative(StructuralCheck::Phase4AdaptiveRetry)
            }
            "check_phase4_full_test_suite_gate" => {
                CheckOwner::RustNative(StructuralCheck::Phase4FullTestSuite)
            }
            "check_data_scope_review_lens" => {
                CheckOwner::RustNative(StructuralCheck::DataScopeReviewLens)
            }
            "check_phase4_cost_epic_parity_lockstep" => {
                CheckOwner::RustNative(StructuralCheck::Phase4CostEpicParity)
            }
            "check_docs_drift_gate_regen_conditional_parity" => {
                CheckOwner::RustNative(StructuralCheck::DocsDriftGateRegenConditionalParity)
            }
            "check_stop_mode_section" => CheckOwner::RustNative(StructuralCheck::StopMode),
            "check_keyword_routing_section" => {
                CheckOwner::RustNative(StructuralCheck::KeywordRouting)
            }
            "check_gap_remediation_section" => {
                CheckOwner::RustNative(StructuralCheck::GapRemediation)
            }
            "check_review_remediation_section" => {
                CheckOwner::RustNative(StructuralCheck::ReviewRemediation)
            }
            "check_enforcement_defaults_section" => {
                CheckOwner::RustNative(StructuralCheck::EnforcementDefaults)
            }
            "check_self_update" => CheckOwner::RustNative(StructuralCheck::SelfUpdateTrio),
            "check_self_update_duo" => CheckOwner::RustNative(StructuralCheck::SelfUpdateDuo),
            "check_codex_skills_install" => {
                CheckOwner::RustNative(StructuralCheck::CodexSkillsInstall)
            }
            "check_shared_script_install" => {
                CheckOwner::RustNative(StructuralCheck::SharedScriptInstall)
            }
            "check_root_helper_wrapper_policy" => {
                CheckOwner::RustNative(StructuralCheck::RootHelperWrapperPolicy)
            }
            "check_reference_pointer_integrity" => {
                CheckOwner::RustNative(StructuralCheck::ReferencePointerIntegrity)
            }
            "check_derive_trio_consistency" => {
                CheckOwner::ExternalBatch(ExternalCheck::DeriveTrioConsistency)
            }
            "check_autospec_gap_miner_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::GapMinerContract)
            }
            "check_startup_preflight" => CheckOwner::RustNative(StructuralCheck::StartupPreflight),
            "check_rust_output_macros" => CheckOwner::RustNative(StructuralCheck::RustOutputMacros),
            "check_grow_define_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::GrowDefineContract)
            }
            "check_autospec_doc_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::AutospecDocContract)
            }
            "check_constitution_validation_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::ConstitutionValidation)
            }
            "check_install_tests" => CheckOwner::ExternalBatch(ExternalCheck::InstallTests),
            "check_control_plane_bootstrap_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::ControlPlaneBootstrap)
            }
            unknown => panic!("frozen validation catalog check has no direct owner: {unknown}"),
        };
        Self {
            id,
            required: true,
            independent: false,
            modes: CheckModes::CatalogSlot,
            reachability: CheckReachability::for_id(id),
            owner,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckReachability {
    TopLevel,
    InternalComponent,
    LegacyUnreachable,
}

impl CheckReachability {
    fn for_id(id: &str) -> Self {
        if id == "check_architecture_fitness_engine" {
            Self::LegacyUnreachable
        } else if catalog_ids::LEGACY_TOP_LEVEL_CALL_IDS.contains(&id) {
            Self::TopLevel
        } else {
            Self::InternalComponent
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckModes {
    CatalogSlot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOwner {
    RustNative(StructuralCheck),
    External(ToolCommand),
    ExternalBatch(ExternalCheck),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralCheck {
    TrioLockstep,
    DuoLockstep,
    RequiredTrioFiles,
    FlagSentinelDocs,
    SubagentModelTier,
    HarnessDetection,
    MonitorBatchExit,
    AgentsMdSubagentSection,
    AgentsMdSubagentMatrix,
    AutospecListenFiles,
    ExamplesDirectory,
    GovernanceHeadings,
    StlDesignGuardrails,
    ExistingSpecMode,
    DocsAmendmentPresence,
    AutospecReviewSkill,
    AutospecReviewTierADirectives,
    AutospecRunPrioritySortLockstep,
    AutospecRunRegressionReviewLockstep,
    Phase1BoundedContext,
    FleetGuiSubcommandLockstep,
    AgentsMdGitHygiene,
    PaletteSingleSource,
    MermaidDocumentation,
    QaDocumentationGate,
    AutospecHarmonize,
    AutospecAutonomousSkill,
    AutospecExploreUserspaceRoster,
    AutospecExploreParallelValidation,
    AutospecAutonomousTier4Discovery,
    TeamPersonalitySelection,
    TeamPersonalityIssueTemplate,
    TeamPersonalityPhase4AndDocs,
    TeamPersonality,
    AutospecReleaseContract,
    QaVerdictContract,
    BruteForceRuleIds,
    CloseoutContract,
    Phase4GuardianBlockLockstep,
    Phase4IssueStartSummary,
    Phase4ImmediateNextIssuePickup,
    AutospecRunContinuation,
    AutospecRunCodexBoundedHandoff,
    Phase4AdaptiveRetry,
    Phase4FullTestSuite,
    DataScopeReviewLens,
    Phase4CostEpicParity,
    DocsDriftGateRegenConditionalParity,
    StopMode,
    KeywordRouting,
    GapRemediation,
    ReviewRemediation,
    EnforcementDefaults,
    SelfUpdateTrio,
    SelfUpdateDuo,
    CodexSkillsInstall,
    SharedScriptInstall,
    RootHelperWrapperPolicy,
    ReferencePointerIntegrity,
    StartupPreflight,
    RustOutputMacros,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationCatalog {
    checks: Vec<ValidationCheck>,
}

impl ValidationCatalog {
    pub fn standard() -> Self {
        Self::from_checks(
            catalog_ids::STANDARD_CHECK_IDS
                .iter()
                .map(|&id| ValidationCheck::catalog_entry(id))
                .collect(),
        )
    }

    pub fn from_checks(checks: Vec<ValidationCheck>) -> Self {
        Self { checks }
    }

    pub fn checks(&self) -> &[ValidationCheck] {
        &self.checks
    }

    pub(crate) fn registered_bats_suites(&self) -> BTreeSet<&'static str> {
        let mut suites = BTreeSet::new();
        for check in &self.checks {
            if let CheckOwner::ExternalBatch(owner) = &check.owner {
                suites.extend(owner.registered_bats_suites().iter().copied());
            }
        }
        suites
    }

    pub(crate) fn registered_bats_directories(&self) -> BTreeSet<&'static str> {
        self.checks
            .iter()
            .filter_map(|check| match &check.owner {
                CheckOwner::ExternalBatch(ExternalCheck::BatsDirectory(path)) => Some(*path),
                _ => None,
            })
            .collect()
    }

    pub fn ids(&self) -> Vec<&'static str> {
        self.checks.iter().map(|check| check.id).collect()
    }
    pub fn legacy_top_level_calls(&self) -> &'static [&'static str] {
        catalog_ids::LEGACY_TOP_LEVEL_CALL_IDS
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();

        for check in &self.checks {
            if check.id.trim().is_empty() {
                return Err("validation catalog check ID must not be empty".to_string());
            }
            if !ids.insert(check.id) {
                return Err(format!(
                    "validation catalog check ID is duplicated: {}",
                    check.id
                ));
            }
        }

        Ok(())
    }
}
