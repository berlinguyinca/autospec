use std::collections::BTreeSet;

use autospec_core::validation::{
    CheckOwner, CheckReachability, ExternalCheck, StructuralCheck, ValidationCatalog,
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

    assert_eq!(calls.len(), 138);
    assert_eq!(calls.iter().copied().collect::<BTreeSet<_>>().len(), 133);
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
    assert_eq!(frozen_catalog_ids().len(), 149);
}

#[test]
fn frozen_catalog_keeps_the_flag_sentinel_docs_gate_in_declaration_order() {
    let ids = frozen_catalog_ids();

    assert_eq!(ids.len(), 149);
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
fn catalog_rejects_empty_and_duplicate_ids() {
    let empty = ValidationCatalog::from_checks(vec![ValidationCheck::catalog_entry("")]);
    let duplicate = ValidationCatalog::from_checks(vec![
        ValidationCheck::catalog_entry("check_once"),
        ValidationCheck::catalog_entry("check_once"),
    ]);

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
