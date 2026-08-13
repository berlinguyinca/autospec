use std::path::PathBuf;

use autospec_core::validation::structural::oversized_module_refactor_issues;
use autospec_core::validation::{StructuralCheck, StructuralValidator};

#[test]
fn oversized_modules_emit_bounded_behavior_preserving_drafts() {
    let root = std::env::temp_dir().join(format!("autospec-oversized-{}", std::process::id()));
    let src = root.join("crates/demo/src");
    std::fs::create_dir_all(&src).unwrap();
    let mut content = String::from("pub fn reconcile() {}\n");
    content.push_str(&"// filler\n".repeat(8));
    std::fs::write(src.join("large.rs"), content).unwrap();
    let drafts = oversized_module_refactor_issues(&root, 5, 7);
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].severity, "error");
    assert!(drafts[0].body.contains("characterization test"));
    assert!(drafts[0].body.contains("behavior-preserving"));
    assert!(!drafts[0].title.contains("whole file cleanup"));
    let _ = std::fs::remove_dir_all(root);
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../autospec-cli/tests/fixtures/validation-cutover")
        .join(name)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn matching_trio_and_duo_skill_bodies_pass_without_external_processes() {
    StructuralValidator::validate_root(&fixture("valid-skill")).expect("valid fixtures pass");
}

#[test]
fn drifted_duo_reports_the_divergent_file() {
    let failure = StructuralValidator::validate_root(&fixture("drifted-skill"))
        .expect_err("drifted duo fails");

    assert!(failure.contains("skills/duo/codex/prompt.md"));
}

#[test]
fn missing_trio_required_file_reports_the_skill_and_file() {
    let failure = StructuralValidator::validate_root(&fixture("missing-trio-file"))
        .expect_err("missing trio file fails");

    assert_eq!(failure, "skills/trio: missing required file uninstall.sh");
}

#[test]
fn trio_lockstep_and_required_files_have_separate_owners() {
    StructuralValidator::validate_trio_lockstep(&fixture("missing-trio-file"))
        .expect("a missing installer does not make matching trio bodies drift");

    let failure = StructuralValidator::validate_required_trio_files(&fixture("missing-trio-file"))
        .expect_err("the required-files owner reports the missing installer");

    assert_eq!(failure, "skills/trio: missing required file uninstall.sh");
}

#[test]
fn trio_and_duo_lockstep_owners_validate_their_respective_harnesses() {
    StructuralValidator::validate_trio_lockstep(&fixture("valid-skill"))
        .expect("matching trio passes its owner");
    StructuralValidator::validate_duo_lockstep(&fixture("valid-skill"))
        .expect("matching duo passes its owner");
}

#[test]
fn structural_dispatch_runs_exactly_the_requested_owner() {
    StructuralValidator::run(StructuralCheck::TrioLockstep, &fixture("valid-skill"))
        .expect("trio owner passes through the dispatcher");
}

#[test]
fn startup_preflight_contract_accepts_markers_and_canonical_blocks() {
    StructuralValidator::validate_startup_preflight(&fixture("startup-preflight"))
        .expect("startup preflight fixture passes");
}

#[test]
fn startup_preflight_contract_requires_persistent_failure_evidence() {
    let root = std::env::temp_dir().join(format!(
        "autospec-startup-failure-contract-{}",
        std::process::id()
    ));
    let template_dir = root.join("templates/skill-blocks");
    let skill_dir = root.join("skills/autospec-design");
    std::fs::create_dir_all(skill_dir.join("codex")).expect("codex fixture dir");
    std::fs::create_dir_all(skill_dir.join("opencode")).expect("opencode fixture dir");
    std::fs::create_dir_all(&template_dir).expect("template fixture dir");
    std::fs::write(
        template_dir.join("startup-self-update.md"),
        "## Startup self-update\n\n```bash\ncurl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh\nbootstrap --skill all --harness all --update\n```\n",
    )
    .expect("legacy template");
    for path in [
        skill_dir.join("SKILL.md"),
        skill_dir.join("codex/prompt.md"),
        skill_dir.join("opencode/agent.md"),
    ] {
        std::fs::write(
            path,
            "<!-- autospec-block:startup-self-update SKILL_NAME=autospec-design -->\n",
        )
        .expect("skill marker");
    }

    let failure = StructuralValidator::validate_startup_preflight(&root)
        .expect_err("legacy silent-failure preflight must fail validation");
    assert!(failure.contains("last-update-failure.json"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn startup_preflight_contract_reports_a_divergent_trio_block() {
    let failure =
        StructuralValidator::validate_startup_preflight(&fixture("startup-preflight-divergent"))
            .expect_err("divergent startup block fails");

    assert_eq!(
        failure,
        "skills/autospec/codex/prompt.md preflight body diverges from canonical"
    );
}

#[test]
fn named_skill_policy_sections_pass_when_every_member_has_the_required_heading() {
    StructuralValidator::validate_policy_sections(&fixture("policy-sections"))
        .expect("policy section fixture passes");
}

#[test]
fn named_skill_policy_sections_report_the_missing_member_heading() {
    let failure = StructuralValidator::validate_policy_sections(&fixture("missing-policy-section"))
        .expect_err("missing policy section fails");

    assert_eq!(
        failure,
        "skills/autospec-run/codex/prompt.md: missing '## Stop mode' section"
    );
}

#[test]
fn self_update_sections_accept_trio_and_duo_forms() {
    StructuralValidator::validate_self_update_sections(&fixture("self-update"))
        .expect("trio and duo self-update fixtures pass");
    StructuralValidator::validate_trio_self_update_sections(&fixture("self-update"))
        .expect("trio self-update fixture passes independently");
    StructuralValidator::validate_duo_self_update_sections(&fixture("self-update"))
        .expect("duo self-update fixture passes independently");
}

#[test]
fn self_update_sections_report_a_missing_update_flag() {
    let failure =
        StructuralValidator::validate_self_update_sections(&fixture("self-update-missing-flag"))
            .expect_err("trio install flag is required");

    assert_eq!(
        failure,
        "autospec: install.sh missing --update flag handling"
    );
}

#[test]
fn keyword_routing_contract_passes_when_trio_and_classifier_literals_match() {
    StructuralValidator::validate_keyword_routing_section(&fixture("keyword-routing"))
        .expect("keyword-routing fixture passes");
}

#[test]
fn keyword_routing_contract_reports_the_missing_trio_literal() {
    let failure = StructuralValidator::validate_keyword_routing_section(&fixture(
        "keyword-routing-missing-literal",
    ))
    .expect_err("missing trio literal fails");

    assert_eq!(
        failure,
        "autospec-listen: codex/prompt.md missing completed Plan-mode handoff route"
    );
}

#[test]
fn codex_skills_install_contract_accepts_registry_and_legacy_destinations() {
    StructuralValidator::validate_codex_skills_install(&fixture("codex-skills-install"))
        .expect("installer fixture passes");
}

#[test]
fn codex_skills_install_contract_reports_a_missing_registry_destination() {
    let failure = StructuralValidator::validate_codex_skills_install(&fixture(
        "codex-skills-install-missing-registry",
    ))
    .expect_err("missing Codex registry destination fails");

    assert_eq!(
        failure,
        "skills/autospec-design/install.sh missing Codex skills-dir install (skills/$SKILL_NAME/SKILL.md)"
    );
}

#[test]
fn shared_script_install_contract_requires_only_shared_runtime_helpers() {
    StructuralValidator::validate_shared_script_install(&fixture("shared-script-install"))
        .expect("shared helper fixture passes");
}

#[test]
fn shared_script_install_contract_reports_the_autospec_design_anchor() {
    let failure = StructuralValidator::validate_shared_script_install(&fixture(
        "shared-script-install-autospec-design-missing-runtime",
    ))
    .expect_err("autospec-design must install a referenced shared helper");

    assert_eq!(
        failure,
        "check_shared_script_install: autospec-design references a shared helper but does not install into ~/.autospec/scripts"
    );
}

#[test]
fn flag_sentinel_docs_contract_requires_every_runtime_flag_to_be_documented() {
    StructuralValidator::validate_flag_sentinel_docs(&fixture("flag-sentinel-docs"))
        .expect("every runtime sentinel flag is documented");

    let failure =
        StructuralValidator::validate_flag_sentinel_docs(&fixture("flag-sentinel-docs-missing"))
            .expect_err("an undocumented runtime sentinel flag fails");

    assert_eq!(
        failure,
        "docs/FLAGS.md: missing sentinel flag(s): second.flag"
    );
}

#[test]
fn flag_sentinel_docs_requires_exact_flag_tokens_not_substrings() {
    let failure =
        StructuralValidator::validate_flag_sentinel_docs(&fixture("flag-sentinel-docs-substring"))
            .expect_err("a different flag containing the name must not document the required flag");

    assert_eq!(
        failure,
        "docs/FLAGS.md: missing sentinel flag(s): second.flag"
    );
}

#[test]
fn per_skill_model_and_monitor_contracts_validate_multi_harness_skills() {
    let root = fixture("skill-model-contracts");

    StructuralValidator::validate_subagent_model_tiers(&root).expect("model-tier directives pass");
    StructuralValidator::validate_harness_detection_blocks(&root)
        .expect("harness detection block passes");
    StructuralValidator::validate_monitor_batch_exits(&root)
        .expect("monitor batch exit contract passes");

    let failure = StructuralValidator::validate_harness_detection_blocks(&fixture(
        "skill-model-contracts-missing-silently",
    ))
    .expect_err("harness fallback wording is required");

    assert_eq!(
        failure,
        "autospec: ## Harness detection section present but missing 'silently' fallback reference"
    );
}

#[test]
fn repository_presence_and_subagent_policy_contracts_have_direct_owners() {
    let root = fixture("presence-contracts");

    StructuralValidator::validate_agents_md_subagent_section(&root)
        .expect("two-tier subsection headings pass");
    StructuralValidator::validate_agents_md_subagent_matrix(&root)
        .expect("decision matrix and skill reference pass");
    StructuralValidator::validate_autospec_listen_files(&root)
        .expect("listener package files pass");
    StructuralValidator::validate_examples_directory(&root).expect("examples directory passes");

    let failure = StructuralValidator::validate_examples_directory(&fixture(
        "presence-contracts-missing-example",
    ))
    .expect_err("a missing canonical example fails");

    assert_eq!(failure, "examples/README.md: required file missing");
}

#[test]
fn policy_section_validators_accept_qualified_headings_used_by_the_repository() {
    StructuralValidator::validate_review_remediation_sections(&workspace_root())
        .expect("qualified remediation headings preserve the legacy prefix contract");
    StructuralValidator::validate_agents_md_subagent_section(&workspace_root())
        .expect("qualified tier headings preserve the legacy prefix contract");
}

#[test]
fn documentation_and_skill_contracts_preserve_their_required_literals() {
    let root = fixture("documentation-contracts");

    StructuralValidator::validate_governance_headings(&root).expect("governance headings pass");
    StructuralValidator::validate_autospec_stl_design_guardrails(&root)
        .expect("CAD guardrails pass");
    StructuralValidator::validate_existing_spec_mode(&root).expect("existing spec mode passes");
    StructuralValidator::validate_docs_amendment_presence(&root)
        .expect("documentation amendment files pass");

    let failure = StructuralValidator::validate_docs_amendment_presence(&fixture(
        "documentation-contracts-missing-llms",
    ))
    .expect_err("missing generated llms index fails");

    assert_eq!(
        failure,
        "llms.txt: missing — run gen-llms-txt.sh --repo-root . to regenerate"
    );
}

#[test]
fn autospec_review_contract_requires_a_complete_lockstep_trio_and_two_tier_a_directives() {
    StructuralValidator::validate_autospec_review_skill(&fixture("review-skill-contract"))
        .expect("complete review trio stays in lockstep");
    StructuralValidator::validate_autospec_review_tier_a_directives(&fixture(
        "review-skill-contract",
    ))
    .expect("two Tier A directives pass");

    let failure = StructuralValidator::validate_autospec_review_tier_a_directives(&fixture(
        "review-skill-contract-one-tier-a",
    ))
    .expect_err("one Tier A directive does not satisfy the contract");

    assert_eq!(
        failure,
        "expected ≥2 'Tier A (spec work)' directives in autospec-review/SKILL.md, found 1"
    );
}

#[test]
fn autospec_run_review_contracts_lock_the_priority_and_folded_regression_guidance() {
    let root = fixture("autospec-run-review-contracts");

    StructuralValidator::validate_autospec_run_priority_sort_lockstep(&root)
        .expect("priority sort excerpts match");
    StructuralValidator::validate_autospec_run_regression_review_lockstep(&root)
        .expect("folded reviewer guidance passes");

    let failure = StructuralValidator::validate_autospec_run_regression_review_lockstep(&fixture(
        "autospec-run-review-contracts-second-tier-a",
    ))
    .expect_err("a second Tier A reviewer dispatch is forbidden");

    assert_eq!(
        failure,
        "second TIER_A regression meta-review dispatch still present in skills/autospec-run/SKILL.md (should be folded into reviewer brief)"
    );
}

#[test]
fn bounded_context_and_fleet_gui_contracts_are_checked_without_shells() {
    StructuralValidator::validate_phase1_bounded_context_contract(&fixture("phase1-bounded"))
        .expect("all Phase 1 adapters carry bounded-context guidance");
    StructuralValidator::validate_fleet_gui_subcommand_lockstep(&fixture("fleet-gui"))
        .expect("fleet GUI route is present in every adapter");

    let failure = StructuralValidator::validate_phase1_bounded_context_contract(&fixture(
        "phase1-bounded-missing-fallback",
    ))
    .expect_err("missing context-overflow fallback fails");

    assert_eq!(
        failure,
        "skills/autospec-define/codex/prompt.md missing context-overflow fallback directive"
    );
}

#[test]
fn agents_git_hygiene_contract_requires_every_policy_anchor() {
    StructuralValidator::validate_agents_md_git_hygiene(&fixture("agents-git-hygiene"))
        .expect("git hygiene policy anchors pass");

    let failure = StructuralValidator::validate_agents_md_git_hygiene(&fixture(
        "agents-git-hygiene-missing-prune",
    ))
    .expect_err("missing worktree prune policy fails");

    assert_eq!(
        failure,
        "AGENTS.md git hygiene section missing 'git worktree prune' cleanup rule (§D5)"
    );
}

#[test]
fn palette_single_source_contract_rejects_hardcoded_palette_values_outside_doc_style() {
    StructuralValidator::validate_palette_single_source(&fixture("palette-single-source"))
        .expect("palette values are limited to the source of truth");

    let failure = StructuralValidator::validate_palette_single_source(&fixture(
        "palette-single-source-duplicate",
    ))
    .expect_err("a duplicate palette color in a shell file fails");

    assert_eq!(
        failure,
        "palette single-source violation: scripts/duplicate.sh contains palette hex #aabbcc"
    );
}

#[test]
fn documentation_guidance_contracts_preserve_mermaid_qa_and_harmonize_requirements() {
    let root = fixture("static-skill-contracts");

    StructuralValidator::validate_mermaid_documentation_contract(&root)
        .expect("Mermaid guidance is present in every required adapter");
    StructuralValidator::validate_qa_documentation_gate(&root)
        .expect("QA documentation gate is present in every adapter");
    StructuralValidator::validate_autospec_harmonize_contract(&root)
        .expect("harmonize stages and handoff are present in every adapter");

    let failure = StructuralValidator::validate_autospec_harmonize_contract(&fixture(
        "static-skill-contracts-missing-harmonize-stage",
    ))
    .expect_err("a missing harmonize stage fails");

    assert_eq!(
        failure,
        "skills/autospec-harmonize/SKILL.md: missing stage label 'pick'"
    );
}

#[test]
fn documentation_guidance_contracts_pass_in_this_repository() {
    let root = workspace_root();

    StructuralValidator::validate_mermaid_documentation_contract(&root)
        .expect("repository Mermaid guidance satisfies the contract");
    StructuralValidator::validate_qa_documentation_gate(&root)
        .expect("repository QA documentation gate satisfies the contract");
    StructuralValidator::validate_autospec_harmonize_contract(&root)
        .expect("repository harmonize guidance satisfies the contract");
}

#[test]
fn autonomous_discovery_and_team_personality_contracts_have_direct_rust_owners() {
    let root = fixture("autonomous-team-contracts");

    StructuralValidator::validate_autospec_autonomous_skill_contract(&root)
        .expect("autonomous skill contract passes");
    StructuralValidator::validate_autospec_explore_userspace_roster_contract(&root)
        .expect("userspace roster contract passes");
    StructuralValidator::validate_autospec_autonomous_tier4_discovery_contract(&root)
        .expect("Tier 4 discovery contract passes");
    StructuralValidator::validate_team_personality_selection_contract(&root)
        .expect("team selection contract passes");
    StructuralValidator::validate_team_personality_issue_template_contract(&root)
        .expect("team issue template contract passes");
    StructuralValidator::validate_team_personality_phase4_and_docs_contract(&root)
        .expect("team Phase 4 and documentation contract passes");
    StructuralValidator::validate_team_personality_contract(&root)
        .expect("team personality aggregate contract passes");

    let failure = StructuralValidator::validate_autospec_autonomous_tier4_discovery_contract(
        &fixture("autonomous-team-contracts-missing-tier4-trust"),
    )
    .expect_err("Tier 4 requires its untrusted-data boundary");

    assert_eq!(
        failure,
        "skills/autospec-autonomous/SKILL.md: Tier 4 must state the untrusted-DATA trust boundary"
    );
}

#[test]
fn autonomous_discovery_and_team_personality_contracts_pass_in_this_repository() {
    let root = workspace_root();

    StructuralValidator::validate_autospec_autonomous_skill_contract(&root)
        .expect("repository autonomous skill contract passes");
    StructuralValidator::validate_autospec_explore_userspace_roster_contract(&root)
        .expect("repository userspace roster contract passes");
    StructuralValidator::validate_autospec_autonomous_tier4_discovery_contract(&root)
        .expect("repository Tier 4 discovery contract passes");
    StructuralValidator::validate_team_personality_contract(&root)
        .expect("repository team personality contract passes");
}

#[test]
fn phase4_static_policy_contracts_have_direct_rust_owners() {
    let root = fixture("phase4-static-contracts");

    StructuralValidator::validate_closeout_contract(&root).expect("closeout contract passes");
    StructuralValidator::validate_phase4_guardian_block_lockstep(&root)
        .expect("guardian blocks are in lockstep");
    StructuralValidator::validate_phase4_issue_start_summary(&root)
        .expect("issue-start summaries pass");
    StructuralValidator::validate_phase4_immediate_next_issue_pickup(&root)
        .expect("immediate queue pickup passes");
    StructuralValidator::validate_autospec_run_continuation_contract(&root)
        .expect("continuation contract passes");
    StructuralValidator::validate_autospec_run_codex_bounded_handoff(&root)
        .expect("bounded handoff contract passes");
    StructuralValidator::validate_phase4_adaptive_retry(&root).expect("adaptive retry passes");
    StructuralValidator::validate_phase4_full_test_suite_gate(&root)
        .expect("full-suite gate passes");
    StructuralValidator::validate_data_scope_review_lens(&root).expect("data-scope lens passes");
    StructuralValidator::validate_phase4_cost_epic_parity_lockstep(&root)
        .expect("cost epic parity passes");
    StructuralValidator::validate_docs_drift_gate_regen_conditional_parity(&root)
        .expect("docs drift conditional parity passes");

    let failure = StructuralValidator::validate_autospec_run_codex_bounded_handoff(&fixture(
        "phase4-static-contracts-missing-bounded-handoff",
    ))
    .expect_err("an inherited full-history handoff is forbidden");

    assert_eq!(
        failure,
        "skills/autospec-run/SKILL.md still directs Codex native subagents to fork/inherit the parent context"
    );

    let failure = StructuralValidator::validate_autospec_run_continuation_contract(&fixture(
        "phase4-static-contracts-missing-run-adapter",
    ))
    .expect_err("every legacy run adapter is mandatory");

    assert_eq!(
        failure,
        "skills/autospec-run/codex/prompt.md: required file missing"
    );

    let failure = StructuralValidator::validate_phase4_guardian_block_lockstep(&fixture(
        "phase4-static-contracts-unmatched-guardian-end",
    ))
    .expect_err("a marker imbalance must not become a valid guardian block");

    assert_eq!(
        failure,
        "guardian lockstep: no guardian block found in skills/autospec/SKILL.md"
    );
}

#[test]
fn phase4_static_policy_contracts_pass_in_this_repository() {
    let root = workspace_root();

    StructuralValidator::validate_closeout_contract(&root)
        .expect("repository closeout contract passes");
    StructuralValidator::validate_phase4_guardian_block_lockstep(&root)
        .expect("repository guardian blocks are in lockstep");
    StructuralValidator::validate_phase4_issue_start_summary(&root)
        .expect("repository issue-start summaries pass");
    StructuralValidator::validate_phase4_immediate_next_issue_pickup(&root)
        .expect("repository immediate queue pickup passes");
    StructuralValidator::validate_autospec_run_continuation_contract(&root)
        .expect("repository continuation contract passes");
    StructuralValidator::validate_autospec_run_codex_bounded_handoff(&root)
        .expect("repository bounded handoff contract passes");
    StructuralValidator::validate_phase4_adaptive_retry(&root)
        .expect("repository adaptive retry passes");
    StructuralValidator::validate_phase4_full_test_suite_gate(&root)
        .expect("repository full-suite gate passes");
    StructuralValidator::validate_data_scope_review_lens(&root)
        .expect("repository data-scope lens passes");
    StructuralValidator::validate_phase4_cost_epic_parity_lockstep(&root)
        .expect("repository cost epic parity passes");
    StructuralValidator::validate_docs_drift_gate_regen_conditional_parity(&root)
        .expect("repository docs drift conditional parity passes");
}

#[test]
fn release_and_qa_verdict_contracts_have_direct_rust_owners() {
    let root = fixture("release-qa-contracts");

    StructuralValidator::validate_autospec_release_contract(&root)
        .expect("release readiness wrapper contract passes");
    StructuralValidator::validate_qa_verdict_contract(&root)
        .expect("QA verdict artifact contract passes");
    StructuralValidator::validate_brute_force_rule_ids(&fixture("brute-force-rule-ids"))
        .expect("RULE_ID lockstep and sweep-script contract pass");

    let failure = StructuralValidator::validate_qa_verdict_contract(&fixture(
        "release-qa-contracts-missing-benchmark-category",
    ))
    .expect_err("the benchmark category stays release-blocking");

    assert_eq!(
        failure,
        "skills/autospec-qa/SKILL.md: missing benchmark_overfit in category enum"
    );
}

#[test]
fn release_and_qa_verdict_contracts_pass_in_this_repository() {
    let root = workspace_root();

    StructuralValidator::validate_autospec_release_contract(&root)
        .expect("repository release readiness wrapper contract passes");
    StructuralValidator::validate_qa_verdict_contract(&root)
        .expect("repository QA verdict artifact contract passes");
}
