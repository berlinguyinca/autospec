use std::path::PathBuf;
use std::process::Command;

use autospec_core::validation::{Jobs, ValidationCatalog, ValidationOptions, ValidationPlan};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn frozen_catalog_ids() -> Vec<String> {
    std::fs::read_to_string(
        workspace_root()
            .join("crates/autospec-cli/tests/fixtures/validation-cutover/catalog-v1.json"),
    )
    .expect("frozen catalog fixture is readable")
    .lines()
    .filter_map(|line| {
        let candidate = line.trim().trim_end_matches(',').trim_matches('"');
        candidate
            .starts_with("check_")
            .then(|| candidate.to_string())
    })
    .collect()
}

#[test]
fn direct_plans_match_the_frozen_catalog_in_full_fast_scoped_and_parallel_modes() {
    let catalog = ValidationCatalog::standard();
    let frozen = frozen_catalog_ids();
    let catalog_ids = catalog
        .checks()
        .iter()
        .map(|check| check.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(catalog_ids, frozen);

    let full = ValidationPlan::build(&catalog, &ValidationOptions::default())
        .expect("full direct plan builds");
    // +1 for check_reference_pointer_integrity (#3158); see validation_runner.rs.
    assert_eq!(full.ids().len(), 144);
    assert_eq!(full.unique_ids().len(), 139);

    let fast_options =
        ValidationOptions::parse(["--fast", "--jobs=4"]).expect("fast options parse");
    let fast = ValidationPlan::build(&catalog, &fast_options).expect("fast direct plan builds");
    assert_eq!(fast.ids().len(), 136);
    assert_eq!(fast.parallelism(), 4);
    assert_eq!(fast_options.jobs, Jobs::Fixed(4));
    assert!(!fast.ids().contains(&"check_python_suites"));

    let scoped_options =
        ValidationOptions::parse(["--changed=HEAD"]).expect("scoped options parse");
    let scoped = ValidationPlan::build_with_changed_paths(
        &catalog,
        &scoped_options,
        ["skills/autospec-run/SKILL.md"],
    )
    .expect("scoped direct plan builds");
    assert_eq!(scoped.changed_base(), Some("HEAD"));
    assert_eq!(scoped.changed_paths(), &["skills/autospec-run/SKILL.md"]);
    assert_eq!(scoped.ids(), full.ids());
}

#[test]
fn legacy_validation_surfaces_are_absent_from_tracked_files() {
    let symbols = [
        ["scripts", "validate.sh"].join("/"),
        ["AUTOSPEC_", "FORCE_LEGACY_SHELL"].concat(),
        ["AUTOSPEC_", "VALIDATE_FROM_SHELL"].concat(),
        ["AUTOSPEC_", "VALIDATE_FROM_RUST"].concat(),
        ["AUTOSPEC_", "VALIDATE_LEGACY_ACTIVE"].concat(),
        ["AUTOSPEC_", "VALIDATE_INSTALL_TESTS_ONLY"].concat(),
    ];

    for symbol in symbols {
        let output = Command::new("git")
            .args(["grep", "-n", "--", &symbol])
            .current_dir(workspace_root())
            .output()
            .expect("git grep runs");

        assert!(
            !output.status.success(),
            "legacy validation surface remains for {symbol}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}
