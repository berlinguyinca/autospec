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
            "check_derive_trio_consistency" => {
                CheckOwner::ExternalBatch(ExternalCheck::DeriveTrioConsistency)
            }
            "check_autospec_gap_miner_contract" => {
                CheckOwner::ExternalBatch(ExternalCheck::GapMinerContract)
            }
            "check_startup_preflight" => CheckOwner::RustNative(StructuralCheck::StartupPreflight),
            _ => CheckOwner::RustNative(StructuralCheck::CatalogSlot),
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
        } else if LEGACY_TOP_LEVEL_CALL_IDS.contains(&id) {
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
    CatalogSlot,
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
    StopMode,
    KeywordRouting,
    GapRemediation,
    ReviewRemediation,
    EnforcementDefaults,
    SelfUpdateTrio,
    SelfUpdateDuo,
    CodexSkillsInstall,
    SharedScriptInstall,
    StartupPreflight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationCatalog {
    checks: Vec<ValidationCheck>,
}

impl ValidationCatalog {
    pub fn standard() -> Self {
        Self::from_checks(
            STANDARD_CHECK_IDS
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

    pub fn ids(&self) -> Vec<&'static str> {
        self.checks.iter().map(|check| check.id).collect()
    }

    pub fn legacy_top_level_calls(&self) -> &'static [&'static str] {
        LEGACY_TOP_LEVEL_CALL_IDS
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

const LEGACY_TOP_LEVEL_CALL_IDS: &[&str] = &[
    "check_startup_preflight",
    "check_stop_mode_section",
    "check_keyword_routing_section",
    "check_flag_sentinel_docs",
    "check_gap_remediation_section",
    "check_autospec_gap_miner_contract",
    "check_review_remediation_section",
    "check_enforcement_defaults_section",
    "check_codex_skills_install",
    "check_shared_script_install",
    "check_mutation_and_negative_path",
    "check_python_suites",
    "check_agents_md_subagent_section",
    "check_agents_md_subagent_matrix",
    "check_autospec_listen_files",
    "check_examples_dir",
    "check_governance_headings",
    "check_autospec_stl_design_guardrails",
    "check_existing_spec_mode",
    "check_lint_issue_helpers",
    "check_lint_implementation_helpers",
    "check_implementer_contract",
    "check_reviewer_contract",
    "check_closeout_contract",
    "check_lint_heredoc_handling",
    "check_lint_reuse_triage",
    "check_reviewer_reuse_lens",
    "check_quality_differential",
    "check_repo_quality_audit_loop",
    "check_usage_limit_helper",
    "check_supersession_contract",
    "check_grooming_contract",
    "check_run_groom_preflight_contract",
    "check_ship_completeness",
    "check_phase4_guardian_block_lockstep",
    "check_phase1_bounded_context_contract",
    "check_phase4_issue_start_summary",
    "check_phase4_immediate_next_issue_pickup",
    "check_autospec_run_continuation_contract",
    "check_autospec_run_codex_bounded_handoff",
    "check_phase4_single_agent_discipline",
    "check_phase4_adaptive_retry",
    "check_phase4_full_test_suite_gate",
    "check_data_scope_review_lens",
    "check_phase4_final_quality_gate",
    "check_phase4_ci_status_compare",
    "check_phase4_cost_epic_parity_lockstep",
    "check_docs_drift_gate_regen_conditional_parity",
    "check_worktree_ladder_assert_parity",
    "check_autospec_sweep_config_contract",
    "check_constitution_validation_contract",
    "check_generated_yaml_parse",
    "check_autospec_fleet_scripts",
    "check_fleet_gui_subcommand_lockstep",
    "check_team_personality_contract",
    "check_autospec_run_priority_sort_lockstep",
    "check_autospec_run_regression_review_lockstep",
    "check_claim_cas_guard",
    "check_autospec_review_skill_present",
    "check_autospec_review_tier_a_directives",
    "check_autospec_test_skill_present",
    "check_autospec_playwright_skill_present",
    "check_autospec_qa_contract",
    "check_qa_deploy_contract",
    "check_brute_force_rule_ids",
    "check_dogfood_detectors",
    "check_qa_verify_first_discipline",
    "check_qa_incident_contract",
    "check_qa_exhaustiveness_contract",
    "check_qa_heal_loop_contract",
    "check_autospec_release_contract",
    "check_qa_verdict_contract",
    "check_release_verdict_script",
    "check_docs_amendment_presence",
    "check_autospec_autonomous_contract",
    "check_autospec_autonomous_skill_contract",
    "check_conductor_wiring_contract",
    "check_autonomy_guardrails_foundation",
    "check_autospec_refine_contract",
    "check_autospec_continue_contract",
    "check_autospec_loop_contract",
    "check_autospec_run_summary_contract",
    "check_install_tests",
    "check_db_module_install",
    "check_phase4_tests",
    "check_autospec_parallel_dispatch_contract",
    "check_autospec_explore_implementer_base",
    "check_loop_handoff_harness_awareness",
    "check_autospec_explore_researchers_deterministic",
    "check_autospec_explore_researchers_llm",
    "check_autospec_explore_specialists_discovery",
    "check_autospec_explore_contract",
    "check_explore_trio_worktree_assert",
    "check_autospec_explore_discovery_contract",
    "check_autospec_explore_stage2_intersect_contract",
    "check_autospec_explore_userspace_roster_contract",
    "check_autospec_autonomous_tier4_discovery_contract",
    "check_autospec_explore_style_normalization_contract",
    "check_autospec_explore_spec_first_contract",
    "check_autospec_explore_qa_gate_contract",
    "check_autospec_release_area_contract",
    "check_release_trio_worktree_assert",
    "check_autospec_sweep_area_contract",
    "check_autospec_qa_cluster_contract",
    "check_autospec_qa_bug_class_contract",
    "check_autospec_resume_contract",
    "check_palette_single_source",
    "check_autospec_doc_contract",
    "check_mermaid_documentation_contract",
    "check_watchdog_worktree_gc",
    "check_qa_documentation_gate",
    "check_agents_md_git_hygiene",
    "check_define_spec_worktree_routing",
    "check_token_baseline_fresh",
    "check_block_expansion",
    "check_derive_trio_consistency",
    "check_claim_guard_contract",
    "check_autospec_harmonize_contract",
    "check_autospec_upgrade_contract",
    "check_autospec_fab_contract",
    "check_autospec_autonomous_contract",
    "check_autospec_autonomous_skill_contract",
    "check_conductor_wiring_contract",
    "check_autonomy_guardrails_foundation",
    "check_autonomous_phase2_suite",
    "check_persona_suite",
    "check_performance_workstream_contract",
    "check_ux_ui_workstream_contract",
    "check_reuse_lens_suite",
    "check_control_plane_bootstrap_contract",
    "check_growth_shared_contract",
    "check_growth_candidate_pipeline_contract",
    "check_grow_define_contract",
    "check_grow_run_pipeline_contract",
    "check_grow_run_contract",
    "check_db_telemetry_contract",
    "check_bash_syntax",
    "check_bash_syntax",
];

const STANDARD_CHECK_IDS: &[&str] = &[
    "check_lockstep",
    "check_lockstep_duo",
    "check_bash_syntax",
    "check_frontmatter",
    "check_required_files",
    "check_flag_sentinel_docs",
    "check_derive_trio_consistency",
    "check_stop_mode_section",
    "check_keyword_routing_section",
    "check_gap_remediation_section",
    "check_autospec_gap_miner_contract",
    "check_review_remediation_section",
    "check_enforcement_defaults_section",
    "check_self_update",
    "check_self_update_duo",
    "check_startup_preflight",
    "check_codex_skills_install",
    "check_shared_script_install",
    "check_mutation_and_negative_path",
    "check_python_suites",
    "check_subagent_model_tier",
    "check_phase1_bounded_context_contract",
    "check_autospec_sweep_config_contract",
    "check_constitution_validation_contract",
    "check_autospec_fleet_scripts",
    "check_generated_yaml_parse",
    "check_fleet_gui_subcommand_lockstep",
    "check_harness_detection_block",
    "check_monitor_batch_exit",
    "check_agents_md_subagent_section",
    "check_agents_md_subagent_matrix",
    "check_autospec_listen_files",
    "check_examples_dir",
    "check_lint_issue_helpers",
    "check_lint_implementation_helpers",
    "check_implementer_contract",
    "check_reviewer_contract",
    "check_closeout_contract",
    "check_usage_limit_helper",
    "check_supersession_contract",
    "check_ship_completeness",
    "check_phase4_guardian_block_lockstep",
    "check_phase4_issue_start_summary",
    "check_phase4_immediate_next_issue_pickup",
    "check_autospec_run_continuation_contract",
    "check_autospec_run_codex_bounded_handoff",
    "check_phase4_single_agent_discipline",
    "check_phase4_adaptive_retry",
    "check_phase4_full_test_suite_gate",
    "check_data_scope_review_lens",
    "check_phase4_final_quality_gate",
    "check_phase4_cost_epic_parity_lockstep",
    "check_docs_drift_gate_regen_conditional_parity",
    "check_worktree_ladder_assert_parity",
    "check_team_personality_selection_contract",
    "check_team_personality_issue_template_contract",
    "check_team_personality_phase4_and_docs_contract",
    "check_team_personality_contract",
    "check_governance_headings",
    "check_autospec_stl_design_guardrails",
    "check_autospec_run_priority_sort_lockstep",
    "check_autospec_run_regression_review_lockstep",
    "check_autospec_review_skill_present",
    "check_autospec_review_tier_a_directives",
    "check_existing_spec_mode",
    "check_docs_amendment_presence",
    "check_autospec_test_skill_present",
    "check_autospec_playwright_skill_present",
    "check_autospec_qa_contract",
    "check_qa_deploy_contract",
    "check_autospec_sweep_area_contract",
    "check_autospec_release_contract",
    "check_release_verdict_script",
    "check_qa_verdict_contract",
    "check_brute_force_rule_ids",
    "check_dogfood_detectors",
    "check_qa_verify_first_discipline",
    "check_qa_incident_contract",
    "check_qa_exhaustiveness_contract",
    "check_qa_heal_loop_contract",
    "check_lint_heredoc_handling",
    "check_lint_reuse_triage",
    "check_reviewer_reuse_lens",
    "check_quality_differential",
    "check_repo_quality_audit_loop",
    "check_autospec_autonomous_contract",
    "check_autospec_refine_contract",
    "check_autospec_run_summary_contract",
    "check_db_module_install",
    "check_install_tests",
    "check_autospec_parallel_dispatch_contract",
    "check_autospec_continue_contract",
    "check_autospec_loop_contract",
    "check_claim_cas_guard",
    "check_watchdog_worktree_gc",
    "check_agents_md_git_hygiene",
    "check_define_spec_worktree_routing",
    "check_token_baseline_fresh",
    "check_block_expansion",
    "check_phase4_ci_status_compare",
    "check_architecture_fitness_engine",
    "check_autospec_qa_cluster_contract",
    "check_autospec_qa_bug_class_contract",
    "check_loop_handoff_harness_awareness",
    "check_phase4_tests",
    "check_autospec_explore_implementer_base",
    "check_autospec_explore_researchers_deterministic",
    "check_autospec_explore_researchers_llm",
    "check_autospec_explore_specialists_discovery",
    "check_autospec_explore_contract",
    "check_explore_trio_worktree_assert",
    "check_autospec_explore_stage2_intersect_contract",
    "check_autospec_explore_userspace_roster_contract",
    "check_autospec_autonomous_tier4_discovery_contract",
    "check_autospec_explore_style_normalization_contract",
    "check_autospec_explore_discovery_contract",
    "check_autospec_explore_spec_first_contract",
    "check_autospec_explore_qa_gate_contract",
    "check_autospec_release_area_contract",
    "check_release_trio_worktree_assert",
    "check_autospec_resume_structure",
    "check_autospec_supervisor_structure",
    "check_autospec_resume_contract",
    "check_palette_single_source",
    "check_autospec_doc_contract",
    "check_mermaid_documentation_contract",
    "check_qa_documentation_gate",
    "check_claim_guard_contract",
    "check_autospec_harmonize_contract",
    "check_autospec_upgrade_contract",
    "check_autospec_fab_contract",
    "check_fab_container_dockerfile",
    "check_autospec_autonomous_skill_contract",
    "check_conductor_wiring_contract",
    "check_autonomy_guardrails_foundation",
    "check_autonomous_phase2_suite",
    "check_persona_suite",
    "check_performance_workstream_contract",
    "check_ux_ui_workstream_contract",
    "check_reuse_lens_suite",
    "check_control_plane_bootstrap_contract",
    "check_growth_shared_contract",
    "check_growth_candidate_pipeline_contract",
    "check_grow_define_contract",
    "check_grow_run_pipeline_contract",
    "check_grow_run_contract",
    "check_db_telemetry_contract",
    "check_grooming_contract",
    "check_run_groom_preflight_contract",
];
