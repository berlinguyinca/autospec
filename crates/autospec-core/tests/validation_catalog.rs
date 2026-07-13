use autospec_core::validation::{CheckOwner, StructuralCheck, ValidationCatalog, ValidationCheck};

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
    assert_eq!(frozen_catalog_ids().len(), 148);
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
