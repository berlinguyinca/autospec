use autospec_core::validation::{
    CheckOwner, ExternalCheck, StructuralCheck, ValidationCatalog, ValidationCheck,
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
