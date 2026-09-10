//! Integration test for issue #3667: two agent patches merged cleanly and
//! did not compile — a semantic integration conflict.
//!
//! The incident shape: `#23` adds fields to `DeploymentManifest` and
//! `ResourceEnvelope`; `#28` adds `gateway_catalog.rs`, which constructs
//! both. Each patch built and passed its tests against the `main` it was
//! written against. `#28` merged first; `#23` then fails with `E0063` at
//! three construction sites in a tree neither agent ever built.
//!
//! The rules under test:
//!
//! 1. a patch is verified against the main it will *land* on, not the main
//!    it was written against — a verdict is invalidated by every merge that
//!    lands in between;
//! 2. two ready issues that touch the same *type* (not merely the same
//!    file) are serialised, unless both specs declare the type's interface
//!    contract;
//! 3. a shape change names its call sites, and an unaddressed constructor on
//!    the landing main is the `E0063` set;
//! 4. a semantic conflict is re-dispatched against the post-landing main,
//!    never hand-repaired, while the authoring context is available.

use autospec_core::semantic_integration::{
    assess, base_currency, repair_strategy, spec_names_call_sites, type_coexistence,
    unaddressed_constructors, BaseCurrency, ConstructorSite, IntegrationVerdict, RepairStrategy,
    ShapeChange, TypeCoexistence, TypeSurface,
};

/// The `main` both agents started from.
const BASE: &str = "a1b2c3d";
/// The `main` after `#28` (the gateway catalog) merged.
const LANDING: &str = "e4f5a6b";

/// The shape change from `#23`.
fn issue_23_change() -> ShapeChange {
    ShapeChange::new(
        "DeploymentManifest",
        ["context_constraint"],
        ["crates/deploy/src/manifest.rs::DeploymentManifest::new"],
    )
}

/// The construction sites of `DeploymentManifest` on the landing main —
/// the two in `gateway_catalog.rs` that `#28` added, plus the one `#23`
/// itself updates.
fn landing_sites() -> Vec<ConstructorSite> {
    vec![
        ConstructorSite {
            file: "crates/deploy/src/manifest.rs".to_string(),
            site: "DeploymentManifest::new".to_string(),
        },
        ConstructorSite {
            file: "crates/deploy/src/gateway_catalog.rs".to_string(),
            site: "DeploymentManifest::from_catalog_entry".to_string(),
        },
        ConstructorSite {
            file: "crates/deploy/src/gateway_catalog.rs".to_string(),
            site: "DeploymentManifest::from_registry".to_string(),
        },
    ]
}

#[test]
fn the_incident_verdict_is_stale_and_the_conflict_is_detected() {
    // #23 verified green on BASE — before #28 landed. The verdict is about
    // a tree the patch will not land on.
    let assessment = assess(
        BASE,
        LANDING,
        &[28],
        &issue_23_change(),
        &landing_sites(),
        true,
    );

    assert_eq!(assessment.base, BaseCurrency::Stale { landed: vec![28] });
    assert_eq!(
        assessment.verdict,
        IntegrationVerdict::SemanticConflict {
            missing: vec![
                ConstructorSite {
                    file: "crates/deploy/src/gateway_catalog.rs".to_string(),
                    site: "DeploymentManifest::from_catalog_entry".to_string(),
                },
                ConstructorSite {
                    file: "crates/deploy/src/gateway_catalog.rs".to_string(),
                    site: "DeploymentManifest::from_registry".to_string(),
                },
            ],
        }
    );
    // Rule 4: the authoring agent's context is available, so the patch is
    // re-dispatched against the post-#28 main — not hand-repaired.
    assert_eq!(
        assessment.repair,
        RepairStrategy::ReDispatch {
            against: LANDING.to_string()
        }
    );
}

#[test]
fn a_verdict_on_the_landing_revision_is_current() {
    let assessment = assess(
        LANDING,
        LANDING,
        &[],
        &issue_23_change(),
        &landing_sites(),
        true,
    );
    assert_eq!(assessment.base, BaseCurrency::Current);
}

#[test]
fn retesting_once_at_open_time_is_not_enough() {
    // The patch was verified on BASE and #28 landed since. Even a patch
    // that names every call site it knows about must be re-verified: the
    // landing main has construction sites the patch never saw.
    let change = ShapeChange::new(
        "DeploymentManifest",
        ["context_constraint"],
        ["crates/deploy/src/manifest.rs::DeploymentManifest::new"],
    );
    let currency = base_currency(BASE, LANDING, &[28]);
    assert_eq!(currency, BaseCurrency::Stale { landed: vec![28] });
    assert!(!unaddressed_constructors(&change, &landing_sites()).is_empty());
}

#[test]
fn a_fully_declared_shape_change_composes_cleanly() {
    let change = ShapeChange::new(
        "DeploymentManifest",
        ["context_constraint"],
        [
            "crates/deploy/src/manifest.rs::DeploymentManifest::new",
            "crates/deploy/src/gateway_catalog.rs::DeploymentManifest::from_catalog_entry",
            "crates/deploy/src/gateway_catalog.rs::DeploymentManifest::from_registry",
        ],
    );
    let assessment = assess(LANDING, LANDING, &[], &change, &landing_sites(), true);
    assert_eq!(assessment.verdict, IntegrationVerdict::Clean);
    assert_eq!(assessment.repair, RepairStrategy::NoAction);
}

#[test]
fn issues_touching_the_same_type_are_serialised() {
    // The incident shape: #23 changes the types, #28 constructs them, the
    // files are disjoint — a file-level write surface sees no overlap.
    let issue_23 = TypeSurface::new(
        23,
        ["DeploymentManifest", "ResourceEnvelope"],
        [] as [&str; 0],
    );
    let issue_28 = TypeSurface::new(
        28,
        ["DeploymentManifest", "ResourceEnvelope"],
        [] as [&str; 0],
    );
    assert_eq!(
        type_coexistence(&issue_23, &issue_28),
        TypeCoexistence::Serialise {
            shared: vec![
                "DeploymentManifest".to_string(),
                "ResourceEnvelope".to_string()
            ]
        }
    );
}

#[test]
fn a_contract_in_both_specs_permits_concurrency() {
    let issue_23 = TypeSurface::new(23, ["DeploymentManifest"], ["DeploymentManifest"]);
    let issue_28 = TypeSurface::new(28, ["DeploymentManifest"], ["DeploymentManifest"]);
    assert_eq!(
        type_coexistence(&issue_23, &issue_28),
        TypeCoexistence::Contracted {
            shared: vec!["DeploymentManifest".to_string()]
        }
    );
}

#[test]
fn a_contract_in_only_one_spec_does_not() {
    // One side declaring the shape it expects is an assumption, not an
    // interface: the issues are still serialised.
    let issue_23 = TypeSurface::new(23, ["DeploymentManifest"], ["DeploymentManifest"]);
    let issue_28 = TypeSurface::new(28, ["DeploymentManifest"], [] as [&str; 0]);
    assert_eq!(
        type_coexistence(&issue_23, &issue_28),
        TypeCoexistence::Serialise {
            shared: vec!["DeploymentManifest".to_string()]
        }
    );
}

#[test]
fn disjoint_type_surfaces_run_concurrently() {
    let a = TypeSurface::new(1, ["DeploymentManifest"], [] as [&str; 0]);
    let b = TypeSurface::new(2, ["ResourceEnvelope"], [] as [&str; 0]);
    assert_eq!(type_coexistence(&a, &b), TypeCoexistence::Concurrent);
}

#[test]
fn unaddressed_constructors_are_sorted_by_key() {
    let change = issue_23_change();
    let unsorted = vec![
        ConstructorSite {
            file: "crates/deploy/src/gateway_catalog.rs".to_string(),
            site: "DeploymentManifest::from_registry".to_string(),
        },
        ConstructorSite {
            file: "crates/deploy/src/manifest.rs".to_string(),
            site: "DeploymentManifest::new".to_string(),
        },
        ConstructorSite {
            file: "crates/deploy/src/gateway_catalog.rs".to_string(),
            site: "DeploymentManifest::from_catalog_entry".to_string(),
        },
    ];
    let missing = unaddressed_constructors(&change, &unsorted);
    assert_eq!(
        missing.iter().map(|s| s.key()).collect::<Vec<_>>(),
        vec![
            "crates/deploy/src/gateway_catalog.rs::DeploymentManifest::from_catalog_entry",
            "crates/deploy/src/gateway_catalog.rs::DeploymentManifest::from_registry",
        ]
    );
}

#[test]
fn a_spec_that_names_no_call_sites_fails_the_gate() {
    assert!(!spec_names_call_sites(&ShapeChange::new(
        "DeploymentManifest",
        ["f"],
        [] as [&str; 0]
    )));
    assert!(spec_names_call_sites(&issue_23_change()));
}

#[test]
fn hand_repair_is_only_the_last_resort() {
    let conflict = IntegrationVerdict::SemanticConflict {
        missing: vec![ConstructorSite {
            file: "crates/deploy/src/gateway_catalog.rs".to_string(),
            site: "DeploymentManifest::from_catalog_entry".to_string(),
        }],
    };
    // The authoring context is available: re-dispatch, never hand-repair.
    assert_eq!(
        repair_strategy(&conflict, true, LANDING),
        RepairStrategy::ReDispatch {
            against: LANDING.to_string()
        }
    );
    // It is not: hand-repair is the only path left, and it is the risky one.
    assert_eq!(
        repair_strategy(&conflict, false, LANDING),
        RepairStrategy::ManualRepair
    );
    assert_eq!(
        repair_strategy(&IntegrationVerdict::Clean, true, LANDING),
        RepairStrategy::NoAction
    );
}
