//! Issue #3816 — ownership, concurrency, and dependency-reason metadata.
//!
//! Spec: `docs/specs/2026-09-08-parallel-decomposition-fleet-saturation.md`
//! (§11 ownership partitioning, §12 candidate dependency graph reason codes,
//! §14 concurrency metadata contract, §15 machine-readable issue metadata,
//! §28 proposed Rust data structures, §29.5 ownership overlap).

use autospec_core::graph::metadata::{
    normalize_path, overlaps, ConcurrencyMetadata, DependencyReason, OwnedSurface, Ownership,
};
use std::str::FromStr;

fn surface(path: &str, symbols: &[&str]) -> OwnedSurface {
    OwnedSurface {
        path: path.to_string(),
        symbols: symbols.iter().map(|symbol| (*symbol).to_string()).collect(),
    }
}

fn ownership(
    exclusive: &[OwnedSurface],
    shared_read: &[OwnedSurface],
    shared_write: &[OwnedSurface],
) -> Ownership {
    Ownership {
        exclusive: exclusive.to_vec(),
        shared_read: shared_read.to_vec(),
        shared_write: shared_write.to_vec(),
    }
}

#[test]
fn from_str_accepts_all_nine_reason_codes() {
    let reasons = DependencyReason::ALL;
    assert_eq!(reasons.len(), 9);
    for reason in reasons {
        let parsed = DependencyReason::from_str(reason.as_str())
            .unwrap_or_else(|err| panic!("{}: {err}", reason.as_str()));
        assert_eq!(parsed, *reason);
    }
}

#[test]
fn from_str_rejects_unknown_reason_codes() {
    assert!(DependencyReason::from_str("consumes-nothing").is_err());
    assert!(DependencyReason::from_str("").is_err());
    assert!(DependencyReason::from_str("consumes_new_interface").is_err());
    assert!(DependencyReason::from_str("CONSUMES-NEW-INTERFACE").is_err());
}

#[test]
fn dependency_reason_serde_round_trips_every_code() {
    for reason in DependencyReason::ALL {
        let json = serde_json::to_string(reason).expect("serialize reason");
        assert_eq!(json, format!("\"{}\"", reason.as_str()));
        let back: DependencyReason = serde_json::from_str(&json).expect("deserialize reason");
        assert_eq!(back, *reason);
    }
}

#[test]
fn ownership_serde_round_trip_preserves_all_lists() {
    let original = ownership(
        &[surface(
            "crates/router/src/http.rs",
            &["HttpRouter", "route_request"],
        )],
        &[surface(
            "crates/router/src/contracts.rs",
            &["RouterBackend"],
        )],
        &[surface("crates/core/src/lib.rs", &[])],
    );
    let json = serde_json::to_string(&original).expect("serialize ownership");
    let back: Ownership = serde_json::from_str(&json).expect("deserialize ownership");
    assert_eq!(back, original);
    assert_eq!(back.exclusive.len(), 1);
    assert_eq!(back.shared_read.len(), 1);
    assert_eq!(back.shared_write.len(), 1);
    assert_eq!(
        back.exclusive[0].symbols,
        vec!["HttpRouter", "route_request"]
    );
}

#[test]
fn concurrency_metadata_serde_round_trip_preserves_fields() {
    let populated = ConcurrencyMetadata {
        parallel_safe: true,
        conflict_domains: vec!["router-http".to_string()],
    };
    let json = serde_json::to_string(&populated).expect("serialize metadata");
    let back: ConcurrencyMetadata = serde_json::from_str(&json).expect("deserialize metadata");
    assert!(back.parallel_safe);
    assert_eq!(back.conflict_domains, vec!["router-http".to_string()]);

    let empty = ConcurrencyMetadata {
        parallel_safe: false,
        conflict_domains: Vec::new(),
    };
    let json = serde_json::to_string(&empty).expect("serialize metadata");
    let back: ConcurrencyMetadata = serde_json::from_str(&json).expect("deserialize metadata");
    assert_eq!(back, empty);
}

#[test]
fn normalize_path_collapses_dots_and_duplicate_slashes() {
    assert_eq!(normalize_path("./a/b/c.rs"), "a/b/c.rs");
    assert_eq!(normalize_path("a//b/c.rs"), "a/b/c.rs");
    assert_eq!(normalize_path("a/./b"), "a/b");
    assert_eq!(normalize_path("/a/b.rs"), "a/b.rs");
}

#[test]
fn normalize_path_neutralizes_parent_traversal() {
    assert_eq!(normalize_path("../a"), "a");
    assert_eq!(normalize_path("a/../b"), "b");
    assert_eq!(normalize_path("a/b/../../.."), "");
    assert_eq!(normalize_path(".."), "");
}

#[test]
fn normalize_path_preserves_directory_glob() {
    assert_eq!(
        normalize_path("./web/src/features/session/**"),
        "web/src/features/session/**"
    );
    assert_eq!(
        normalize_path("web/src/features/session//**"),
        "web/src/features/session/**"
    );
}

#[test]
fn overlaps_true_for_same_file() {
    let a = ownership(&[surface("crates/router/src/http.rs", &[])], &[], &[]);
    let b = ownership(&[surface("./crates/router/src/http.rs", &[])], &[], &[]);
    assert!(overlaps(&a, &b));
}

#[test]
fn overlaps_true_for_glob_against_child_file() {
    let a = ownership(&[surface("a/b/**", &[])], &[], &[]);
    let b = ownership(&[surface("a/b/c.rs", &[])], &[], &[]);
    assert!(overlaps(&a, &b));
    // overlap is symmetric
    assert!(overlaps(&b, &a));
}

#[test]
fn overlaps_true_for_same_declared_symbol() {
    let a = ownership(
        &[surface("crates/router/src/http.rs", &["HttpRouter"])],
        &[],
        &[],
    );
    let b = ownership(
        &[surface("crates/router/src/contracts.rs", &["HttpRouter"])],
        &[],
        &[],
    );
    assert!(overlaps(&a, &b));
}

#[test]
fn overlaps_considers_shared_read_and_shared_write_lists() {
    let a = ownership(&[], &[surface("crates/router/src/contracts.rs", &[])], &[]);
    let b = ownership(&[], &[], &[surface("crates/router/src/contracts.rs", &[])]);
    assert!(overlaps(&a, &b));
}

#[test]
fn overlaps_false_for_disjoint_surfaces() {
    let a = ownership(
        &[surface("crates/router/src/http.rs", &["HttpRouter"])],
        &[],
        &[],
    );
    let b = ownership(&[surface("crates/cli/src/main.rs", &["CliMain"])], &[], &[]);
    assert!(!overlaps(&a, &b));
}

#[test]
fn overlaps_false_for_empty_ownership() {
    let a = ownership(&[], &[], &[]);
    let b = ownership(&[surface("crates/router/src/http.rs", &[])], &[], &[]);
    assert!(!overlaps(&a, &b));
    assert!(!overlaps(&b, &a));
}

#[test]
fn overlaps_glob_does_not_cover_base_or_siblings() {
    let glob = ownership(&[surface("a/b/**", &[])], &[], &[]);
    let base = ownership(&[surface("a/b", &[])], &[], &[]);
    assert!(!overlaps(&glob, &base));
    let sibling = ownership(&[surface("a/bc.rs", &[])], &[], &[]);
    assert!(!overlaps(&glob, &sibling));
}

#[test]
fn schema_file_exists_and_parses_as_json() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../schemas/autospec-issue-concurrency.schema.json"
    );
    let raw = std::fs::read_to_string(path).expect("schema file must exist");
    let value: serde_json::Value =
        serde_json::from_str(&raw).expect("schema file must parse as JSON");
    assert_eq!(value["title"], "AutoSpec Issue Concurrency Metadata");
    assert_eq!(
        value["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
}
