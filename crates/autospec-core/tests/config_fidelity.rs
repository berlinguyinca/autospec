//! Config fidelity (#3692): the config file is not the whole configuration —
//! a stale value survives in database records and per-entity rows.
//!
//! The four invariants, one test group each:
//! 1. a migration plan must name every store the value can live in, not just
//!    the file (`MigrationPlan::uncovered`);
//! 2. the migration is complete only when every store in the footprint was
//!    searched and every searched store reports zero hits
//!    (`verify_migration` / `MigrationAudit`);
//! 3. rewriting a record preserves the prior value and a dated marker
//!    (`record_rewrite` / `RewrittenRecord`);
//! 4. a store that was never searched is unverified, never clean — a clean
//!    grep of the file is `UNVERIFIED`, not `COMPLETE` (`StoreScan`).

use std::collections::BTreeSet;

use autospec_core::config_fidelity::{
    record_rewrite, verify_migration, MigrationAudit, MigrationPlan, MigrationVerdict, StoreKind,
    StoreScan,
};

/// The value's full footprint in the #3692 shape: it lives in the file *and*
/// in per-entity records (the agent and conversation rows). The file is the
/// obvious half; the records are the half a grep of the config directory
/// cannot see.
fn footprint() -> BTreeSet<StoreKind> {
    [StoreKind::ConfigFile, StoreKind::PerEntityRecord]
        .into_iter()
        .collect()
}

// ── 1. enumerate the stores before editing ─────────────────────────────

#[test]
fn a_plan_naming_only_the_file_is_incomplete_by_construction() {
    let plan = MigrationPlan::new(
        "model",
        "qwen3.5:9b",
        "fry-qwen-27b",
        [StoreKind::ConfigFile],
    );

    assert!(!plan.is_complete(&footprint()));
    assert_eq!(
        plan.uncovered(&footprint()),
        vec![StoreKind::PerEntityRecord]
    );
}

#[test]
fn a_plan_that_names_every_store_in_the_footprint_is_complete() {
    let plan = MigrationPlan::new(
        "model",
        "qwen3.5:9b",
        "fry-qwen-27b",
        [StoreKind::ConfigFile, StoreKind::PerEntityRecord],
    );

    assert!(plan.is_complete(&footprint()));
    assert!(plan.uncovered(&footprint()).is_empty());
}

#[test]
fn a_plan_is_checked_only_against_the_footprint_it_is_given() {
    // A footprint with no per-entity records (a stateless service) is fully
    // covered by a file-only plan — completeness is relative to the footprint.
    let file_only = [StoreKind::ConfigFile].into_iter().collect();
    let plan = MigrationPlan::new("model", "a", "b", [StoreKind::ConfigFile]);

    assert!(plan.is_complete(&file_only));
}

// ── 2. require zero hits in every store ────────────────────────────────

#[test]
fn the_3692_incident_cannot_read_as_complete() {
    // The file was edited and greps clean, but the per-entity records were
    // never inspected. The audit must be Unverified, never Complete.
    let scans = [StoreScan::clean(StoreKind::ConfigFile, "config.yaml")];

    let audit = verify_migration(&footprint(), &scans);

    assert!(!audit.is_complete());
    assert_eq!(audit.verdict(), MigrationVerdict::Unverified);
    assert_eq!(audit.code(), "UNVERIFIED");
    // The blind spot is named: the footprint's record store was never scanned.
    assert_eq!(audit.unsearched, vec!["per_entity_record"]);
    assert!(audit.dirty.is_empty());
    assert_eq!(audit.clean, vec!["config.yaml"]);
}

#[test]
fn a_searched_database_still_holding_the_old_value_is_incomplete() {
    // The operator did look at the database (the #3692 "database count per
    // collection") and found agents: 1, conversations: 8.
    let scans = [
        StoreScan::clean(StoreKind::ConfigFile, "config.yaml"),
        StoreScan::dirty(StoreKind::PerEntityRecord, "agents", 1),
        StoreScan::dirty(StoreKind::PerEntityRecord, "conversations", 8),
    ];

    let audit = verify_migration(&footprint(), &scans);

    assert!(!audit.is_complete());
    assert_eq!(audit.verdict(), MigrationVerdict::Incomplete);
    assert_eq!(audit.code(), "INCOMPLETE");
    assert_eq!(audit.dirty, vec!["agents", "conversations"]);
    assert!(audit.unsearched.is_empty());
}

#[test]
fn a_migration_is_complete_only_when_every_store_is_searched_and_clean() {
    let scans = [
        StoreScan::clean(StoreKind::ConfigFile, "config.yaml"),
        StoreScan::clean(StoreKind::PerEntityRecord, "agents"),
        StoreScan::clean(StoreKind::PerEntityRecord, "conversations"),
    ];

    let audit = verify_migration(&footprint(), &scans);

    assert!(audit.is_complete());
    assert_eq!(audit.verdict(), MigrationVerdict::Complete);
    assert_eq!(audit.code(), "COMPLETE");
    assert!(audit.dirty.is_empty());
    assert!(audit.unsearched.is_empty());
}

#[test]
fn a_never_searched_store_outranks_a_dirty_one_in_the_summary_code() {
    // Both a known-dirty store and a blind spot are present. The summary is
    // Unverified (the hole means nothing can be claimed), but the dirty store
    // is still named — no finding is hidden behind the summary code.
    let scans = [
        StoreScan::dirty(StoreKind::PerEntityRecord, "agents", 1),
        StoreScan::unverified(StoreKind::Database, "presets"),
    ];
    let footprint = [
        StoreKind::ConfigFile,
        StoreKind::Database,
        StoreKind::PerEntityRecord,
    ]
    .into_iter()
    .collect();

    let audit = verify_migration(&footprint, &scans);

    assert!(!audit.is_complete());
    assert_eq!(audit.verdict(), MigrationVerdict::Unverified);
    assert_eq!(audit.code(), "UNVERIFIED");
    assert_eq!(audit.dirty, vec!["agents"]);
    // The explicit unsearched scan plus the file kind, which was never scanned.
    assert_eq!(audit.unsearched, vec!["config_file", "presets"]);
}

#[test]
fn an_audit_reports_the_stores_it_found_clean() {
    let audit = MigrationAudit {
        clean: vec!["config.yaml".into(), "agents".into()],
        dirty: vec![],
        unsearched: vec![],
    };

    assert!(audit.is_complete());
    assert_eq!(audit.verdict(), MigrationVerdict::Complete);
}

// ── 3. preserve the prior value when rewriting ─────────────────────────

#[test]
fn a_record_rewrite_keeps_the_prior_value_and_a_dated_marker() {
    let record = record_rewrite(
        StoreKind::PerEntityRecord,
        "agent-42",
        "model",
        "qwen3.5:9b",
        "fry-qwen-27b",
        "2026-08-30",
    );

    assert_eq!(record.previous_value, "qwen3.5:9b");
    assert_eq!(record.new_value, "fry-qwen-27b");
    assert_eq!(record.rewritten_at, "2026-08-30");
    // Reversible: the old value is retained and a date exists to audit it.
    assert!(record.is_reversible());
}

#[test]
fn a_rewrite_that_drops_the_prior_value_is_not_reversible() {
    let mut record = record_rewrite(
        StoreKind::PerEntityRecord,
        "agent-42",
        "model",
        "qwen3.5:9b",
        "fry-qwen-27b",
        "2026-08-30",
    );
    record.previous_value.clear();

    assert!(!record.is_reversible());
}

#[test]
fn a_rewrite_without_a_date_is_not_auditable() {
    let mut record = record_rewrite(
        StoreKind::PerEntityRecord,
        "agent-42",
        "model",
        "qwen3.5:9b",
        "fry-qwen-27b",
        "2026-08-30",
    );
    record.rewritten_at.clear();

    assert!(!record.is_reversible());
}

// ── 4. a clean grep is unverified, not done ────────────────────────────

#[test]
fn a_never_searched_store_is_unverified_and_never_clean() {
    let scan = StoreScan::unverified(StoreKind::Database, "conversations");

    assert!(scan.is_unverified());
    assert!(!scan.is_clean());
    assert!(!scan.is_dirty());
}

#[test]
fn a_searched_clean_store_is_clean_and_verified() {
    let scan = StoreScan::clean(StoreKind::ConfigFile, "config.yaml");

    assert!(scan.is_clean());
    assert!(!scan.is_unverified());
    assert!(!scan.is_dirty());
}

#[test]
fn a_searched_dirty_store_is_dirty_not_clean_or_unverified() {
    let scan = StoreScan::dirty(StoreKind::PerEntityRecord, "agents", 3);

    assert!(scan.is_dirty());
    assert!(!scan.is_clean());
    assert!(!scan.is_unverified());
}

#[test]
fn an_unverified_scan_cannot_masquerade_as_a_clean_one() {
    // Even if a caller forgets to set `searched`, the constructor forces the
    // hit count to zero *and* marks it unsearched — `is_clean` still returns
    // false because `searched` is false.
    let scan = StoreScan {
        kind: StoreKind::Database,
        name: "conversations".into(),
        searched: false,
        hits: 0,
    };

    assert!(!scan.is_clean());
    assert!(scan.is_unverified());
}

// ── store-kind vocabulary ──────────────────────────────────────────────

#[test]
fn store_kind_round_trips_through_its_machine_name() {
    for kind in StoreKind::ALL {
        assert_eq!(StoreKind::parse(kind.as_str()), Some(kind));
    }
}

#[test]
fn store_kind_parse_rejects_unknown_names() {
    assert_eq!(StoreKind::parse("redis"), None);
    assert_eq!(StoreKind::parse(""), None);
}
